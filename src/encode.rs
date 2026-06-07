use crate::entropy::{ENTROPY_BACKEND_COUNT, encode_payload};
use crate::format::{
    CHANNELS_RGBA8, DEFAULT_TILE_SIZE, EncodedTile, PresselFile, PresselHeader, TILE_METADATA_SIZE,
    TileHeader, rgba_sha256,
};
use crate::predict::{PREDICTOR_COUNT, encode_residuals};
use crate::tiles::{TileBounds, extract_tile_rgba, split_into_tiles};
use crate::transform::{
    TRANSFORM_COUNT, apply_transform, encode_special_transform, is_special_transform,
    transform_ids_for_tile,
};
use anyhow::{Context, Result};
use image::ImageReader;
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub width: u32,
    pub height: u32,
    pub tile_size: u16,
    pub tile_count: usize,
    pub transform_counts: [u64; TRANSFORM_COUNT as usize],
    pub predictor_counts: [u64; PREDICTOR_COUNT as usize],
    pub entropy_counts: [u64; ENTROPY_BACKEND_COUNT as usize],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeOptions {
    pub cores: usize,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self { cores: 1 }
    }
}

pub fn run_encode(input_image: &Path, output_prsl: &Path, cores: usize) -> Result<()> {
    let dyn_image = ImageReader::open(input_image)
        .with_context(|| format!("opening image {}", input_image.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", input_image.display()))?;
    let rgba = dyn_image.to_rgba8();
    let options = EncodeOptions {
        cores: cores.max(1),
    };
    let (encoded, _) = encode_rgba_to_prsl_bytes_with_options(
        rgba.width(),
        rgba.height(),
        rgba.as_raw(),
        options,
    )?;
    fs::write(output_prsl, encoded)
        .with_context(|| format!("writing {}", output_prsl.display()))?;
    Ok(())
}

#[allow(dead_code)]
pub fn encode_rgba_to_prsl_bytes(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(Vec<u8>, EncodeStats)> {
    encode_rgba_to_prsl_bytes_with_options(width, height, rgba, EncodeOptions::default())
}

pub fn encode_rgba_to_prsl_bytes_with_options(
    width: u32,
    height: u32,
    rgba: &[u8],
    options: EncodeOptions,
) -> Result<(Vec<u8>, EncodeStats)> {
    let mut best: Option<(Vec<u8>, EncodeStats)> = None;
    let pool = build_thread_pool(options.cores.max(1))?;
    for tile_size in candidate_tile_sizes(width, height) {
        let candidate = encode_rgba_with_tile_size(&pool, width, height, rgba, tile_size)?;
        if best
            .as_ref()
            .is_none_or(|(best_bytes, _)| candidate.0.len() < best_bytes.len())
        {
            best = Some(candidate);
        }
    }
    Ok(best.expect("at least one tile-size candidate must exist"))
}

fn encode_rgba_with_tile_size(
    pool: &ThreadPool,
    width: u32,
    height: u32,
    rgba: &[u8],
    tile_size: u16,
) -> Result<(Vec<u8>, EncodeStats)> {
    let tiles = split_into_tiles(width, height, tile_size);
    let encoded_tiles = pool.install(|| {
        tiles
            .par_iter()
            .map(|&tile| {
                let tile_bytes = extract_tile_rgba(rgba, width, height, tile)?;
                encode_tile(tile, &tile_bytes)
            })
            .collect::<Result<Vec<_>>>()
    })?;

    let mut stats = EncodeStats {
        width,
        height,
        tile_size,
        tile_count: encoded_tiles.len(),
        ..EncodeStats::default()
    };
    for encoded_tile in &encoded_tiles {
        stats.transform_counts[encoded_tile.header.transform_id as usize] += 1;
        stats.predictor_counts[encoded_tile.header.predictor_id as usize] += 1;
        stats.entropy_counts[encoded_tile.header.entropy_backend_id as usize] += 1;
    }

    let header = PresselHeader {
        width,
        height,
        channels: CHANNELS_RGBA8,
        tile_size,
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

fn build_thread_pool(cores: usize) -> Result<ThreadPool> {
    ThreadPoolBuilder::new()
        .num_threads(cores)
        .build()
        .context("building rayon thread pool")
}

fn candidate_tile_sizes(width: u32, height: u32) -> Vec<u16> {
    let max_dim = width.max(height).min(u16::MAX as u32) as u16;
    let mut candidates = vec![DEFAULT_TILE_SIZE];
    for candidate in [128_u16, 256_u16, 512_u16, 1024_u16, max_dim] {
        if candidate >= DEFAULT_TILE_SIZE && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates.sort_unstable();
    candidates
}

fn encode_tile(tile: TileBounds, tile_rgba: &[u8]) -> Result<EncodedTile> {
    let mut best: Option<EncodedTile> = None;
    let mut best_total_len = usize::MAX;

    for transform_id in transform_ids_for_tile(tile_rgba) {
        if is_special_transform(transform_id) {
            let raw_payload =
                encode_special_transform(transform_id, tile_rgba, tile.width, tile.height)?;
            for entropy_backend_id in 0..ENTROPY_BACKEND_COUNT {
                let payload = encode_payload(entropy_backend_id, &raw_payload)?;
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
                            predictor_id: 0,
                            entropy_backend_id,
                            compressed_payload_len: payload.len() as u32,
                        },
                        payload,
                    });
                }
            }
            continue;
        }
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
