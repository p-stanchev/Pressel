use crate::entropy::decode_payload;
use crate::format::{
    CHANNELS_RGBA8, MAX_RGBA_BYTES, PresselFile, TileHeader, rgba_byte_len_u64, rgba_sha256,
};
use crate::predict::{decode_residuals, expected_residual_len};
use crate::tiles::{TileBounds, write_tile_rgba};
use crate::transform::{decode_special_transform, is_special_transform, reverse_transform};
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
    let rgba_len_u64 = rgba_byte_len_u64(prsl.header.width, prsl.header.height)?;
    if rgba_len_u64 > MAX_RGBA_BYTES as u64 {
        bail!("decoded RGBA buffer exceeds limit: {rgba_len_u64} bytes > {MAX_RGBA_BYTES} bytes");
    }
    let rgba_len =
        usize::try_from(rgba_len_u64).context("decoded RGBA byte count exceeds platform usize")?;
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
    if is_special_transform(header.transform_id) {
        let decoded_payload = match header.entropy_backend_id {
            0 => payload.to_vec(),
            1 => zstd::stream::decode_all(Cursor::new(payload))?,
            _ => bail!(
                "unsupported entropy backend {} for special transform {}",
                header.entropy_backend_id,
                header.transform_id
            ),
        };
        return decode_special_transform(
            header.transform_id,
            &decoded_payload,
            header.width,
            header.height,
        );
    }
    let expected_len = expected_residual_len(header.width, header.height, header.predictor_id)?;
    let residuals = decode_payload(header.entropy_backend_id, payload, expected_len)?;
    let transformed =
        decode_residuals(&residuals, header.width, header.height, header.predictor_id)?;
    reverse_transform(header.transform_id, &transformed)
}

fn validate_header(prsl: &PresselFile) -> Result<()> {
    let tile_size = u64::from(prsl.header.tile_size);
    let expected_tiles_x = div_ceil_u64(u64::from(prsl.header.width), tile_size)?;
    let expected_tiles_y = div_ceil_u64(u64::from(prsl.header.height), tile_size)?;
    let expected_tile_count = expected_tiles_x
        .checked_mul(expected_tiles_y)
        .context("tile count overflow")?;
    if u64::from(prsl.header.tile_count) != expected_tile_count {
        bail!(
            "tile count mismatch: header says {}, expected {} for {}x{} with tile size {}",
            prsl.header.tile_count,
            expected_tile_count,
            prsl.header.width,
            prsl.header.height,
            prsl.header.tile_size
        );
    }
    if prsl.header.tile_size == 0 {
        bail!("invalid tile size 0 in decode header");
    }
    Ok(())
}

fn div_ceil_u64(a: u64, b: u64) -> Result<u64> {
    if b == 0 {
        bail!("invalid tile size 0 in decode header");
    }
    a.checked_add(b - 1)
        .context("tile ceil division overflow")?
        .checked_div(b)
        .context("tile ceil division failed")
}
