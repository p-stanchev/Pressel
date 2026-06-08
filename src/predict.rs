use anyhow::{Result, bail};

pub const PREDICTOR_COUNT: u8 = 10;
pub const ADAPTIVE_PREDICTOR_ID: u8 = 6;
pub const EDGE_GUIDED_PREDICTOR_ID: u8 = 7;
pub const PHOTO_GUIDED_PREDICTOR_ID: u8 = 8;
pub const WEIGHTED_GRADIENT_PREDICTOR_ID: u8 = 9;
pub const ADAPTIVE_BLOCK_SIZE: usize = 8;
const PHOTO_GUIDED_PREFIX_LEN: usize = 4;
const PHOTO_GUIDED_GREEN_PREDICTORS: [u8; 4] = [
    4,
    5,
    EDGE_GUIDED_PREDICTOR_ID,
    WEIGHTED_GRADIENT_PREDICTOR_ID,
];
const PHOTO_GUIDED_CHROMA_PREDICTORS: [u8; 2] = [1, 5];
const PHOTO_GUIDED_K_NUMS: [i16; 7] = [0, 1, 2, 3, 4, 5, 6];
const PHOTO_GUIDED_K_DEN_SHIFT: u8 = 2;
const PHOTO_GUIDED_K_INDICES: [u8; 4] = [2, 3, 4, 5];

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
    if predictor_id == PHOTO_GUIDED_PREDICTOR_ID {
        return encode_photo_guided_residuals(transformed, width, height);
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
    if predictor_id == PHOTO_GUIDED_PREDICTOR_ID {
        return decode_photo_guided_residuals(residuals, width, height);
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
    let top_right = if y > 0 && x + 1 < width {
        bytes[(((y - 1) * width + (x + 1)) * 4) + channel]
    } else {
        top
    };

    let pred = match predictor_id {
        0 => 0,
        1 => left,
        2 => top,
        3 => ((left as u16 + top as u16) / 2) as u8,
        4 => paeth(left, top, top_left),
        5 => med(left, top, top_left),
        6 => bail!("adaptive predictor id must be handled through block map"),
        7 => edge_guided(left, top, top_left),
        8 => bail!("photo-guided predictor id must be handled through tile prefix"),
        9 => weighted_gradient(left, top, top_left, top_right),
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
    if predictor_id == PHOTO_GUIDED_PREDICTOR_ID {
        return PHOTO_GUIDED_PREFIX_LEN
            .checked_add(pixel_bytes)
            .ok_or_else(|| anyhow::anyhow!("photo-guided residual stream length overflow"));
    }
    Ok(pixel_bytes)
}

pub fn residual_prefix_len(width: u16, height: u16, predictor_id: u8) -> Result<usize> {
    let pixel_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("residual pixel byte count overflow"))?;
    let total_len = expected_residual_len(width, height, predictor_id)?;
    total_len
        .checked_sub(pixel_bytes)
        .ok_or_else(|| anyhow::anyhow!("residual prefix length underflow"))
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
    for predictor_id in base_predictor_ids() {
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

fn base_predictor_ids() -> impl Iterator<Item = u8> {
    (0..PREDICTOR_COUNT).filter(|&predictor_id| {
        predictor_id != ADAPTIVE_PREDICTOR_ID && predictor_id != PHOTO_GUIDED_PREDICTOR_ID
    })
}

fn encode_photo_guided_residuals(transformed: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let params = select_photo_guided_params(transformed, width, height)?;
    let mut out = Vec::with_capacity(PHOTO_GUIDED_PREFIX_LEN + transformed.len());
    out.push(params.green_predictor_id);
    out.push(params.chroma_predictor_id);
    out.push(params.k_r_index);
    out.push(params.k_b_index);

    for y in 0..height {
        for x in 0..width {
            let base = PixelBases::from_source(transformed, width, x, y, &params)?;

            let g_idx = ((y * width + x) * 4) + 1;
            let g_actual = transformed[g_idx];
            let g_residual = g_actual.wrapping_sub(base.g_pred);
            let g_recon = base.g_pred.wrapping_add(g_residual);

            let r_idx = (y * width + x) * 4;
            let r_actual = transformed[r_idx];
            let r_pred =
                photo_guided_chroma_pred(base.r_base, base.g_base, g_recon, params.k_r_index);
            let r_residual = r_actual.wrapping_sub(r_pred);

            let b_idx = ((y * width + x) * 4) + 2;
            let b_actual = transformed[b_idx];
            let b_pred =
                photo_guided_chroma_pred(base.b_base, base.g_base, g_recon, params.k_b_index);
            let b_residual = b_actual.wrapping_sub(b_pred);

            let a_idx = ((y * width + x) * 4) + 3;
            let a_actual = transformed[a_idx];
            let a_residual = a_actual.wrapping_sub(base.a_pred);

            out.push(r_residual);
            out.push(g_residual);
            out.push(b_residual);
            out.push(a_residual);
        }
    }
    Ok(out)
}

fn decode_photo_guided_residuals(residuals: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("photo-guided residual byte count overflow"))?;
    if residuals.len() != PHOTO_GUIDED_PREFIX_LEN + pixel_bytes {
        bail!(
            "photo-guided residual stream length mismatch: expected {}, got {}",
            PHOTO_GUIDED_PREFIX_LEN + pixel_bytes,
            residuals.len()
        );
    }
    let params = PhotoGuidedParams::from_prefix(&residuals[..PHOTO_GUIDED_PREFIX_LEN])?;
    let residual_bytes = &residuals[PHOTO_GUIDED_PREFIX_LEN..];
    let mut out = vec![0_u8; pixel_bytes];

    for y in 0..height {
        for x in 0..width {
            let base = PixelBases::from_source(&out, width, x, y, &params)?;
            let idx = (y * width + x) * 4;
            let r_residual = residual_bytes[idx];
            let g_residual = residual_bytes[idx + 1];
            let b_residual = residual_bytes[idx + 2];
            let a_residual = residual_bytes[idx + 3];

            let g = base.g_pred.wrapping_add(g_residual);
            let r_pred = photo_guided_chroma_pred(base.r_base, base.g_base, g, params.k_r_index);
            let b_pred = photo_guided_chroma_pred(base.b_base, base.g_base, g, params.k_b_index);

            out[idx] = r_pred.wrapping_add(r_residual);
            out[idx + 1] = g;
            out[idx + 2] = b_pred.wrapping_add(b_residual);
            out[idx + 3] = base.a_pred.wrapping_add(a_residual);
        }
    }

    Ok(out)
}

#[derive(Clone, Copy)]
struct PhotoGuidedParams {
    green_predictor_id: u8,
    chroma_predictor_id: u8,
    k_r_index: u8,
    k_b_index: u8,
}

impl PhotoGuidedParams {
    fn from_prefix(prefix: &[u8]) -> Result<Self> {
        if prefix.len() != PHOTO_GUIDED_PREFIX_LEN {
            bail!("invalid photo-guided prefix length");
        }
        let params = Self {
            green_predictor_id: prefix[0],
            chroma_predictor_id: prefix[1],
            k_r_index: prefix[2],
            k_b_index: prefix[3],
        };
        params.validate()?;
        Ok(params)
    }

    fn validate(self) -> Result<Self> {
        if !PHOTO_GUIDED_GREEN_PREDICTORS.contains(&self.green_predictor_id) {
            bail!(
                "unsupported photo-guided green predictor id: {}",
                self.green_predictor_id
            );
        }
        if !PHOTO_GUIDED_CHROMA_PREDICTORS.contains(&self.chroma_predictor_id) {
            bail!(
                "unsupported photo-guided chroma predictor id: {}",
                self.chroma_predictor_id
            );
        }
        if !PHOTO_GUIDED_K_INDICES.contains(&self.k_r_index)
            || !PHOTO_GUIDED_K_INDICES.contains(&self.k_b_index)
        {
            bail!("photo-guided coefficient index out of range");
        }
        Ok(self)
    }
}

#[derive(Clone, Copy)]
struct PixelBases {
    g_pred: u8,
    g_base: u8,
    r_base: u8,
    b_base: u8,
    a_pred: u8,
}

impl PixelBases {
    fn from_source(
        bytes: &[u8],
        width: usize,
        x: usize,
        y: usize,
        params: &PhotoGuidedParams,
    ) -> Result<Self> {
        let g_pred = predict_value(bytes, width, x, y, 1, params.green_predictor_id)?;
        let g_base = predict_value(bytes, width, x, y, 1, params.chroma_predictor_id)?;
        let r_base = predict_value(bytes, width, x, y, 0, params.chroma_predictor_id)?;
        let b_base = predict_value(bytes, width, x, y, 2, params.chroma_predictor_id)?;
        let a_pred = predict_value(bytes, width, x, y, 3, params.green_predictor_id)?;
        Ok(Self {
            g_pred,
            g_base,
            r_base,
            b_base,
            a_pred,
        })
    }
}

fn select_photo_guided_params(
    transformed: &[u8],
    width: usize,
    height: usize,
) -> Result<PhotoGuidedParams> {
    let mut best = PhotoGuidedParams {
        green_predictor_id: 5,
        chroma_predictor_id: 1,
        k_r_index: 4,
        k_b_index: 4,
    };
    let mut best_score = u64::MAX;

    for &green_predictor_id in &PHOTO_GUIDED_GREEN_PREDICTORS {
        for &chroma_predictor_id in &PHOTO_GUIDED_CHROMA_PREDICTORS {
            for &k_r_index in &PHOTO_GUIDED_K_INDICES {
                for &k_b_index in &PHOTO_GUIDED_K_INDICES {
                    let params = PhotoGuidedParams {
                        green_predictor_id,
                        chroma_predictor_id,
                        k_r_index,
                        k_b_index,
                    };
                    let mut score = PHOTO_GUIDED_PREFIX_LEN as u64 * 8;
                    for y in 0..height {
                        for x in 0..width {
                            let base = PixelBases::from_source(transformed, width, x, y, &params)?;
                            let idx = (y * width + x) * 4;
                            let g_actual = transformed[idx + 1];
                            let g_residual = g_actual.wrapping_sub(base.g_pred);
                            let g_recon = base.g_pred.wrapping_add(g_residual);
                            let r_pred = photo_guided_chroma_pred(
                                base.r_base,
                                base.g_base,
                                g_recon,
                                params.k_r_index,
                            );
                            let b_pred = photo_guided_chroma_pred(
                                base.b_base,
                                base.g_base,
                                g_recon,
                                params.k_b_index,
                            );
                            score += residual_cost(transformed[idx].wrapping_sub(r_pred)) as u64;
                            score += residual_cost(g_residual) as u64;
                            score +=
                                residual_cost(transformed[idx + 2].wrapping_sub(b_pred)) as u64;
                            score += residual_cost(transformed[idx + 3].wrapping_sub(base.a_pred))
                                as u64;
                        }
                    }
                    if score < best_score {
                        best_score = score;
                        best = params;
                    }
                }
            }
        }
    }

    Ok(best)
}

fn photo_guided_chroma_pred(base_c: u8, base_g: u8, current_g: u8, k_index: u8) -> u8 {
    let delta_g = signed_residual(current_g.wrapping_sub(base_g));
    let scaled = scale_signed(delta_g, k_index);
    base_c.wrapping_add(scaled as u8)
}

fn signed_residual(residual: u8) -> i16 {
    if residual < 128 {
        residual as i16
    } else {
        residual as i16 - 256
    }
}

fn scale_signed(value: i16, k_index: u8) -> i16 {
    let numerator = PHOTO_GUIDED_K_NUMS[usize::from(k_index)];
    let scaled = i32::from(value) * i32::from(numerator);
    let adjust = if scaled >= 0 { 2 } else { -2 };
    ((scaled + adjust) >> PHOTO_GUIDED_K_DEN_SHIFT) as i16
}

fn residual_cost(residual: u8) -> u16 {
    let signed = signed_residual(residual);
    signed.unsigned_abs()
}

fn edge_guided(left: u8, top: u8, top_left: u8) -> u8 {
    let horizontal = (i16::from(left) - i16::from(top_left)).unsigned_abs();
    let vertical = (i16::from(top) - i16::from(top_left)).unsigned_abs();
    if horizontal + 4 < vertical {
        left
    } else if vertical + 4 < horizontal {
        top
    } else {
        let gradient = i16::from(left) + i16::from(top) - i16::from(top_left);
        gradient.clamp(0, 255) as u8
    }
}

fn weighted_gradient(left: u8, top: u8, top_left: u8, top_right: u8) -> u8 {
    let gradient = (i16::from(left) + i16::from(top) - i16::from(top_left)).clamp(0, 255) as u8;
    let left_support = 1
        + (i16::from(top) - i16::from(top_left)).unsigned_abs()
        + (i16::from(top_right) - i16::from(top)).unsigned_abs();
    let top_support = 1
        + (i16::from(left) - i16::from(top_left)).unsigned_abs()
        + (i16::from(left) - i16::from(top)).unsigned_abs();

    let left_weight = u32::from(257_u16.saturating_sub(left_support.min(256)));
    let top_weight = u32::from(257_u16.saturating_sub(top_support.min(256)));
    let denom = left_weight + top_weight;
    let blended = (u32::from(left) * left_weight + u32::from(top) * top_weight + denom / 2)
        .checked_div(denom)
        .map(|value| value as u8)
        .unwrap_or_else(|| ((u16::from(left) + u16::from(top)) / 2) as u8);
    ((u16::from(blended) + u16::from(gradient)) / 2) as u8
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
