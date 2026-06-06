use crate::entropy::{ENTROPY_BACKEND_COUNT, encode_payload};
use crate::format::{
    CHANNELS_RGBA8, DEFAULT_TILE_SIZE, EncodedTile, PresselFile, PresselHeader, TILE_METADATA_SIZE,
    TileHeader, rgba_sha256,
};
use crate::predict::{PREDICTOR_COUNT, encode_residuals};
use crate::tiles::{TileBounds, extract_tile_rgba, split_into_tiles};
use crate::transform::{TRANSFORM_COUNT, apply_transform, transform_ids_for_tile};
use anyhow::{Context, Result};
use image::ImageReader;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub width: u32,
    pub height: u32,
    pub tile_count: usize,
    pub transform_counts: [u64; TRANSFORM_COUNT as usize],
    pub predictor_counts: [u64; PREDICTOR_COUNT as usize],
    pub entropy_counts: [u64; ENTROPY_BACKEND_COUNT as usize],
}

pub fn run_encode(input_image: &Path, output_prsl: &Path) -> Result<()> {
    let dyn_image = ImageReader::open(input_image)
        .with_context(|| format!("opening image {}", input_image.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", input_image.display()))?;
    let rgba = dyn_image.to_rgba8();
    let (encoded, _) = encode_rgba_to_prsl_bytes(rgba.width(), rgba.height(), rgba.as_raw())?;
    fs::write(output_prsl, encoded)
        .with_context(|| format!("writing {}", output_prsl.display()))?;
    Ok(())
}

pub fn encode_rgba_to_prsl_bytes(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(Vec<u8>, EncodeStats)> {
    let tiles = split_into_tiles(width, height, DEFAULT_TILE_SIZE);
    let mut encoded_tiles = Vec::with_capacity(tiles.len());
    let mut stats = EncodeStats {
        width,
        height,
        tile_count: tiles.len(),
        ..EncodeStats::default()
    };

    for tile in tiles {
        let tile_bytes = extract_tile_rgba(rgba, width, height, tile)?;
        let encoded_tile = encode_tile(tile, &tile_bytes)?;
        stats.transform_counts[encoded_tile.header.transform_id as usize] += 1;
        stats.predictor_counts[encoded_tile.header.predictor_id as usize] += 1;
        stats.entropy_counts[encoded_tile.header.entropy_backend_id as usize] += 1;
        encoded_tiles.push(encoded_tile);
    }

    let header = PresselHeader {
        width,
        height,
        channels: CHANNELS_RGBA8,
        tile_size: DEFAULT_TILE_SIZE,
        tile_count: encoded_tiles.len() as u32,
        original_pixel_hash: rgba_sha256(rgba),
    };
    let prsl = PresselFile {
        header,
        tiles: encoded_tiles,
    };
    let mut bytes = Vec::new();
    prsl.write_to(&mut bytes)?;
    Ok((bytes, stats))
}

fn encode_tile(tile: TileBounds, tile_rgba: &[u8]) -> Result<EncodedTile> {
    let mut best: Option<EncodedTile> = None;
    let mut best_total_len = usize::MAX;

    for transform_id in transform_ids_for_tile(tile_rgba) {
        let transformed = apply_transform(transform_id, tile_rgba)?;
        for predictor_id in 0..PREDICTOR_COUNT {
            let residuals = encode_residuals(&transformed, tile.width, tile.height, predictor_id)?;
            for entropy_backend_id in 0..ENTROPY_BACKEND_COUNT {
                let payload = encode_payload(entropy_backend_id, &residuals)?;
                let total_len = TILE_METADATA_SIZE + payload.len();
                if total_len < best_total_len {
                    best_total_len = total_len;
                    best = Some(EncodedTile {
                        header: TileHeader {
                            x: tile.x,
                            y: tile.y,
                            width: tile.width,
                            height: tile.height,
                            transform_id,
                            predictor_id,
                            entropy_backend_id,
                            compressed_payload_len: payload.len() as u32,
                        },
                        payload,
                    });
                }
            }
        }
    }

    Ok(best.expect("at least one tile strategy must be available"))
}
