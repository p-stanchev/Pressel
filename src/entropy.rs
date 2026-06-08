use crate::predict::{expected_residual_len, residual_prefix_len};
use anyhow::{Context, Result, bail};
use constriction::stream::{
    Decode, model::DefaultContiguousCategoricalEntropyModel, stack::DefaultAnsCoder,
};
use std::io::Cursor;

pub const ENTROPY_BACKEND_COUNT: u8 = 9;
const CHANNEL_SPLIT_HEADER_U32S: usize = 5;
const CHANNEL_SPLIT_HEADER_LEN: usize = CHANNEL_SPLIT_HEADER_U32S * 4;
const CONTEXT_STREAM_COUNT: usize = 16;
const CONTEXT_SPLIT_HEADER_U32S: usize = 1 + CONTEXT_STREAM_COUNT;
const CONTEXT_SPLIT_HEADER_LEN: usize = CONTEXT_SPLIT_HEADER_U32S * 4;
const CONTEXT_RANS_HEADER_U32S: usize = 1 + CONTEXT_STREAM_COUNT + CONTEXT_STREAM_COUNT;
const CONTEXT_RANS_HEADER_LEN: usize = CONTEXT_RANS_HEADER_U32S * 4;
const RANS_SCALE_BITS: u32 = 12;
const RANS_TOTAL: u32 = 1 << RANS_SCALE_BITS;
const RANS_FREQ_TABLE_LEN: usize = 256 * 2;
const RANS_WORD_COUNT_LEN: usize = 4;
const SPARSE_RANS_COUNT_LEN: usize = 2;
const SPARSE_RANS_ENTRY_LEN: usize = 3;

pub fn encode_payload(backend_id: u8, residuals: &[u8]) -> Result<Vec<u8>> {
    match backend_id {
        0 => Ok(residuals.to_vec()),
        1 => Ok(zstd::stream::encode_all(Cursor::new(residuals), 9)?),
        2 => Ok(fold_residuals(residuals)),
        3 => Ok(zstd::stream::encode_all(
            Cursor::new(fold_residuals(residuals)),
            9,
        )?),
        6 => encode_rans_folded_payload(residuals),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    }
}

pub fn decode_payload(backend_id: u8, payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let out = match backend_id {
        0 => payload.to_vec(),
        1 => zstd::stream::decode_all(Cursor::new(payload))?,
        2 => unfold_residuals(payload),
        3 => unfold_residuals(&zstd::stream::decode_all(Cursor::new(payload))?),
        6 => decode_rans_folded_payload(payload, expected_len)?,
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
        0..=3 | 6 => encode_payload(backend_id, residuals),
        4 => encode_channel_split_payload(residuals, width, height, predictor_id, false),
        5 => encode_channel_split_payload(residuals, width, height, predictor_id, true),
        7 => encode_context_split_payload(residuals, width, height, predictor_id),
        8 => encode_context_rans_payload(residuals, width, height, predictor_id),
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
        0..=3 | 6 => decode_payload(backend_id, payload, expected_len),
        4 => decode_channel_split_payload(payload, width, height, predictor_id, false),
        5 => decode_channel_split_payload(payload, width, height, predictor_id, true),
        7 => decode_context_split_payload(payload, width, height, predictor_id),
        8 => decode_context_rans_payload(payload, width, height, predictor_id),
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
    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = zstd::stream::decode_all(Cursor::new(segments[0]))?;
    if prefix_bytes.len() != prefix_len {
        bail!(
            "channel-split prefix length mismatch: expected {}, got {}",
            prefix_len,
            prefix_bytes.len()
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

    merge_residual_stream(&prefix_bytes, &channels)
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
    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = residuals[..prefix_len].to_vec();
    let body = &residuals[prefix_len..];
    if body.len() != pixel_count * 4 {
        bail!("channel-split residual body length mismatch");
    }
    let mut channels = std::array::from_fn(|_| Vec::with_capacity(pixel_count));
    for chunk in body.chunks_exact(4) {
        for channel in 0..4 {
            channels[channel].push(chunk[channel]);
        }
    }
    Ok((prefix_bytes, channels))
}

fn merge_residual_stream(prefix_bytes: &[u8], channels: &[Vec<u8>; 4]) -> Result<Vec<u8>> {
    let pixel_count = channels[0].len();
    for channel in channels.iter().skip(1) {
        if channel.len() != pixel_count {
            bail!("channel-split channel length mismatch during merge");
        }
    }
    let mut out = Vec::with_capacity(
        prefix_bytes
            .len()
            .checked_add(
                pixel_count
                    .checked_mul(4)
                    .context("merge residual byte count overflow")?,
            )
            .context("merge residual allocation overflow")?,
    );
    out.extend_from_slice(prefix_bytes);
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

fn encode_context_split_payload(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = &residuals[..prefix_len];
    let body = &residuals[prefix_len..];
    if !body.len().is_multiple_of(4) {
        bail!("context-split residual body length must be divisible by 4");
    }

    let prefix_payload = zstd::stream::encode_all(Cursor::new(prefix_bytes), 9)?;
    let folded = fold_residuals(body);
    let streams = split_folded_context_streams(&folded, usize::from(width))?;
    let context_payloads = streams
        .iter()
        .map(|stream| zstd::stream::encode_all(Cursor::new(stream), 9))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(
        CONTEXT_SPLIT_HEADER_LEN
            + prefix_payload.len()
            + context_payloads.iter().map(Vec::len).sum::<usize>(),
    );
    out.extend_from_slice(&(prefix_payload.len() as u32).to_le_bytes());
    for payload in &context_payloads {
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(&prefix_payload);
    for payload in &context_payloads {
        out.extend_from_slice(payload);
    }
    Ok(out)
}

fn decode_context_split_payload(
    payload: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let expected_len = expected_residual_len(width, height, predictor_id)?;
    if payload.len() < CONTEXT_SPLIT_HEADER_LEN {
        bail!("context-split payload too short");
    }
    let lengths = read_context_u32_lengths(payload)?;
    let mut cursor = CONTEXT_SPLIT_HEADER_LEN;
    let mut segments = Vec::with_capacity(CONTEXT_SPLIT_HEADER_U32S);
    for &len_u32 in &lengths {
        let len = usize::try_from(len_u32).context("context-split segment length exceeds usize")?;
        let end = cursor
            .checked_add(len)
            .context("context-split payload length overflow")?;
        if end > payload.len() {
            bail!("context-split payload truncated");
        }
        segments.push(&payload[cursor..end]);
        cursor = end;
    }
    if cursor != payload.len() {
        bail!("context-split payload has trailing bytes");
    }

    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = zstd::stream::decode_all(Cursor::new(segments[0]))?;
    if prefix_bytes.len() != prefix_len {
        bail!(
            "context-split prefix length mismatch: expected {}, got {}",
            prefix_len,
            prefix_bytes.len()
        );
    }
    let body_len = expected_len - prefix_len;
    if body_len % 4 != 0 {
        bail!("context-split expected body length must be divisible by 4");
    }

    let mut streams = std::array::from_fn::<_, CONTEXT_STREAM_COUNT, _>(|_| Vec::new());
    for context in 0..CONTEXT_STREAM_COUNT {
        streams[context] = zstd::stream::decode_all(Cursor::new(segments[context + 1]))?;
    }
    let folded_body = merge_folded_context_streams(&streams, body_len, usize::from(width))?;
    let unfolded_body = unfold_residuals(&folded_body);

    let mut out = prefix_bytes;
    out.extend_from_slice(&unfolded_body);
    if out.len() != expected_len {
        bail!(
            "context-split decode length mismatch: expected {}, got {}",
            expected_len,
            out.len()
        );
    }
    Ok(out)
}

fn encode_context_rans_payload(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = &residuals[..prefix_len];
    let body = &residuals[prefix_len..];
    if !body.len().is_multiple_of(4) {
        bail!("context-rANS residual body length must be divisible by 4");
    }

    let prefix_payload = zstd::stream::encode_all(Cursor::new(prefix_bytes), 9)?;
    let folded = fold_residuals(body);
    let streams = split_folded_context_streams(&folded, usize::from(width))?;
    let context_payloads = streams
        .iter()
        .map(|stream| encode_sparse_rans_folded_bytes(stream))
        .collect::<Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(
        CONTEXT_RANS_HEADER_LEN
            + prefix_payload.len()
            + context_payloads.iter().map(Vec::len).sum::<usize>(),
    );
    out.extend_from_slice(&(prefix_payload.len() as u32).to_le_bytes());
    for payload in &context_payloads {
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    for stream in &streams {
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(&prefix_payload);
    for payload in &context_payloads {
        out.extend_from_slice(payload);
    }
    Ok(out)
}

fn decode_context_rans_payload(
    payload: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let expected_len = expected_residual_len(width, height, predictor_id)?;
    if payload.len() < CONTEXT_RANS_HEADER_LEN {
        bail!("context-rANS payload too short");
    }
    let header = read_context_rans_u32s(payload)?;
    let lengths = &header[..CONTEXT_STREAM_COUNT + 1];
    let counts = &header[CONTEXT_STREAM_COUNT + 1..];
    let mut cursor = CONTEXT_RANS_HEADER_LEN;
    let mut segments = Vec::with_capacity(CONTEXT_STREAM_COUNT + 1);
    for &len_u32 in lengths {
        let len = usize::try_from(len_u32).context("context-rANS segment length exceeds usize")?;
        let end = cursor
            .checked_add(len)
            .context("context-rANS payload length overflow")?;
        if end > payload.len() {
            bail!("context-rANS payload truncated");
        }
        segments.push(&payload[cursor..end]);
        cursor = end;
    }
    if cursor != payload.len() {
        bail!("context-rANS payload has trailing bytes");
    }

    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let prefix_bytes = zstd::stream::decode_all(Cursor::new(segments[0]))?;
    if prefix_bytes.len() != prefix_len {
        bail!(
            "context-rANS prefix length mismatch: expected {}, got {}",
            prefix_len,
            prefix_bytes.len()
        );
    }
    let body_len = expected_len - prefix_len;
    if body_len % 4 != 0 {
        bail!("context-rANS expected body length must be divisible by 4");
    }

    let mut streams = std::array::from_fn::<_, CONTEXT_STREAM_COUNT, _>(|_| Vec::new());
    for context in 0..CONTEXT_STREAM_COUNT {
        streams[context] = decode_sparse_rans_folded_bytes(
            segments[context + 1],
            usize::try_from(counts[context])
                .context("context-rANS stream symbol count exceeds usize")?,
        )?;
    }
    let folded_body = merge_folded_context_streams(&streams, body_len, usize::from(width))?;
    let unfolded_body = unfold_residuals(&folded_body);

    let mut out = prefix_bytes;
    out.extend_from_slice(&unfolded_body);
    if out.len() != expected_len {
        bail!(
            "context-rANS decode length mismatch: expected {}, got {}",
            expected_len,
            out.len()
        );
    }
    Ok(out)
}

fn encode_rans_folded_payload(residuals: &[u8]) -> Result<Vec<u8>> {
    let folded = fold_residuals(residuals);
    encode_dense_rans_folded_bytes(&folded)
}

fn encode_dense_rans_folded_bytes(folded: &[u8]) -> Result<Vec<u8>> {
    let freqs = normalized_rans_freqs(folded);
    let model = rans_model_from_freqs(&freqs)?;
    let mut coder = DefaultAnsCoder::new();
    let symbols = folded.iter().copied().map(usize::from);
    coder
        .encode_iid_symbols_reverse(symbols, &model)
        .map_err(|err| anyhow::anyhow!("rANS encode failed: {err}"))?;
    let compressed: Vec<u32> = coder.get_compressed()?.to_vec();

    let mut out =
        Vec::with_capacity(RANS_FREQ_TABLE_LEN + RANS_WORD_COUNT_LEN + compressed.len() * 4);
    for &freq in &freqs {
        out.extend_from_slice(&freq.to_le_bytes());
    }
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    for word in compressed {
        out.extend_from_slice(&word.to_le_bytes());
    }
    Ok(out)
}

fn decode_rans_folded_payload(payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let folded = decode_dense_rans_folded_bytes(payload, expected_len)?;
    Ok(unfold_residuals(&folded))
}

fn decode_dense_rans_folded_bytes(payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    if payload.len() < RANS_FREQ_TABLE_LEN + RANS_WORD_COUNT_LEN {
        bail!("rANS payload too short");
    }
    let freqs = read_rans_freqs(payload)?;
    let model = rans_model_from_freqs(&freqs)?;
    let words_offset = RANS_FREQ_TABLE_LEN;
    let word_count = u32::from_le_bytes(
        payload[words_offset..words_offset + RANS_WORD_COUNT_LEN]
            .try_into()
            .expect("fixed 4-byte rANS word count"),
    ) as usize;
    let words_bytes = &payload[words_offset + RANS_WORD_COUNT_LEN..];
    let expected_bytes = word_count
        .checked_mul(4)
        .context("rANS compressed word length overflow")?;
    if words_bytes.len() != expected_bytes {
        bail!(
            "rANS payload word data length mismatch: expected {}, got {}",
            expected_bytes,
            words_bytes.len()
        );
    }
    let mut compressed = Vec::with_capacity(word_count);
    for idx in 0..word_count {
        let start = idx * 4;
        compressed.push(u32::from_le_bytes(
            words_bytes[start..start + 4]
                .try_into()
                .expect("fixed 4-byte rANS word"),
        ));
    }
    let mut coder = DefaultAnsCoder::from_compressed(compressed)
        .map_err(|_| anyhow::anyhow!("rANS decode init failed"))?;
    let folded = coder
        .decode_iid_symbols(expected_len, &model)
        .map(|symbol| symbol.map(|value| value as u8))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| anyhow::anyhow!("rANS decode failed: {err}"))?;
    Ok(folded)
}

fn encode_sparse_rans_folded_bytes(folded: &[u8]) -> Result<Vec<u8>> {
    if folded.is_empty() {
        return Ok(Vec::new());
    }
    let freqs = normalized_rans_freqs(folded);
    let model = rans_model_from_freqs(&freqs)?;
    let mut coder = DefaultAnsCoder::new();
    let symbols = folded.iter().copied().map(usize::from);
    coder
        .encode_iid_symbols_reverse(symbols, &model)
        .map_err(|err| anyhow::anyhow!("context rANS encode failed: {err}"))?;
    let compressed: Vec<u32> = coder.get_compressed()?.to_vec();

    let nonzero = freqs
        .iter()
        .enumerate()
        .filter_map(|(symbol, &freq)| (freq > 0).then_some((symbol as u8, freq)))
        .collect::<Vec<_>>();
    let nonzero_len = u16::try_from(nonzero.len()).context("sparse rANS symbol count overflow")?;

    let mut out = Vec::with_capacity(
        SPARSE_RANS_COUNT_LEN
            + nonzero.len() * SPARSE_RANS_ENTRY_LEN
            + RANS_WORD_COUNT_LEN
            + compressed.len() * 4,
    );
    out.extend_from_slice(&nonzero_len.to_le_bytes());
    for (symbol, freq) in nonzero {
        out.push(symbol);
        out.extend_from_slice(&freq.to_le_bytes());
    }
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    for word in compressed {
        out.extend_from_slice(&word.to_le_bytes());
    }
    Ok(out)
}

fn decode_sparse_rans_folded_bytes(payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    if expected_len == 0 {
        if !payload.is_empty() {
            bail!("empty context stream should not have payload");
        }
        return Ok(Vec::new());
    }
    if payload.len() < SPARSE_RANS_COUNT_LEN + RANS_WORD_COUNT_LEN {
        bail!("sparse rANS payload too short");
    }
    let symbol_count = u16::from_le_bytes(
        payload[..SPARSE_RANS_COUNT_LEN]
            .try_into()
            .expect("fixed 2-byte sparse rANS count"),
    ) as usize;
    let table_len = symbol_count
        .checked_mul(SPARSE_RANS_ENTRY_LEN)
        .context("sparse rANS table length overflow")?;
    let words_offset = SPARSE_RANS_COUNT_LEN
        .checked_add(table_len)
        .context("sparse rANS words offset overflow")?;
    if payload.len() < words_offset + RANS_WORD_COUNT_LEN {
        bail!("sparse rANS payload truncated before word count");
    }

    let mut freqs = [0_u16; 256];
    let mut cursor = SPARSE_RANS_COUNT_LEN;
    for _ in 0..symbol_count {
        let symbol = payload[cursor];
        let freq = u16::from_le_bytes(
            payload[cursor + 1..cursor + 3]
                .try_into()
                .expect("fixed 2-byte sparse rANS freq"),
        );
        freqs[symbol as usize] = freq;
        cursor += SPARSE_RANS_ENTRY_LEN;
    }
    let total = freqs.iter().map(|&freq| u32::from(freq)).sum::<u32>();
    if total != RANS_TOTAL {
        bail!("invalid sparse rANS frequency total: expected {RANS_TOTAL}, got {total}");
    }
    let model = rans_model_from_freqs(&freqs)?;
    let word_count = u32::from_le_bytes(
        payload[words_offset..words_offset + RANS_WORD_COUNT_LEN]
            .try_into()
            .expect("fixed 4-byte sparse rANS word count"),
    ) as usize;
    let words_bytes = &payload[words_offset + RANS_WORD_COUNT_LEN..];
    let expected_bytes = word_count
        .checked_mul(4)
        .context("sparse rANS compressed word length overflow")?;
    if words_bytes.len() != expected_bytes {
        bail!(
            "sparse rANS payload word data length mismatch: expected {}, got {}",
            expected_bytes,
            words_bytes.len()
        );
    }
    let mut compressed = Vec::with_capacity(word_count);
    for idx in 0..word_count {
        let start = idx * 4;
        compressed.push(u32::from_le_bytes(
            words_bytes[start..start + 4]
                .try_into()
                .expect("fixed 4-byte sparse rANS word"),
        ));
    }
    let mut coder = DefaultAnsCoder::from_compressed(compressed)
        .map_err(|_| anyhow::anyhow!("sparse rANS decode init failed"))?;
    coder
        .decode_iid_symbols(expected_len, &model)
        .map(|symbol| symbol.map(|value| value as u8))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| anyhow::anyhow!("sparse rANS decode failed: {err}"))
}

fn normalized_rans_freqs(bytes: &[u8]) -> [u16; 256] {
    let mut hist = [0_u32; 256];
    for &byte in bytes {
        hist[byte as usize] += 1;
    }
    let total = hist.iter().sum::<u32>();
    if total == 0 {
        return [0_u16; 256];
    }

    let mut freqs = [0_u16; 256];
    let mut remainders = Vec::new();
    let mut sum = 0_u32;
    for (symbol, &count) in hist.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let scaled = u64::from(count) * u64::from(RANS_TOTAL);
        let mut freq = (scaled / u64::from(total)) as u16;
        if freq == 0 {
            freq = 1;
        }
        freqs[symbol] = freq;
        sum += u32::from(freq);
        remainders.push((symbol, (scaled % u64::from(total)) as u32));
    }

    if sum < RANS_TOTAL {
        remainders.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        let mut idx = 0;
        while sum < RANS_TOTAL {
            let symbol = remainders[idx % remainders.len()].0;
            freqs[symbol] += 1;
            sum += 1;
            idx += 1;
        }
    } else if sum > RANS_TOTAL {
        remainders.sort_by_key(|entry| entry.1);
        let mut idx = 0;
        while sum > RANS_TOTAL {
            let symbol = remainders[idx % remainders.len()].0;
            if freqs[symbol] > 1 {
                freqs[symbol] -= 1;
                sum -= 1;
            }
            idx += 1;
        }
    }

    freqs
}

fn read_rans_freqs(payload: &[u8]) -> Result<[u16; 256]> {
    let mut freqs = [0_u16; 256];
    for (idx, slot) in freqs.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u16::from_le_bytes(
            payload[start..end]
                .try_into()
                .expect("fixed 2-byte rANS frequency"),
        );
    }
    let total = freqs.iter().map(|&freq| u32::from(freq)).sum::<u32>();
    if total != 0 && total != RANS_TOTAL {
        bail!("invalid rANS frequency total: expected 0 or {RANS_TOTAL}, got {total}");
    }
    Ok(freqs)
}

fn rans_model_from_freqs(freqs: &[u16; 256]) -> Result<DefaultContiguousCategoricalEntropyModel> {
    let total = freqs.iter().map(|&freq| u32::from(freq)).sum::<u32>();
    if total == 0 {
        bail!("cannot build rANS model from an empty frequency table");
    }
    let probabilities = freqs
        .iter()
        .map(|&freq| f64::from(freq) / f64::from(RANS_TOTAL))
        .collect::<Vec<_>>();
    DefaultContiguousCategoricalEntropyModel::from_floating_point_probabilities_fast(
        &probabilities,
        None,
    )
    .map_err(|_| anyhow::anyhow!("rANS model construction failed"))
}

fn split_folded_context_streams(
    folded_body: &[u8],
    width: usize,
) -> Result<[Vec<u8>; CONTEXT_STREAM_COUNT]> {
    let mut streams = std::array::from_fn(|_| Vec::new());
    let mut decoded = vec![0_u8; folded_body.len()];
    for idx in 0..folded_body.len() {
        let context = folded_context_id(&decoded, width, idx);
        let value = folded_body[idx];
        streams[context].push(value);
        decoded[idx] = value;
    }
    Ok(streams)
}

fn merge_folded_context_streams(
    streams: &[Vec<u8>; CONTEXT_STREAM_COUNT],
    body_len: usize,
    width: usize,
) -> Result<Vec<u8>> {
    let mut cursors = [0_usize; CONTEXT_STREAM_COUNT];
    let mut out = vec![0_u8; body_len];
    for idx in 0..body_len {
        let context = folded_context_id(&out, width, idx);
        let cursor = &mut cursors[context];
        if *cursor >= streams[context].len() {
            bail!("context stream {} underflow during decode", context);
        }
        out[idx] = streams[context][*cursor];
        *cursor += 1;
    }
    for context in 0..CONTEXT_STREAM_COUNT {
        if cursors[context] != streams[context].len() {
            bail!("context stream {} has unused bytes after decode", context);
        }
    }
    Ok(out)
}

fn folded_context_id(decoded_folded: &[u8], width: usize, idx: usize) -> usize {
    let channel = idx % 4;
    let pixel_index = idx / 4;
    let x = pixel_index % width;
    let y = pixel_index / width;

    let left = if x > 0 { decoded_folded[idx - 4] } else { 0 };
    let top = if y > 0 {
        decoded_folded[idx - (width * 4)]
    } else {
        0
    };
    let activity = left.max(top);
    let bin = folded_activity_bin(activity);
    channel * 4 + bin
}

fn folded_activity_bin(activity: u8) -> usize {
    match activity {
        0..=1 => 0,
        2..=7 => 1,
        8..=31 => 2,
        _ => 3,
    }
}

fn read_context_u32_lengths(payload: &[u8]) -> Result<[u32; CONTEXT_SPLIT_HEADER_U32S]> {
    let mut lengths = [0_u32; CONTEXT_SPLIT_HEADER_U32S];
    for (idx, slot) in lengths.iter_mut().enumerate() {
        let start = idx * 4;
        let end = start + 4;
        *slot = u32::from_le_bytes(
            payload[start..end]
                .try_into()
                .expect("fixed 4-byte slice for context-split header"),
        );
    }
    Ok(lengths)
}

fn read_context_rans_u32s(payload: &[u8]) -> Result<[u32; CONTEXT_RANS_HEADER_U32S]> {
    let mut lengths = [0_u32; CONTEXT_RANS_HEADER_U32S];
    for (idx, slot) in lengths.iter_mut().enumerate() {
        let start = idx * 4;
        let end = start + 4;
        *slot = u32::from_le_bytes(
            payload[start..end]
                .try_into()
                .expect("fixed 4-byte slice for context-rANS header"),
        );
    }
    Ok(lengths)
}
