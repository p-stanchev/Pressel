use crate::entropy::decode_payload;
use crate::format::{
    CHANNELS_RGBA8, DEFAULT_TILE_SIZE, MAX_RGBA_BYTES, PresselFile, TileHeader, rgba_sha256,
};
use crate::predict::decode_residuals;
use crate::tiles::{TileBounds, write_tile_rgba};
use crate::transform::reverse_transform;
use anyhow::{Context, Result, bail};
use image::ColorType;
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use std::fs;
use std::io::Cursor;
use std::path::Path;

pub fn run_decode(input_prsl: &Path, output_png: &Path) -> Result<()> {
    let bytes =
        fs::read(input_prsl).with_context(|| format!("reading {}", input_prsl.display()))?;
    let decoded = decode_prsl_bytes(&bytes)?;
    let file = fs::File::create(output_png)
        .with_context(|| format!("creating {}", output_png.display()))?;
    let encoder = PngEncoder::new(file);
    encoder.write_image(
        &decoded.rgba,
        decoded.width,
        decoded.height,
        ColorType::Rgba8.into(),
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode_prsl_bytes(bytes: &[u8]) -> Result<DecodedImage> {
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor)?;
    if prsl.header.channels != CHANNELS_RGBA8 {
        bail!("unsupported channel count: {}", prsl.header.channels);
    }
    validate_header(&prsl)?;
    let rgba_len = rgba_byte_len(prsl.header.width, prsl.header.height)?;
    if rgba_len > MAX_RGBA_BYTES {
        bail!("decoded RGBA buffer exceeds limit: {rgba_len} bytes > {MAX_RGBA_BYTES} bytes");
    }
    let mut rgba = vec![0_u8; rgba_len];
    for tile in prsl.tiles {
        let tile_rgba = decode_tile(&tile.header, &tile.payload)?;
        let bounds = TileBounds {
            x: tile.header.x,
            y: tile.header.y,
            width: tile.header.width,
            height: tile.header.height,
        };
        write_tile_rgba(
            &mut rgba,
            prsl.header.width,
            prsl.header.height,
            bounds,
            &tile_rgba,
        )?;
    }

    let actual_hash = rgba_sha256(&rgba);
    if actual_hash != prsl.header.original_pixel_hash {
        bail!("decoded RGBA SHA-256 does not match stored original hash");
    }

    Ok(DecodedImage {
        width: prsl.header.width,
        height: prsl.header.height,
        rgba,
    })
}

fn decode_tile(header: &TileHeader, payload: &[u8]) -> Result<Vec<u8>> {
    let expected_len = tile_rgba_len(header.width, header.height)?;
    let residuals = decode_payload(header.entropy_backend_id, payload, expected_len)?;
    let transformed =
        decode_residuals(&residuals, header.width, header.height, header.predictor_id)?;
    reverse_transform(header.transform_id, &transformed)
}

fn validate_header(prsl: &PresselFile) -> Result<()> {
    let expected_tiles_x = prsl.header.width.div_ceil(prsl.header.tile_size as u32);
    let expected_tiles_y = prsl.header.height.div_ceil(prsl.header.tile_size as u32);
    let expected_tile_count = expected_tiles_x
        .checked_mul(expected_tiles_y)
        .context("tile count overflow")?;
    if prsl.header.tile_count != expected_tile_count {
        bail!(
            "tile count mismatch: header says {}, expected {} for {}x{} with tile size {}",
            prsl.header.tile_count,
            expected_tile_count,
            prsl.header.width,
            prsl.header.height,
            prsl.header.tile_size
        );
    }
    if prsl.header.tile_size != DEFAULT_TILE_SIZE {
        bail!(
            "unsupported tile size {} in v1 decode path",
            prsl.header.tile_size
        );
    }
    Ok(())
}

fn rgba_byte_len(width: u32, height: u32) -> Result<usize> {
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .context("decoded image pixel count overflow")?;
    let rgba_bytes = pixels
        .checked_mul(CHANNELS_RGBA8 as u64)
        .context("decoded image RGBA byte count overflow")?;
    usize::try_from(rgba_bytes).context("decoded image RGBA byte count exceeds platform usize")
}

fn tile_rgba_len(width: u16, height: u16) -> Result<usize> {
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .context("tile pixel count overflow")?;
    let rgba_bytes = pixels
        .checked_mul(CHANNELS_RGBA8 as u64)
        .context("tile RGBA byte count overflow")?;
    usize::try_from(rgba_bytes).context("tile RGBA byte count exceeds platform usize")
}
