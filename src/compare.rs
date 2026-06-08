use crate::format::sha256_hex;
use anyhow::{Context, Result};
use image::ImageReader;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareImageInfo {
    pub path: String,
    pub file_size: u64,
    pub file_sha256: String,
    pub width: u32,
    pub height: u32,
    pub decoded_rgba_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareResult {
    pub first: CompareImageInfo,
    pub second: CompareImageInfo,
    pub exact_file_match: bool,
    pub exact_rgba_match: bool,
}

pub fn run_compare(first_image: &Path, second_image: &Path) -> Result<CompareResult> {
    let first = inspect_image(first_image)?;
    let second = inspect_image(second_image)?;

    let exact_file_match = first.file_sha256 == second.file_sha256;
    let exact_rgba_match = first.decoded_rgba_sha256 == second.decoded_rgba_sha256;

    println!("first path: {}", first.path);
    println!("first file size: {}", first.file_size);
    println!("first file SHA-256: {}", first.file_sha256);
    println!("first dimensions: {}x{}", first.width, first.height);
    println!("first decoded RGBA SHA-256: {}", first.decoded_rgba_sha256);
    println!();
    println!("second path: {}", second.path);
    println!("second file size: {}", second.file_size);
    println!("second file SHA-256: {}", second.file_sha256);
    println!("second dimensions: {}x{}", second.width, second.height);
    println!(
        "second decoded RGBA SHA-256: {}",
        second.decoded_rgba_sha256
    );
    println!();
    println!("exact file match: {exact_file_match}");
    println!("exact decoded RGBA match: {exact_rgba_match}");

    Ok(CompareResult {
        first,
        second,
        exact_file_match,
        exact_rgba_match,
    })
}

fn inspect_image(path: &Path) -> Result<CompareImageInfo> {
    let file_bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let file_size = file_bytes.len() as u64;
    let file_sha256 = sha256_hex(&file_bytes);

    let rgba = ImageReader::open(path)
        .with_context(|| format!("opening image {}", path.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", path.display()))?
        .to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let decoded_rgba_sha256 = sha256_hex(rgba.as_raw());

    Ok(CompareImageInfo {
        path: path.display().to_string(),
        file_size,
        file_sha256,
        width,
        height,
        decoded_rgba_sha256,
    })
}
