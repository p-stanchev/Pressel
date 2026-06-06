use anyhow::{Result, bail};

pub const TRANSFORM_COUNT: u8 = 2;

pub fn apply_transform(transform_id: u8, rgba: &[u8]) -> Result<Vec<u8>> {
    match transform_id {
        0 => Ok(rgba.to_vec()),
        1 => Ok(subtract_green(rgba)),
        _ => bail!("unsupported transform id: {transform_id}"),
    }
}

pub fn reverse_transform(transform_id: u8, transformed: &[u8]) -> Result<Vec<u8>> {
    match transform_id {
        0 => Ok(transformed.to_vec()),
        1 => Ok(add_green_back(transformed)),
        _ => bail!("unsupported transform id: {transform_id}"),
    }
}

fn subtract_green(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let r = px[0];
        let g = px[1];
        let b = px[2];
        let a = px[3];
        out.push(r.wrapping_sub(g));
        out.push(g);
        out.push(b.wrapping_sub(g));
        out.push(a);
    }
    out
}

fn add_green_back(transformed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(transformed.len());
    for px in transformed.chunks_exact(4) {
        let r_prime = px[0];
        let g = px[1];
        let b_prime = px[2];
        let a = px[3];
        out.push(r_prime.wrapping_add(g));
        out.push(g);
        out.push(b_prime.wrapping_add(g));
        out.push(a);
    }
    out
}
