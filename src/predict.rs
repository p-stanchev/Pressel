use anyhow::{Result, bail};

pub const PREDICTOR_COUNT: u8 = 7;
pub const ADAPTIVE_PREDICTOR_ID: u8 = 6;
pub const ADAPTIVE_BLOCK_SIZE: usize = 8;

pub fn encode_residuals(
    transformed: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    if transformed.len() != width as usize * height as usize * 4 {
        bail!("transformed tile length mismatch");
    }
    if predictor_id == ADAPTIVE_PREDICTOR_ID {
        return encode_adaptive_residuals(transformed, width, height);
    }
    let mut out = Vec::with_capacity(transformed.len());
    for y in 0..height as usize {
        for x in 0..width as usize {
            for c in 0..4 {
                let idx = ((y * width as usize + x) * 4) + c;
                let actual = transformed[idx];
                let pred = predict_value(transformed, width as usize, x, y, c, predictor_id)?;
                out.push(actual.wrapping_sub(pred));
            }
        }
    }
    Ok(out)
}

pub fn decode_residuals(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    if predictor_id == ADAPTIVE_PREDICTOR_ID {
        return decode_adaptive_residuals(residuals, width, height);
    }
    if residuals.len() != width as usize * height as usize * 4 {
        bail!("residual tile length mismatch");
    }
    let mut out = vec![0_u8; residuals.len()];
    for y in 0..height as usize {
        for x in 0..width as usize {
            for c in 0..4 {
                let idx = ((y * width as usize + x) * 4) + c;
                let pred = predict_value(&out, width as usize, x, y, c, predictor_id)?;
                out[idx] = pred.wrapping_add(residuals[idx]);
            }
        }
    }
    Ok(out)
}

pub fn predict_value(
    bytes: &[u8],
    width: usize,
    x: usize,
    y: usize,
    channel: usize,
    predictor_id: u8,
) -> Result<u8> {
    let left = if x > 0 {
        bytes[((y * width + (x - 1)) * 4) + channel]
    } else {
        0
    };
    let top = if y > 0 {
        bytes[(((y - 1) * width + x) * 4) + channel]
    } else {
        0
    };
    let top_left = if x > 0 && y > 0 {
        bytes[(((y - 1) * width + (x - 1)) * 4) + channel]
    } else {
        0
    };

    let pred = match predictor_id {
        0 => 0,
        1 => left,
        2 => top,
        3 => ((left as u16 + top as u16) / 2) as u8,
        4 => paeth(left, top, top_left),
        5 => med(left, top, top_left),
        6 => bail!("adaptive predictor id must be handled through block map"),
        _ => bail!("unsupported predictor id: {predictor_id}"),
    };
    Ok(pred)
}

pub fn expected_residual_len(width: u16, height: u16, predictor_id: u8) -> Result<usize> {
    let pixel_bytes = width as usize * height as usize * 4;
    if predictor_id == ADAPTIVE_PREDICTOR_ID {
        let blocks_x = (width as usize).div_ceil(ADAPTIVE_BLOCK_SIZE);
        let blocks_y = (height as usize).div_ceil(ADAPTIVE_BLOCK_SIZE);
        let map_len = blocks_x
            .checked_mul(blocks_y)
            .ok_or_else(|| anyhow::anyhow!("adaptive predictor map length overflow"))?;
        return map_len
            .checked_add(pixel_bytes)
            .ok_or_else(|| anyhow::anyhow!("adaptive residual stream length overflow"));
    }
    Ok(pixel_bytes)
}

fn encode_adaptive_residuals(transformed: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let blocks_x = width.div_ceil(ADAPTIVE_BLOCK_SIZE);
    let blocks_y = height.div_ceil(ADAPTIVE_BLOCK_SIZE);
    let map_len = blocks_x
        .checked_mul(blocks_y)
        .ok_or_else(|| anyhow::anyhow!("adaptive predictor map length overflow"))?;

    let mut predictor_map = Vec::with_capacity(map_len);
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            predictor_map.push(select_block_predictor(
                transformed,
                width,
                height,
                block_x,
                block_y,
            )?);
        }
    }

    let mut residuals = Vec::with_capacity(transformed.len());
    for y in 0..height {
        for x in 0..width {
            let predictor_id =
                predictor_map[(y / ADAPTIVE_BLOCK_SIZE) * blocks_x + (x / ADAPTIVE_BLOCK_SIZE)];
            for c in 0..4 {
                let idx = ((y * width + x) * 4) + c;
                let actual = transformed[idx];
                let pred = predict_value(transformed, width, x, y, c, predictor_id)?;
                residuals.push(actual.wrapping_sub(pred));
            }
        }
    }

    let mut out = predictor_map;
    out.extend_from_slice(&residuals);
    Ok(out)
}

fn decode_adaptive_residuals(residuals: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let blocks_x = width.div_ceil(ADAPTIVE_BLOCK_SIZE);
    let blocks_y = height.div_ceil(ADAPTIVE_BLOCK_SIZE);
    let map_len = blocks_x
        .checked_mul(blocks_y)
        .ok_or_else(|| anyhow::anyhow!("adaptive predictor map length overflow"))?;
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("adaptive residual byte count overflow"))?;

    if residuals.len() != map_len + pixel_bytes {
        bail!(
            "adaptive residual stream length mismatch: expected {}, got {}",
            map_len + pixel_bytes,
            residuals.len()
        );
    }

    let predictor_map = &residuals[..map_len];
    let residual_bytes = &residuals[map_len..];
    let mut out = vec![0_u8; pixel_bytes];
    for y in 0..height {
        for x in 0..width {
            let predictor_id =
                predictor_map[(y / ADAPTIVE_BLOCK_SIZE) * blocks_x + (x / ADAPTIVE_BLOCK_SIZE)];
            for c in 0..4 {
                let idx = ((y * width + x) * 4) + c;
                let pred = predict_value(&out, width, x, y, c, predictor_id)?;
                out[idx] = pred.wrapping_add(residual_bytes[idx]);
            }
        }
    }
    Ok(out)
}

fn select_block_predictor(
    transformed: &[u8],
    width: usize,
    height: usize,
    block_x: usize,
    block_y: usize,
) -> Result<u8> {
    let start_x = block_x * ADAPTIVE_BLOCK_SIZE;
    let start_y = block_y * ADAPTIVE_BLOCK_SIZE;
    let end_x = (start_x + ADAPTIVE_BLOCK_SIZE).min(width);
    let end_y = (start_y + ADAPTIVE_BLOCK_SIZE).min(height);

    let mut best_predictor = 0_u8;
    let mut best_score = u64::MAX;
    for predictor_id in 0..ADAPTIVE_PREDICTOR_ID {
        let mut score = 0_u64;
        for y in start_y..end_y {
            for x in start_x..end_x {
                for c in 0..4 {
                    let idx = ((y * width + x) * 4) + c;
                    let actual = transformed[idx];
                    let pred = predict_value(transformed, width, x, y, c, predictor_id)?;
                    score += residual_cost(actual.wrapping_sub(pred)) as u64;
                }
            }
        }
        if score < best_score {
            best_score = score;
            best_predictor = predictor_id;
        }
    }

    Ok(best_predictor)
}

fn residual_cost(residual: u8) -> u16 {
    let signed = if residual < 128 {
        residual as i16
    } else {
        residual as i16 - 256
    };
    signed.unsigned_abs()
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn med(a: u8, b: u8, c: u8) -> u8 {
    if c >= a.max(b) {
        a.min(b)
    } else if c <= a.min(b) {
        a.max(b)
    } else {
        a.wrapping_add(b).wrapping_sub(c)
    }
}
