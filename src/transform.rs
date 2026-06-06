use anyhow::{Result, bail};

pub const TRANSFORM_COUNT: u8 = 6;

pub fn apply_transform(transform_id: u8, rgba: &[u8]) -> Result<Vec<u8>> {
    match transform_id {
        0 => Ok(rgba.to_vec()),
        1 => Ok(subtract_green(rgba)),
        2 => Ok(ycocg_r(rgba)),
        3 => Ok(alpha_plane_separation(rgba)),
        4 => Ok(green_average_decorrelation(rgba)),
        5 => pack_palette_index_transform(rgba),
        _ => bail!("unsupported transform id: {transform_id}"),
    }
}

pub fn reverse_transform(transform_id: u8, transformed: &[u8]) -> Result<Vec<u8>> {
    match transform_id {
        0 => Ok(transformed.to_vec()),
        1 => Ok(add_green_back(transformed)),
        2 => Ok(reverse_ycocg_r(transformed)),
        3 => Ok(reverse_alpha_plane_separation(transformed)),
        4 => Ok(reverse_green_average_decorrelation(transformed)),
        5 => unpack_palette_index_transform(transformed),
        _ => bail!("unsupported transform id: {transform_id}"),
    }
}

pub fn transform_ids_for_tile(rgba: &[u8]) -> Vec<u8> {
    let mut ids = vec![0, 1, 2, 3, 4];
    if palette_index_transform_possible(rgba) {
        ids.push(5);
    }
    ids
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

fn ycocg_r(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let r = px[0];
        let g = px[1];
        let b = px[2];
        let a = px[3];

        let co = r.wrapping_sub(b);
        let t = b.wrapping_add(co >> 1);
        let cg = g.wrapping_sub(t);
        let y = t.wrapping_add(cg >> 1);

        out.push(y);
        out.push(co);
        out.push(cg);
        out.push(a);
    }
    out
}

fn reverse_ycocg_r(transformed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(transformed.len());
    for px in transformed.chunks_exact(4) {
        let y = px[0];
        let co = px[1];
        let cg = px[2];
        let a = px[3];

        let t = y.wrapping_sub(cg >> 1);
        let g = cg.wrapping_add(t);
        let b = t.wrapping_sub(co >> 1);
        let r = b.wrapping_add(co);

        out.push(r);
        out.push(g);
        out.push(b);
        out.push(a);
    }
    out
}

fn alpha_plane_separation(rgba: &[u8]) -> Vec<u8> {
    let pixel_count = rgba.len() / 4;
    let mut out = vec![0_u8; rgba.len()];
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        out[i] = px[3];
        out[pixel_count + i] = px[0];
        out[pixel_count * 2 + i] = px[1];
        out[pixel_count * 3 + i] = px[2];
    }
    out
}

fn reverse_alpha_plane_separation(transformed: &[u8]) -> Vec<u8> {
    let pixel_count = transformed.len() / 4;
    let mut out = Vec::with_capacity(transformed.len());
    for i in 0..pixel_count {
        out.push(transformed[pixel_count + i]);
        out.push(transformed[pixel_count * 2 + i]);
        out.push(transformed[pixel_count * 3 + i]);
        out.push(transformed[i]);
    }
    out
}

fn green_average_decorrelation(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let r = px[0];
        let g = px[1];
        let b = px[2];
        let a = px[3];
        let avg = ((r as u16 + b as u16) / 2) as u8;
        out.push(r);
        out.push(g.wrapping_sub(avg));
        out.push(b);
        out.push(a);
    }
    out
}

fn reverse_green_average_decorrelation(transformed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(transformed.len());
    for px in transformed.chunks_exact(4) {
        let r = px[0];
        let g_prime = px[1];
        let b = px[2];
        let a = px[3];
        let avg = ((r as u16 + b as u16) / 2) as u8;
        out.push(r);
        out.push(g_prime.wrapping_add(avg));
        out.push(b);
        out.push(a);
    }
    out
}

fn palette_index_transform_possible(rgba: &[u8]) -> bool {
    build_palette_index_data(rgba).is_some()
}

fn pack_palette_index_transform(rgba: &[u8]) -> Result<Vec<u8>> {
    let (palette, indices) = build_palette_index_data(rgba)
        .ok_or_else(|| anyhow::anyhow!("palette/index transform not applicable to this tile"))?;
    let pixel_count = rgba.len() / 4;
    let palette_bytes = palette.len() * 4;
    let mut out = vec![0_u8; rgba.len()];
    out[0] = (palette.len() - 1) as u8;
    let palette_start = 1;
    let indices_start = palette_start + palette_bytes;

    for (i, color) in palette.iter().enumerate() {
        let start = palette_start + i * 4;
        out[start..start + 4].copy_from_slice(color);
    }
    out[indices_start..indices_start + pixel_count].copy_from_slice(&indices);
    Ok(out)
}

fn unpack_palette_index_transform(transformed: &[u8]) -> Result<Vec<u8>> {
    if transformed.is_empty() || !transformed.len().is_multiple_of(4) {
        bail!("invalid palette/index transformed payload length");
    }
    let pixel_count = transformed.len() / 4;
    let palette_len = transformed[0] as usize + 1;
    let palette_bytes = palette_len
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("palette byte count overflow"))?;
    let indices_start = 1 + palette_bytes;
    let indices_end = indices_start
        .checked_add(pixel_count)
        .ok_or_else(|| anyhow::anyhow!("palette index section overflow"))?;
    if indices_end > transformed.len() {
        bail!("palette/index transformed payload is truncated");
    }

    let palette_bytes_slice = &transformed[1..1 + palette_bytes];
    let indices = &transformed[indices_start..indices_end];
    let mut out = Vec::with_capacity(transformed.len());
    for &index in indices {
        let idx = index as usize;
        if idx >= palette_len {
            bail!("palette index {idx} exceeds palette length {palette_len}");
        }
        let start = idx * 4;
        out.extend_from_slice(&palette_bytes_slice[start..start + 4]);
    }
    Ok(out)
}

fn build_palette_index_data(rgba: &[u8]) -> Option<(Vec<[u8; 4]>, Vec<u8>)> {
    if rgba.is_empty() || !rgba.len().is_multiple_of(4) {
        return None;
    }

    let pixel_count = rgba.len() / 4;
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut indices = Vec::with_capacity(pixel_count);

    for px in rgba.chunks_exact(4) {
        let color = [px[0], px[1], px[2], px[3]];
        let index = match palette.iter().position(|entry| *entry == color) {
            Some(existing) => existing,
            None => {
                if palette.len() == 256 {
                    return None;
                }
                palette.push(color);
                palette.len() - 1
            }
        };
        indices.push(index as u8);
    }

    let total_used = 1 + palette.len() * 4 + pixel_count;
    if total_used > rgba.len() {
        return None;
    }

    Some((palette, indices))
}
