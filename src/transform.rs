use anyhow::{Result, bail};

pub const TRANSFORM_COUNT: u8 = 8;
pub const STRUCTURED_PLANE_TRANSFORM_ID: u8 = 6;
pub const QOI_CACHE_TRANSFORM_ID: u8 = 7;
const PLANE_MODE_RAW: u8 = 0;
const PLANE_MODE_CONSTANT: u8 = 1;
const PLANE_MODE_GLOBAL_AFFINE_SPARSE: u8 = 2;
const PLANE_MODE_ROW_AFFINE_SPARSE: u8 = 3;
const PLANE_MODE_PALETTE_RLE: u8 = 4;
const PLANE_MODE_PALETTE_BITPACK: u8 = 5;
const PLANE_MODE_BLOCK_PULSE: u8 = 6;

#[derive(Clone, Copy)]
struct BlockPulseSpec {
    block_size: usize,
    period: u8,
    offset: u8,
}

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
    let mut ids = vec![
        0,
        1,
        2,
        3,
        4,
        STRUCTURED_PLANE_TRANSFORM_ID,
        QOI_CACHE_TRANSFORM_ID,
    ];
    if palette_index_transform_possible(rgba) {
        ids.push(5);
    }
    ids
}

pub fn is_special_transform(transform_id: u8) -> bool {
    matches!(
        transform_id,
        STRUCTURED_PLANE_TRANSFORM_ID | QOI_CACHE_TRANSFORM_ID
    )
}

pub fn encode_special_transform(
    transform_id: u8,
    rgba: &[u8],
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    match transform_id {
        STRUCTURED_PLANE_TRANSFORM_ID => encode_structured_plane_transform(rgba, width, height),
        QOI_CACHE_TRANSFORM_ID => encode_qoi_cache_transform(rgba, width, height),
        _ => bail!("unsupported special transform id: {transform_id}"),
    }
}

pub fn decode_special_transform(
    transform_id: u8,
    payload: &[u8],
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    match transform_id {
        STRUCTURED_PLANE_TRANSFORM_ID => decode_structured_plane_transform(payload, width, height),
        QOI_CACHE_TRANSFORM_ID => decode_qoi_cache_transform(payload, width, height),
        _ => bail!("unsupported special transform id: {transform_id}"),
    }
}

const QOI_OP_RGB: u8 = 0xFE;
const QOI_OP_RGBA: u8 = 0xFF;
const QOI_MAX_SEED_COLORS: usize = 8;
const QOI_MASK_2: u8 = 0xC0;
const QOI_OP_INDEX: u8 = 0x00;
const QOI_OP_DIFF: u8 = 0x40;
const QOI_OP_LUMA: u8 = 0x80;
const QOI_OP_RUN: u8 = 0xC0;

fn encode_qoi_cache_transform(rgba: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let pixel_count = usize::from(width)
        .checked_mul(usize::from(height))
        .ok_or_else(|| anyhow::anyhow!("QOI cache pixel count overflow"))?;
    if rgba.len() != pixel_count * 4 {
        bail!("QOI cache transform input length mismatch");
    }

    let seed_colors = qoi_seed_colors(rgba);
    let mut index = [[0_u8; 4]; 64];
    for &color in &seed_colors {
        index[qoi_hash(color)] = color;
    }
    let mut prev = [0_u8, 0_u8, 0_u8, 255_u8];
    let mut run = 0_u8;
    let mut out = Vec::with_capacity(1 + seed_colors.len() * 4 + rgba.len());
    out.push(seed_colors.len() as u8);
    for &color in &seed_colors {
        out.extend_from_slice(&color);
    }

    for px in rgba.chunks_exact(4) {
        let current = [px[0], px[1], px[2], px[3]];
        if current == prev {
            run = run.saturating_add(1);
            if run == 62 {
                out.push(QOI_OP_RUN | (run - 1));
                run = 0;
            }
            continue;
        }

        if run > 0 {
            out.push(QOI_OP_RUN | (run - 1));
            run = 0;
        }

        let cache_index = qoi_hash(current);
        if index[cache_index] == current {
            out.push(QOI_OP_INDEX | cache_index as u8);
            prev = current;
            continue;
        }

        index[cache_index] = current;
        if current[3] == prev[3] {
            let dr = i16::from(current[0]) - i16::from(prev[0]);
            let dg = i16::from(current[1]) - i16::from(prev[1]);
            let db = i16::from(current[2]) - i16::from(prev[2]);
            if (-2..=1).contains(&dr) && (-2..=1).contains(&dg) && (-2..=1).contains(&db) {
                out.push(
                    QOI_OP_DIFF | ((dr + 2) as u8) << 4 | ((dg + 2) as u8) << 2 | (db + 2) as u8,
                );
            } else {
                let dr_dg = dr - dg;
                let db_dg = db - dg;
                if (-32..=31).contains(&dg)
                    && (-8..=7).contains(&dr_dg)
                    && (-8..=7).contains(&db_dg)
                {
                    out.push(QOI_OP_LUMA | (dg + 32) as u8);
                    out.push(((dr_dg + 8) as u8) << 4 | (db_dg + 8) as u8);
                } else {
                    out.push(QOI_OP_RGB);
                    out.extend_from_slice(&current[..3]);
                }
            }
        } else {
            out.push(QOI_OP_RGBA);
            out.extend_from_slice(&current);
        }
        prev = current;
    }

    if run > 0 {
        out.push(QOI_OP_RUN | (run - 1));
    }

    Ok(out)
}

fn decode_qoi_cache_transform(payload: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let pixel_count = usize::from(width)
        .checked_mul(usize::from(height))
        .ok_or_else(|| anyhow::anyhow!("QOI cache pixel count overflow"))?;
    if payload.is_empty() {
        bail!("QOI cache payload truncated before seed header");
    }
    let seed_len = usize::from(payload[0]);
    if seed_len > QOI_MAX_SEED_COLORS {
        bail!("QOI cache seed palette too large");
    }
    let seed_bytes = seed_len
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("QOI cache seed size overflow"))?;
    if payload.len() < 1 + seed_bytes {
        bail!("QOI cache payload truncated in seed palette");
    }

    let mut index = [[0_u8; 4]; 64];
    let mut cursor = 1_usize;
    for _ in 0..seed_len {
        let color = [
            payload[cursor],
            payload[cursor + 1],
            payload[cursor + 2],
            payload[cursor + 3],
        ];
        index[qoi_hash(color)] = color;
        cursor += 4;
    }
    let mut prev = [0_u8, 0_u8, 0_u8, 255_u8];
    let mut out = Vec::with_capacity(pixel_count * 4);
    let mut emitted = 0_usize;

    while emitted < pixel_count {
        if cursor >= payload.len() {
            bail!("QOI cache payload truncated");
        }
        let op = payload[cursor];
        cursor += 1;

        let current = match op {
            QOI_OP_RGB => {
                if cursor + 3 > payload.len() {
                    bail!("QOI cache RGB payload truncated");
                }
                [
                    payload[cursor],
                    payload[cursor + 1],
                    payload[cursor + 2],
                    prev[3],
                ]
            }
            QOI_OP_RGBA => {
                if cursor + 4 > payload.len() {
                    bail!("QOI cache RGBA payload truncated");
                }
                [
                    payload[cursor],
                    payload[cursor + 1],
                    payload[cursor + 2],
                    payload[cursor + 3],
                ]
            }
            _ => match op & QOI_MASK_2 {
                QOI_OP_INDEX => index[(op & 0x3F) as usize],
                QOI_OP_DIFF => {
                    let dr = ((op >> 4) & 0x03) as i8 - 2;
                    let dg = ((op >> 2) & 0x03) as i8 - 2;
                    let db = (op & 0x03) as i8 - 2;
                    [
                        prev[0].wrapping_add_signed(dr),
                        prev[1].wrapping_add_signed(dg),
                        prev[2].wrapping_add_signed(db),
                        prev[3],
                    ]
                }
                QOI_OP_LUMA => {
                    if cursor >= payload.len() {
                        bail!("QOI cache luma payload truncated");
                    }
                    let next = payload[cursor];
                    cursor += 1;
                    let dg = (op & 0x3F) as i8 - 32;
                    let dr_dg = ((next >> 4) & 0x0F) as i8 - 8;
                    let db_dg = (next & 0x0F) as i8 - 8;
                    [
                        prev[0].wrapping_add_signed(dg + dr_dg),
                        prev[1].wrapping_add_signed(dg),
                        prev[2].wrapping_add_signed(dg + db_dg),
                        prev[3],
                    ]
                }
                QOI_OP_RUN => {
                    let run = usize::from(op & 0x3F) + 1;
                    let new_len = emitted
                        .checked_add(run)
                        .ok_or_else(|| anyhow::anyhow!("QOI cache run overflow"))?;
                    if new_len > pixel_count {
                        bail!("QOI cache run exceeds pixel count");
                    }
                    for _ in 0..run {
                        out.extend_from_slice(&prev);
                    }
                    emitted = new_len;
                    continue;
                }
                _ => unreachable!(),
            },
        };

        if matches!(op, QOI_OP_RGB) {
            cursor += 3;
        } else if matches!(op, QOI_OP_RGBA) {
            cursor += 4;
        }
        out.extend_from_slice(&current);
        prev = current;
        index[qoi_hash(current)] = current;
        emitted += 1;
    }

    if cursor != payload.len() {
        bail!("QOI cache payload has trailing bytes");
    }
    Ok(out)
}

fn qoi_hash(px: [u8; 4]) -> usize {
    (usize::from(px[0]) * 3
        + usize::from(px[1]) * 5
        + usize::from(px[2]) * 7
        + usize::from(px[3]) * 11)
        % 64
}

fn qoi_seed_colors(rgba: &[u8]) -> Vec<[u8; 4]> {
    let mut counts: Vec<([u8; 4], usize)> = Vec::new();
    for px in rgba.chunks_exact(4) {
        let color = [px[0], px[1], px[2], px[3]];
        if let Some((_, count)) = counts.iter_mut().find(|(entry, _)| *entry == color) {
            *count += 1;
        } else {
            counts.push((color, 1));
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts
        .into_iter()
        .take(QOI_MAX_SEED_COLORS)
        .map(|(color, _)| color)
        .collect()
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

fn encode_structured_plane_transform(rgba: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    if rgba.len() != width as usize * height as usize * 4 {
        bail!("structured plane transform input length mismatch");
    }
    let planes = split_rgba_planes(rgba);
    let mut out = Vec::new();
    for plane in &planes {
        let (mode, payload) = encode_best_plane_mode(plane, width as usize, height as usize)?;
        out.push(mode);
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| anyhow::anyhow!("structured plane payload exceeds u32"))?;
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&payload);
    }
    Ok(out)
}

fn decode_structured_plane_transform(payload: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let pixel_count = width as usize * height as usize;
    let mut cursor = 0_usize;
    let mut planes = Vec::with_capacity(4);
    for _ in 0..4 {
        if cursor + 5 > payload.len() {
            bail!("structured plane payload truncated before plane header");
        }
        let mode = payload[cursor];
        cursor += 1;
        let payload_len = u32::from_le_bytes([
            payload[cursor],
            payload[cursor + 1],
            payload[cursor + 2],
            payload[cursor + 3],
        ]) as usize;
        cursor += 4;
        if cursor + payload_len > payload.len() {
            bail!("structured plane payload truncated inside plane data");
        }
        let plane_payload = &payload[cursor..cursor + payload_len];
        cursor += payload_len;
        let plane = decode_plane_mode(mode, plane_payload, width as usize, height as usize)?;
        if plane.len() != pixel_count {
            bail!("structured plane decode produced wrong plane length");
        }
        planes.push(plane);
    }
    if cursor != payload.len() {
        bail!("structured plane payload has trailing bytes");
    }
    interleave_rgba_planes(&planes, pixel_count)
}

fn split_rgba_planes(rgba: &[u8]) -> [Vec<u8>; 4] {
    let pixel_count = rgba.len() / 4;
    let mut planes = [
        Vec::with_capacity(pixel_count),
        Vec::with_capacity(pixel_count),
        Vec::with_capacity(pixel_count),
        Vec::with_capacity(pixel_count),
    ];
    for px in rgba.chunks_exact(4) {
        planes[0].push(px[0]);
        planes[1].push(px[1]);
        planes[2].push(px[2]);
        planes[3].push(px[3]);
    }
    planes
}

fn interleave_rgba_planes(planes: &[Vec<u8>], pixel_count: usize) -> Result<Vec<u8>> {
    if planes.len() != 4 || planes.iter().any(|plane| plane.len() != pixel_count) {
        bail!("invalid plane set for RGBA interleave");
    }
    let mut out = Vec::with_capacity(pixel_count * 4);
    for (((&r, &g), &b), &a) in planes[0]
        .iter()
        .zip(&planes[1])
        .zip(&planes[2])
        .zip(&planes[3])
    {
        out.push(r);
        out.push(g);
        out.push(b);
        out.push(a);
    }
    Ok(out)
}

fn encode_best_plane_mode(plane: &[u8], width: usize, height: usize) -> Result<(u8, Vec<u8>)> {
    let mut best_mode = PLANE_MODE_RAW;
    let mut best_payload = plane.to_vec();

    for candidate in [
        encode_constant_plane(plane),
        encode_global_affine_sparse_plane(plane, width, height)?,
        encode_row_affine_sparse_plane(plane, width, height)?,
        encode_palette_rle_plane(plane)?,
        encode_palette_bitpack_plane(plane)?,
        encode_block_pulse_plane(plane, width, height)?,
    ]
    .into_iter()
    .flatten()
    {
        let (mode, payload) = candidate;
        if payload.len() < best_payload.len() {
            best_mode = mode;
            best_payload = payload;
        }
    }

    Ok((best_mode, best_payload))
}

fn decode_plane_mode(mode: u8, payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    match mode {
        PLANE_MODE_RAW => {
            let expected_len = width
                .checked_mul(height)
                .ok_or_else(|| anyhow::anyhow!("plane length overflow"))?;
            if payload.len() != expected_len {
                bail!("raw plane payload length mismatch");
            }
            Ok(payload.to_vec())
        }
        PLANE_MODE_CONSTANT => decode_constant_plane(payload, width, height),
        PLANE_MODE_GLOBAL_AFFINE_SPARSE => {
            decode_global_affine_sparse_plane(payload, width, height)
        }
        PLANE_MODE_ROW_AFFINE_SPARSE => decode_row_affine_sparse_plane(payload, width, height),
        PLANE_MODE_PALETTE_RLE => decode_palette_rle_plane(payload, width, height),
        PLANE_MODE_PALETTE_BITPACK => decode_palette_bitpack_plane(payload, width, height),
        PLANE_MODE_BLOCK_PULSE => decode_block_pulse_plane(payload, width, height),
        _ => bail!("unsupported structured plane mode: {mode}"),
    }
}

fn encode_constant_plane(plane: &[u8]) -> Option<(u8, Vec<u8>)> {
    let first = *plane.first()?;
    if plane.iter().all(|&value| value == first) {
        Some((PLANE_MODE_CONSTANT, vec![first]))
    } else {
        None
    }
}

fn decode_constant_plane(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if payload.len() != 1 {
        bail!("constant plane payload must contain exactly one byte");
    }
    Ok(vec![payload[0]; width * height])
}

fn encode_global_affine_sparse_plane(
    plane: &[u8],
    width: usize,
    height: usize,
) -> Result<Option<(u8, Vec<u8>)>> {
    if plane.is_empty() {
        return Ok(None);
    }
    let base = plane[0];
    let dx_candidates = collect_delta_candidates(plane, width, true);
    let dy_candidates = collect_delta_candidates(plane, width, false);
    let mut best: Option<Vec<u8>> = None;
    for dx in dx_candidates {
        for &dy in &dy_candidates {
            let payload = encode_global_affine_payload(plane, width, height, base, dx, dy)?;
            if best
                .as_ref()
                .is_none_or(|current| payload.len() < current.len())
            {
                best = Some(payload);
            }
        }
    }
    Ok(best.map(|payload| (PLANE_MODE_GLOBAL_AFFINE_SPARSE, payload)))
}

fn decode_global_affine_sparse_plane(
    payload: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    if payload.len() < 3 {
        bail!("global affine plane payload too short");
    }
    let base = payload[0];
    let dx = payload[1];
    let dy = payload[2];
    let pixel_count = width * height;
    let mut plane = Vec::with_capacity(pixel_count);
    for y in 0..height {
        for x in 0..width {
            plane.push(
                base.wrapping_add((x as u8).wrapping_mul(dx))
                    .wrapping_add((y as u8).wrapping_mul(dy)),
            );
        }
    }
    apply_sparse_exceptions(&mut plane, &payload[3..])?;
    Ok(plane)
}

fn encode_global_affine_payload(
    plane: &[u8],
    width: usize,
    height: usize,
    base: u8,
    dx: u8,
    dy: u8,
) -> Result<Vec<u8>> {
    let mut payload = vec![base, dx, dy];
    let exceptions = encode_sparse_exceptions(plane.len(), |idx| {
        let x = idx % width;
        let y = idx / width;
        let predicted = base
            .wrapping_add((x as u8).wrapping_mul(dx))
            .wrapping_add((y as u8).wrapping_mul(dy));
        plane[idx].wrapping_sub(predicted)
    })?;
    payload.extend_from_slice(&exceptions);
    let _ = height;
    Ok(payload)
}

fn encode_row_affine_sparse_plane(
    plane: &[u8],
    width: usize,
    height: usize,
) -> Result<Option<(u8, Vec<u8>)>> {
    if plane.is_empty() {
        return Ok(None);
    }
    let mut starts = Vec::with_capacity(height);
    let mut steps = Vec::with_capacity(height);
    for y in 0..height {
        let row_start = y * width;
        let row = &plane[row_start..row_start + width];
        starts.push(row[0]);
        steps.push(select_best_row_step(row));
    }

    let mut payload = Vec::with_capacity(height * 2);
    for (&start, &step) in starts.iter().zip(&steps) {
        payload.push(start);
        payload.push(step);
    }
    let exceptions = encode_sparse_exceptions(plane.len(), |idx| {
        let y = idx / width;
        let x = idx % width;
        let predicted = starts[y].wrapping_add((x as u8).wrapping_mul(steps[y]));
        plane[idx].wrapping_sub(predicted)
    })?;
    payload.extend_from_slice(&exceptions);
    Ok(Some((PLANE_MODE_ROW_AFFINE_SPARSE, payload)))
}

fn decode_row_affine_sparse_plane(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let header_len = height
        .checked_mul(2)
        .ok_or_else(|| anyhow::anyhow!("row affine plane header length overflow"))?;
    if payload.len() < header_len {
        bail!("row affine plane payload too short");
    }
    let pixel_count = width * height;
    let mut plane = Vec::with_capacity(pixel_count);
    for y in 0..height {
        let start = payload[y * 2];
        let step = payload[y * 2 + 1];
        for x in 0..width {
            plane.push(start.wrapping_add((x as u8).wrapping_mul(step)));
        }
    }
    apply_sparse_exceptions(&mut plane, &payload[header_len..])?;
    Ok(plane)
}

fn encode_palette_rle_plane(plane: &[u8]) -> Result<Option<(u8, Vec<u8>)>> {
    let Some(palette) = build_plane_palette(plane, 16) else {
        return Ok(None);
    };
    let mut payload = Vec::new();
    payload.push((palette.len() - 1) as u8);
    payload.extend_from_slice(&palette);

    let mut i = 0_usize;
    while i < plane.len() {
        let value = plane[i];
        let palette_index = palette
            .iter()
            .position(|&entry| entry == value)
            .ok_or_else(|| anyhow::anyhow!("plane palette entry missing"))?
            as u8;
        let mut run_len = 1_usize;
        while i + run_len < plane.len() && plane[i + run_len] == value {
            run_len += 1;
        }
        payload.push(palette_index);
        write_varint(run_len, &mut payload)?;
        i += run_len;
    }
    Ok(Some((PLANE_MODE_PALETTE_RLE, payload)))
}

fn decode_palette_rle_plane(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if payload.is_empty() {
        bail!("palette RLE plane payload too short");
    }
    let palette_len = payload[0] as usize + 1;
    if payload.len() < 1 + palette_len {
        bail!("palette RLE plane payload truncated before palette");
    }
    let palette = &payload[1..1 + palette_len];
    let mut cursor = 1 + palette_len;
    let pixel_count = width * height;
    let mut plane = Vec::with_capacity(pixel_count);
    while plane.len() < pixel_count {
        if cursor >= payload.len() {
            bail!("palette RLE plane payload ended before filling plane");
        }
        let index = payload[cursor] as usize;
        cursor += 1;
        if index >= palette_len {
            bail!("palette RLE index out of range");
        }
        let run_len = read_varint(payload, &mut cursor)?;
        let new_len = plane
            .len()
            .checked_add(run_len)
            .ok_or_else(|| anyhow::anyhow!("palette RLE run length overflow"))?;
        if new_len > pixel_count {
            bail!("palette RLE run exceeds plane length");
        }
        plane.resize(new_len, palette[index]);
    }
    if cursor != payload.len() {
        bail!("palette RLE plane payload has trailing bytes");
    }
    Ok(plane)
}

fn encode_palette_bitpack_plane(plane: &[u8]) -> Result<Option<(u8, Vec<u8>)>> {
    let Some(palette) = build_plane_palette(plane, 16) else {
        return Ok(None);
    };
    let bits_per_index = if palette.len() <= 2 {
        1
    } else if palette.len() <= 4 {
        2
    } else {
        4
    };
    let mut indices = Vec::with_capacity(plane.len());
    for &value in plane {
        let index = palette
            .iter()
            .position(|&entry| entry == value)
            .ok_or_else(|| anyhow::anyhow!("plane palette entry missing"))?
            as u8;
        indices.push(index);
    }
    let packed = pack_plane_indices(&indices, bits_per_index)?;
    let mut payload = Vec::with_capacity(2 + palette.len() + packed.len());
    payload.push((palette.len() - 1) as u8);
    payload.push(bits_per_index);
    payload.extend_from_slice(&palette);
    payload.extend_from_slice(&packed);
    Ok(Some((PLANE_MODE_PALETTE_BITPACK, payload)))
}

fn decode_palette_bitpack_plane(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if payload.len() < 2 {
        bail!("palette bitpack plane payload too short");
    }
    let palette_len = payload[0] as usize + 1;
    let bits_per_index = payload[1];
    if !matches!(bits_per_index, 1 | 2 | 4 | 8) {
        bail!("invalid plane palette bits-per-index");
    }
    if payload.len() < 2 + palette_len {
        bail!("palette bitpack plane payload truncated before palette");
    }
    let palette = &payload[2..2 + palette_len];
    let packed = &payload[2 + palette_len..];
    let pixel_count = width * height;
    let indices = unpack_plane_indices(packed, bits_per_index, pixel_count)?;
    let mut plane = Vec::with_capacity(pixel_count);
    for index in indices {
        let idx = index as usize;
        if idx >= palette_len {
            bail!("plane palette index out of range");
        }
        plane.push(palette[idx]);
    }
    Ok(plane)
}

fn encode_block_pulse_plane(
    plane: &[u8],
    width: usize,
    height: usize,
) -> Result<Option<(u8, Vec<u8>)>> {
    let Some(plane_palette) = build_plane_palette(plane, 16) else {
        return Ok(None);
    };

    let mut best: Option<Vec<u8>> = None;
    for block_size in [4_usize, 8, 16, 32] {
        let blocks_x = width.div_ceil(block_size);
        let blocks_y = height.div_ceil(block_size);
        for period in 2_u8..=32 {
            for offset in 0_u8..period {
                for &override_value in &plane_palette {
                    let spec = BlockPulseSpec {
                        block_size,
                        period,
                        offset,
                    };
                    let mut block_values = Vec::with_capacity(blocks_x * blocks_y);
                    for block_y in 0..blocks_y {
                        for block_x in 0..blocks_x {
                            block_values.push(select_block_base_value(
                                plane, width, height, block_x, block_y, spec,
                            ));
                        }
                    }
                    let Some(block_palette) = build_plane_palette(&block_values, 16) else {
                        continue;
                    };
                    let bits_per_block = palette_bits_for_len(block_palette.len());
                    let mut block_indices = Vec::with_capacity(block_values.len());
                    for &value in &block_values {
                        let index = block_palette
                            .iter()
                            .position(|&entry| entry == value)
                            .ok_or_else(|| anyhow::anyhow!("missing block palette value"))?
                            as u8;
                        block_indices.push(index);
                    }
                    let packed_blocks = pack_plane_indices(&block_indices, bits_per_block)?;

                    let mut payload = vec![
                        block_size as u8,
                        period,
                        offset,
                        override_value,
                        (block_palette.len() - 1) as u8,
                        bits_per_block,
                    ];
                    payload.extend_from_slice(&block_palette);
                    payload.extend_from_slice(&packed_blocks);
                    let exceptions = encode_sparse_exceptions(plane.len(), |idx| {
                        let x = idx % width;
                        let y = idx / width;
                        let block_idx = (y / block_size) * blocks_x + (x / block_size);
                        let base = block_values[block_idx];
                        let predicted = if (x + y + offset as usize).is_multiple_of(period as usize)
                        {
                            override_value
                        } else {
                            base
                        };
                        plane[idx].wrapping_sub(predicted)
                    })?;
                    payload.extend_from_slice(&exceptions);

                    if best
                        .as_ref()
                        .is_none_or(|current| payload.len() < current.len())
                    {
                        best = Some(payload);
                    }
                }
            }
        }
    }

    Ok(best.map(|payload| (PLANE_MODE_BLOCK_PULSE, payload)))
}

fn decode_block_pulse_plane(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if payload.len() < 6 {
        bail!("block pulse plane payload too short");
    }
    let block_size = payload[0] as usize;
    let period = payload[1] as usize;
    let offset = payload[2] as usize;
    let override_value = payload[3];
    let block_palette_len = payload[4] as usize + 1;
    let bits_per_block = payload[5];
    if block_size == 0 || period < 2 {
        bail!("invalid block pulse plane header values");
    }
    let palette_start = 6;
    let palette_end = palette_start + block_palette_len;
    if payload.len() < palette_end {
        bail!("block pulse plane payload truncated before palette");
    }
    let block_palette = &payload[palette_start..palette_end];
    let blocks_x = width.div_ceil(block_size);
    let blocks_y = height.div_ceil(block_size);
    let block_count = blocks_x
        .checked_mul(blocks_y)
        .ok_or_else(|| anyhow::anyhow!("block pulse block count overflow"))?;
    let packed_len = (block_count * bits_per_block as usize).div_ceil(8);
    let packed_start = palette_end;
    let packed_end = packed_start + packed_len;
    if payload.len() < packed_end {
        bail!("block pulse plane payload truncated before packed blocks");
    }
    let block_indices = unpack_plane_indices(
        &payload[packed_start..packed_end],
        bits_per_block,
        block_count,
    )?;
    let mut block_values = Vec::with_capacity(block_count);
    for index in block_indices {
        let idx = index as usize;
        if idx >= block_palette_len {
            bail!("block pulse plane palette index out of range");
        }
        block_values.push(block_palette[idx]);
    }

    let mut plane = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let block_idx = (y / block_size) * blocks_x + (x / block_size);
            let base = block_values[block_idx];
            let predicted = if (x + y + offset).is_multiple_of(period) {
                override_value
            } else {
                base
            };
            plane.push(predicted);
        }
    }
    apply_sparse_exceptions(&mut plane, &payload[packed_end..])?;
    Ok(plane)
}

fn collect_delta_candidates(plane: &[u8], width: usize, horizontal: bool) -> Vec<u8> {
    let mut candidates = vec![0];
    if horizontal {
        for pair in plane[..width.min(8)].windows(2) {
            let delta = pair[1].wrapping_sub(pair[0]);
            if !candidates.contains(&delta) {
                candidates.push(delta);
            }
        }
    } else {
        let max_y = (plane.len() / width).min(8);
        for y in 1..max_y {
            let delta = plane[y * width].wrapping_sub(plane[(y - 1) * width]);
            if !candidates.contains(&delta) {
                candidates.push(delta);
            }
        }
    }
    candidates
}

fn select_best_row_step(row: &[u8]) -> u8 {
    if row.len() < 2 {
        return 0;
    }
    let mut candidates = vec![0, row[1].wrapping_sub(row[0])];
    for pair in row.windows(2).take(8) {
        let delta = pair[1].wrapping_sub(pair[0]);
        if !candidates.contains(&delta) {
            candidates.push(delta);
        }
    }

    let mut best_step = candidates[0];
    let mut best_score = usize::MAX;
    for step in candidates {
        let start = row[0];
        let mut score = 0_usize;
        for (x, &actual) in row.iter().enumerate() {
            let predicted = start.wrapping_add((x as u8).wrapping_mul(step));
            if actual != predicted {
                score += 1;
            }
        }
        if score < best_score {
            best_score = score;
            best_step = step;
        }
    }
    best_step
}

fn encode_sparse_exceptions<F>(len: usize, mut residual_at: F) -> Result<Vec<u8>>
where
    F: FnMut(usize) -> u8,
{
    let mut out = Vec::new();
    let mut last_pos = 0_usize;
    let mut nonzero_count = 0_usize;
    let mut entries = Vec::new();
    for idx in 0..len {
        let residual = residual_at(idx);
        if residual != 0 {
            nonzero_count += 1;
            let delta = if entries.is_empty() {
                idx
            } else {
                idx - last_pos - 1
            };
            write_varint(delta, &mut entries)?;
            entries.push(residual);
            last_pos = idx;
        }
    }
    write_varint(nonzero_count, &mut out)?;
    out.extend_from_slice(&entries);
    Ok(out)
}

fn apply_sparse_exceptions(plane: &mut [u8], payload: &[u8]) -> Result<()> {
    let mut cursor = 0_usize;
    let nonzero_count = read_varint(payload, &mut cursor)?;
    let mut pos = 0_usize;
    for i in 0..nonzero_count {
        let delta = read_varint(payload, &mut cursor)?;
        pos = if i == 0 {
            delta
        } else {
            pos.checked_add(delta + 1)
                .ok_or_else(|| anyhow::anyhow!("sparse exception position overflow"))?
        };
        if pos >= plane.len() {
            bail!("sparse exception position out of range");
        }
        if cursor >= payload.len() {
            bail!("sparse exception payload truncated before residual byte");
        }
        plane[pos] = plane[pos].wrapping_add(payload[cursor]);
        cursor += 1;
    }
    if cursor != payload.len() {
        bail!("sparse exception payload has trailing bytes");
    }
    Ok(())
}

fn build_plane_palette(plane: &[u8], max_len: usize) -> Option<Vec<u8>> {
    let mut palette = Vec::new();
    for &value in plane {
        if !palette.contains(&value) {
            if palette.len() == max_len {
                return None;
            }
            palette.push(value);
        }
    }
    Some(palette)
}

fn palette_bits_for_len(palette_len: usize) -> u8 {
    if palette_len <= 2 {
        1
    } else if palette_len <= 4 {
        2
    } else if palette_len <= 16 {
        4
    } else {
        8
    }
}

fn select_block_base_value(
    plane: &[u8],
    width: usize,
    height: usize,
    block_x: usize,
    block_y: usize,
    spec: BlockPulseSpec,
) -> u8 {
    let start_x = block_x * spec.block_size;
    let start_y = block_y * spec.block_size;
    let end_x = (start_x + spec.block_size).min(width);
    let end_y = (start_y + spec.block_size).min(height);
    let mut primary_counts = [0_u16; 256];
    let mut fallback_counts = [0_u16; 256];
    for y in start_y..end_y {
        for x in start_x..end_x {
            let value = plane[y * width + x];
            fallback_counts[value as usize] += 1;
            if !(x + y + spec.offset as usize).is_multiple_of(spec.period as usize) {
                primary_counts[value as usize] += 1;
            }
        }
    }
    let counts = if primary_counts.iter().any(|&count| count > 0) {
        &primary_counts
    } else {
        &fallback_counts
    };
    let mut best_value = 0_u8;
    let mut best_count = 0_u16;
    for (value, &count) in counts.iter().enumerate() {
        if count > best_count {
            best_count = count;
            best_value = value as u8;
        }
    }
    best_value
}

fn pack_plane_indices(indices: &[u8], bits_per_index: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut current = 0_u8;
    let mut used_bits = 0_u8;
    for &index in indices {
        if bits_per_index < 8 && index >= (1_u8 << bits_per_index) {
            bail!("plane palette index exceeds bit width");
        }
        let mut remaining = bits_per_index;
        while remaining > 0 {
            let free_bits = 8 - used_bits;
            let take = remaining.min(free_bits);
            let shift = remaining - take;
            let mask = (1_u8 << take) - 1;
            let chunk = (index >> shift) & mask;
            current |= chunk << (free_bits - take);
            used_bits += take;
            remaining -= take;
            if used_bits == 8 {
                out.push(current);
                current = 0;
                used_bits = 0;
            }
        }
    }
    if used_bits > 0 {
        out.push(current);
    }
    Ok(out)
}

fn unpack_plane_indices(bytes: &[u8], bits_per_index: u8, count: usize) -> Result<Vec<u8>> {
    let expected_bytes = (count * bits_per_index as usize).div_ceil(8);
    if bytes.len() != expected_bytes {
        bail!("plane palette packed byte length mismatch");
    }
    let mut out = Vec::with_capacity(count);
    let mut byte_index = 0_usize;
    let mut bit_index = 0_u8;
    for _ in 0..count {
        let mut value = 0_u8;
        let mut remaining = bits_per_index;
        while remaining > 0 {
            let available = 8 - bit_index;
            let take = remaining.min(available);
            let shift = available - take;
            let mask = (1_u8 << take) - 1;
            let chunk = (bytes[byte_index] >> shift) & mask;
            value = (value << take) | chunk;
            bit_index += take;
            remaining -= take;
            if bit_index == 8 {
                bit_index = 0;
                byte_index += 1;
            }
        }
        out.push(value);
    }
    Ok(out)
}

fn write_varint(mut value: usize, out: &mut Vec<u8>) -> Result<()> {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return Ok(());
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<usize> {
    let mut value = 0_usize;
    let mut shift = 0_u32;
    loop {
        if *cursor >= bytes.len() {
            bail!("unexpected end of varint");
        }
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= usize::BITS {
            bail!("varint is too large");
        }
    }
}
