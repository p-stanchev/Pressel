use crate::predict::{ADAPTIVE_PREDICTOR_ID, expected_residual_len};
use anyhow::{Context, Result, bail};
use std::io::Cursor;

pub const ENTROPY_BACKEND_COUNT: u8 = 6;
const CHANNEL_SPLIT_HEADER_U32S: usize = 5;
const CHANNEL_SPLIT_HEADER_LEN: usize = CHANNEL_SPLIT_HEADER_U32S * 4;

pub fn encode_payload(backend_id: u8, residuals: &[u8]) -> Result<Vec<u8>> {
    match backend_id {
        0 => Ok(residuals.to_vec()),
        1 => Ok(zstd::stream::encode_all(Cursor::new(residuals), 9)?),
        2 => Ok(fold_residuals(residuals)),
        3 => Ok(zstd::stream::encode_all(
            Cursor::new(fold_residuals(residuals)),
            9,
        )?),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    }
}

pub fn decode_payload(backend_id: u8, payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let out = match backend_id {
        0 => payload.to_vec(),
        1 => zstd::stream::decode_all(Cursor::new(payload))?,
        2 => unfold_residuals(payload),
        3 => unfold_residuals(&zstd::stream::decode_all(Cursor::new(payload))?),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    };
    if out.len() != expected_len {
        bail!(
            "entropy decode length mismatch: expected {expected_len}, got {}",
            out.len()
        );
    }
    Ok(out)
}

pub fn encode_residual_payload(
    backend_id: u8,
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    match backend_id {
        0..=3 => encode_payload(backend_id, residuals),
        4 => encode_channel_split_payload(residuals, width, height, predictor_id, false),
        5 => encode_channel_split_payload(residuals, width, height, predictor_id, true),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    }
}

pub fn decode_residual_payload(
    backend_id: u8,
    payload: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let expected_len = expected_residual_len(width, height, predictor_id)?;
    match backend_id {
        0..=3 => decode_payload(backend_id, payload, expected_len),
        4 => decode_channel_split_payload(payload, width, height, predictor_id, false),
        5 => decode_channel_split_payload(payload, width, height, predictor_id, true),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    }
}

fn fold_residuals(residuals: &[u8]) -> Vec<u8> {
    residuals.iter().copied().map(fold_residual).collect()
}

fn unfold_residuals(folded: &[u8]) -> Vec<u8> {
    folded.iter().copied().map(unfold_residual).collect()
}

fn fold_residual(residual: u8) -> u8 {
    if residual <= 127 {
        residual.wrapping_mul(2)
    } else {
        (255 - residual).wrapping_mul(2).wrapping_add(1)
    }
}

fn unfold_residual(folded: u8) -> u8 {
    if folded & 1 == 0 {
        folded / 2
    } else {
        255 - (folded / 2)
    }
}

fn encode_channel_split_payload(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
    fold_channels: bool,
) -> Result<Vec<u8>> {
    let (map_bytes, mut channels) = split_residual_stream(residuals, width, height, predictor_id)?;
    if fold_channels {
        for channel in &mut channels {
            *channel = fold_residuals(channel);
        }
    }

    let map_payload = zstd::stream::encode_all(Cursor::new(map_bytes), 9)?;
    let channel_payloads = channels
        .map(|channel| zstd::stream::encode_all(Cursor::new(channel), 9))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(
        CHANNEL_SPLIT_HEADER_LEN
            + map_payload.len()
            + channel_payloads.iter().map(Vec::len).sum::<usize>(),
    );
    out.extend_from_slice(&(map_payload.len() as u32).to_le_bytes());
    for payload in &channel_payloads {
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(&map_payload);
    for payload in &channel_payloads {
        out.extend_from_slice(payload);
    }
    Ok(out)
}

fn decode_channel_split_payload(
    payload: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
    folded_channels: bool,
) -> Result<Vec<u8>> {
    if payload.len() < CHANNEL_SPLIT_HEADER_LEN {
        bail!("channel-split payload too short");
    }
    let lengths = read_u32_lengths(payload)?;
    let mut cursor = CHANNEL_SPLIT_HEADER_LEN;
    let mut segments = Vec::with_capacity(CHANNEL_SPLIT_HEADER_U32S);
    for &len_u32 in &lengths {
        let len = usize::try_from(len_u32).context("channel-split segment length exceeds usize")?;
        let end = cursor
            .checked_add(len)
            .context("channel-split payload length overflow")?;
        if end > payload.len() {
            bail!("channel-split payload truncated");
        }
        segments.push(&payload[cursor..end]);
        cursor = end;
    }
    if cursor != payload.len() {
        bail!("channel-split payload has trailing bytes");
    }

    let pixel_count = usize::from(width)
        .checked_mul(usize::from(height))
        .context("channel-split pixel count overflow")?;
    let map_len = residual_map_len(width, height, predictor_id)?;
    let map_bytes = zstd::stream::decode_all(Cursor::new(segments[0]))?;
    if map_bytes.len() != map_len {
        bail!(
            "channel-split map length mismatch: expected {}, got {}",
            map_len,
            map_bytes.len()
        );
    }

    let mut channels = std::array::from_fn::<_, 4, _>(|_| Vec::new());
    for channel in 0..4 {
        let mut bytes = zstd::stream::decode_all(Cursor::new(segments[channel + 1]))?;
        if folded_channels {
            bytes = unfold_residuals(&bytes);
        }
        if bytes.len() != pixel_count {
            bail!(
                "channel-split channel length mismatch: expected {}, got {}",
                pixel_count,
                bytes.len()
            );
        }
        channels[channel] = bytes;
    }

    merge_residual_stream(&map_bytes, &channels)
}

fn split_residual_stream(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<(Vec<u8>, [Vec<u8>; 4])> {
    let expected_len = expected_residual_len(width, height, predictor_id)?;
    if residuals.len() != expected_len {
        bail!(
            "residual length mismatch: expected {}, got {}",
            expected_len,
            residuals.len()
        );
    }
    let pixel_count = usize::from(width)
        .checked_mul(usize::from(height))
        .context("channel-split pixel count overflow")?;
    let map_len = residual_map_len(width, height, predictor_id)?;
    let map_bytes = residuals[..map_len].to_vec();
    let body = &residuals[map_len..];
    if body.len() != pixel_count * 4 {
        bail!("channel-split residual body length mismatch");
    }
    let mut channels = std::array::from_fn(|_| Vec::with_capacity(pixel_count));
    for chunk in body.chunks_exact(4) {
        for channel in 0..4 {
            channels[channel].push(chunk[channel]);
        }
    }
    Ok((map_bytes, channels))
}

fn merge_residual_stream(map_bytes: &[u8], channels: &[Vec<u8>; 4]) -> Result<Vec<u8>> {
    let pixel_count = channels[0].len();
    for channel in channels.iter().skip(1) {
        if channel.len() != pixel_count {
            bail!("channel-split channel length mismatch during merge");
        }
    }
    let mut out = Vec::with_capacity(
        map_bytes
            .len()
            .checked_add(
                pixel_count
                    .checked_mul(4)
                    .context("merge residual byte count overflow")?,
            )
            .context("merge residual allocation overflow")?,
    );
    out.extend_from_slice(map_bytes);
    for (((c0, c1), c2), c3) in channels[0]
        .iter()
        .zip(channels[1].iter())
        .zip(channels[2].iter())
        .zip(channels[3].iter())
    {
        out.push(*c0);
        out.push(*c1);
        out.push(*c2);
        out.push(*c3);
    }
    Ok(out)
}

fn residual_map_len(width: u16, height: u16, predictor_id: u8) -> Result<usize> {
    let total_len = expected_residual_len(width, height, predictor_id)?;
    let pixel_bytes = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|n| n.checked_mul(4))
        .context("residual map pixel byte count overflow")?;
    if predictor_id == ADAPTIVE_PREDICTOR_ID {
        total_len
            .checked_sub(pixel_bytes)
            .context("adaptive residual map length underflow")
    } else {
        Ok(0)
    }
}

fn read_u32_lengths(payload: &[u8]) -> Result<[u32; CHANNEL_SPLIT_HEADER_U32S]> {
    let mut lengths = [0_u32; CHANNEL_SPLIT_HEADER_U32S];
    for (idx, slot) in lengths.iter_mut().enumerate() {
        let start = idx * 4;
        let end = start + 4;
        *slot = u32::from_le_bytes(
            payload[start..end]
                .try_into()
                .expect("fixed 4-byte slice for channel-split header"),
        );
    }
    Ok(lengths)
}
