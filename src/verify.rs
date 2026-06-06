use crate::decode::decode_prsl_bytes;
use crate::format::sha256_hex;
use anyhow::{Context, Result};
use image::ImageReader;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub original_sha256: String,
    pub decoded_sha256: String,
    pub exact_match: bool,
}

pub fn run_verify(input_image: &Path, input_prsl: &Path) -> Result<VerifyResult> {
    let original = ImageReader::open(input_image)
        .with_context(|| format!("opening image {}", input_image.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", input_image.display()))?
        .to_rgba8();
    let prsl_bytes =
        fs::read(input_prsl).with_context(|| format!("reading {}", input_prsl.display()))?;
    let decoded = decode_prsl_bytes(&prsl_bytes)?;

    let original_sha256 = sha256_hex(original.as_raw());
    let decoded_sha256 = sha256_hex(&decoded.rgba);
    let exact_match = original.as_raw() == &decoded.rgba;

    println!("original SHA-256: {original_sha256}");
    println!("decoded SHA-256: {decoded_sha256}");
    println!("exact match: {exact_match}");

    Ok(VerifyResult {
        original_sha256,
        decoded_sha256,
        exact_match,
    })
}
