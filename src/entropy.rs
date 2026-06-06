use anyhow::{Result, bail};
use std::io::Cursor;

pub const ENTROPY_BACKEND_COUNT: u8 = 2;

pub fn encode_payload(backend_id: u8, residuals: &[u8]) -> Result<Vec<u8>> {
    match backend_id {
        0 => Ok(residuals.to_vec()),
        1 => Ok(zstd::stream::encode_all(Cursor::new(residuals), 9)?),
        _ => bail!("unsupported entropy backend id: {backend_id}"),
    }
}

pub fn decode_payload(backend_id: u8, payload: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let out = match backend_id {
        0 => payload.to_vec(),
        1 => zstd::stream::decode_all(Cursor::new(payload))?,
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
