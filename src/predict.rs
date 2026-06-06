use anyhow::{Result, bail};

pub const PREDICTOR_COUNT: u8 = 6;

pub fn encode_residuals(
    transformed: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    if transformed.len() != width as usize * height as usize * 4 {
        bail!("transformed tile length mismatch");
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
        _ => bail!("unsupported predictor id: {predictor_id}"),
    };
    Ok(pred)
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
