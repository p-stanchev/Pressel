use crate::entropy::{ENTROPY_BACKEND_COUNT, encode_payload, encode_residual_payload};
use crate::format::{
    CHANNELS_RGBA8, DEFAULT_TILE_SIZE, EncodedTile, PresselFile, PresselHeader, TILE_METADATA_SIZE,
    TaggedSection, TileHeader, rgba_sha256,
};
use crate::png_chunks::{
    PngPreservationOptions, TAG_ORIGINAL_SOURCE_FILE, TAG_PNG_ANCILLARY_CHUNKS,
    TAG_PNG_METADATA_CHUNKS, collect_png_preservation, encode_chunk_records, is_png_file,
};
use crate::predict::{
    PHOTO_GUIDED_PREDICTOR_ID, PREDICTOR_COUNT, encode_residuals, residual_prefix_len,
};
use crate::tiles::{TileBounds, extract_tile_rgba, split_into_tiles};
use crate::transform::{
    TRANSFORM_COUNT, apply_transform, encode_special_transform, is_special_transform,
    transform_ids_for_tile,
};
use anyhow::{Context, Result, bail};
use image::ImageReader;
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::fs;
use std::path::Path;

const FINALIST_COUNT_PER_TILE: usize = 6;
const MAX_BACKENDS_PER_CANDIDATE: usize = 3;
const ZSTD_STREAM_OVERHEAD_ESTIMATE: f64 = 24.0;
const CHANNEL_SPLIT_OVERHEAD_ESTIMATE: f64 = 80.0;
const RANS_OVERHEAD_ESTIMATE: f64 = (256 * 2 + 4) as f64;
const CONTEXT_SPLIT_OVERHEAD_ESTIMATE: f64 = 476.0;

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
    pub preserve_png_metadata: bool,
    pub preserve_png_chunks: bool,
    pub preserve_source_file: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            cores: 1,
            preserve_png_metadata: false,
            preserve_png_chunks: false,
            preserve_source_file: false,
        }
    }
}

pub fn run_encode(input_image: &Path, output_prsl: &Path, options: EncodeOptions) -> Result<()> {
    let source_bytes = fs::read(input_image)
        .with_context(|| format!("reading source file {}", input_image.display()))?;
    let dyn_image = ImageReader::open(input_image)
        .with_context(|| format!("opening image {}", input_image.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", input_image.display()))?;
    let rgba = dyn_image.to_rgba8();
    let tagged_sections = build_tagged_sections(&source_bytes, options)?;
    let encode_options = EncodeOptions {
        cores: options.cores.max(1),
        ..options
    };
    let (encoded, _) = encode_rgba_to_prsl_bytes_with_options(
        rgba.width(),
        rgba.height(),
        rgba.as_raw(),
        encode_options,
        tagged_sections,
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
    encode_rgba_to_prsl_bytes_with_options(
        width,
        height,
        rgba,
        EncodeOptions::default(),
        Vec::new(),
    )
}

pub fn encode_rgba_to_prsl_bytes_with_options(
    width: u32,
    height: u32,
    rgba: &[u8],
    options: EncodeOptions,
    tagged_sections: Vec<TaggedSection>,
) -> Result<(Vec<u8>, EncodeStats)> {
    let mut best: Option<(Vec<u8>, EncodeStats)> = None;
    let pool = build_thread_pool(options.cores.max(1))?;
    for tile_size in candidate_tile_sizes(width, height) {
        let candidate =
            encode_rgba_with_tile_size(&pool, width, height, rgba, tile_size, &tagged_sections)?;
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
    tagged_sections: &[TaggedSection],
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
        sections: tagged_sections.to_vec(),
    };
    let mut bytes = Vec::new();
    prsl.write_to(&mut bytes)?;
    Ok((bytes, stats))
}

fn build_tagged_sections(
    source_bytes: &[u8],
    options: EncodeOptions,
) -> Result<Vec<TaggedSection>> {
    if !options.preserve_png_metadata
        && !options.preserve_png_chunks
        && !options.preserve_source_file
    {
        return Ok(Vec::new());
    }
    if !is_png_file(source_bytes) {
        bail!("PNG preservation flags require a PNG input file");
    }
    let preservation = collect_png_preservation(
        source_bytes,
        PngPreservationOptions {
            preserve_png_metadata: options.preserve_png_metadata,
            preserve_png_chunks: options.preserve_png_chunks,
            preserve_source_file: options.preserve_source_file,
        },
    )?;
    let mut sections = Vec::new();
    if !preservation.ancillary_chunks.is_empty() {
        sections.push(TaggedSection {
            tag_type: TAG_PNG_ANCILLARY_CHUNKS,
            payload: encode_chunk_records(&preservation.ancillary_chunks)?,
        });
    } else if !preservation.metadata_chunks.is_empty() {
        sections.push(TaggedSection {
            tag_type: TAG_PNG_METADATA_CHUNKS,
            payload: encode_chunk_records(&preservation.metadata_chunks)?,
        });
    }
    if let Some(source_file) = preservation.source_file {
        sections.push(TaggedSection {
            tag_type: TAG_ORIGINAL_SOURCE_FILE,
            payload: source_file,
        });
    }
    Ok(sections)
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
    let allow_photo_guided = photo_guided_applicable(tile, tile_rgba);
    let mut residual_candidates = Vec::new();

    for transform_id in transform_ids_for_tile(tile_rgba) {
        if is_special_transform(transform_id) {
            let raw_payload =
                encode_special_transform(transform_id, tile_rgba, tile.width, tile.height)?;
            // Folded residual backends are only valid for predictor residual streams.
            for entropy_backend_id in 0..2 {
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
            if predictor_id == PHOTO_GUIDED_PREDICTOR_ID
                && (!allow_photo_guided || !photo_guided_transform_applicable(transform_id))
            {
                continue;
            }
            let residuals = encode_residuals(&transformed, tile.width, tile.height, predictor_id)?;
            let backend_ids = select_entropy_backend_shortlist(
                &residuals,
                tile.width,
                tile.height,
                predictor_id,
            )?;
            let estimated_total_len = estimate_candidate_total_len(
                &residuals,
                tile.width,
                tile.height,
                predictor_id,
                &backend_ids,
            )?;
            residual_candidates.push(ResidualCandidate {
                transform_id,
                predictor_id,
                residuals,
                backend_ids,
                estimated_total_len,
            });
        }
    }

    residual_candidates.sort_by(|a, b| {
        a.estimated_total_len
            .partial_cmp(&b.estimated_total_len)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for candidate in residual_candidates
        .into_iter()
        .take(FINALIST_COUNT_PER_TILE)
    {
        for entropy_backend_id in candidate.backend_ids {
            let payload = encode_residual_payload(
                entropy_backend_id,
                &candidate.residuals,
                tile.width,
                tile.height,
                candidate.predictor_id,
            )?;
            let total_len = TILE_METADATA_SIZE + payload.len();
            if total_len < best_total_len {
                best_total_len = total_len;
                best = Some(EncodedTile {
                    header: TileHeader {
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        transform_id: candidate.transform_id,
                        predictor_id: candidate.predictor_id,
                        entropy_backend_id,
                        compressed_payload_len: payload.len() as u32,
                    },
                    payload,
                });
            }
        }
    }

    Ok(best.expect("at least one tile strategy must be available"))
}

#[derive(Debug)]
struct ResidualCandidate {
    transform_id: u8,
    predictor_id: u8,
    residuals: Vec<u8>,
    backend_ids: Vec<u8>,
    estimated_total_len: f64,
}

fn select_entropy_backend_shortlist(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<u8>> {
    let estimates = estimate_entropy_payload_lengths(residuals, width, height, predictor_id)?;
    let raw_estimate = estimates
        .iter()
        .find(|(backend_id, _)| *backend_id == 0)
        .map(|(_, estimate)| *estimate)
        .expect("raw backend estimate must exist");

    let mut compressed = estimates
        .into_iter()
        .filter(|(backend_id, _)| *backend_id != 0)
        .collect::<Vec<_>>();
    compressed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut shortlist = vec![0_u8];
    for (backend_id, estimate) in compressed {
        if shortlist.len() >= MAX_BACKENDS_PER_CANDIDATE {
            break;
        }
        if shortlist.len() == 1 || estimate < raw_estimate * 1.10 {
            shortlist.push(backend_id);
        }
    }
    Ok(shortlist)
}

fn estimate_candidate_total_len(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
    backend_ids: &[u8],
) -> Result<f64> {
    let estimates = estimate_entropy_payload_lengths(residuals, width, height, predictor_id)?;
    let best_payload = estimates
        .into_iter()
        .filter(|(backend_id, _)| backend_ids.contains(backend_id))
        .map(|(_, estimate)| estimate)
        .fold(f64::INFINITY, f64::min);
    Ok(best_payload + TILE_METADATA_SIZE as f64)
}

fn estimate_entropy_payload_lengths(
    residuals: &[u8],
    width: u16,
    height: u16,
    predictor_id: u8,
) -> Result<Vec<(u8, f64)>> {
    let prefix_len = residual_prefix_len(width, height, predictor_id)?;
    let body = &residuals[prefix_len..];
    let mut hist = [0_u32; 256];
    let mut folded_hist = [0_u32; 256];
    let mut channel_hist = [[0_u32; 256]; 4];
    let mut folded_channel_hist = [[0_u32; 256]; 4];
    let mut folded_context_hist = [[0_u32; 256]; 16];
    let width_usize = usize::from(width);
    let pixel_count = usize::from(width)
        .checked_mul(usize::from(height))
        .context("entropy estimate pixel count overflow")?;
    let mut folded_body = Vec::with_capacity(body.len());

    for chunk in body.chunks_exact(4) {
        for channel in 0..4 {
            let value = chunk[channel];
            hist[value as usize] += 1;
            channel_hist[channel][value as usize] += 1;
            let folded = fold_residual_for_score(value);
            folded_hist[folded as usize] += 1;
            folded_channel_hist[channel][folded as usize] += 1;
            folded_body.push(folded);
        }
    }

    for idx in 0..folded_body.len() {
        let context = folded_context_id_for_score(&folded_body, width_usize, pixel_count, idx);
        folded_context_hist[context][folded_body[idx] as usize] += 1;
    }

    let total_symbols = body.len();
    let interleaved_bits = estimated_histogram_bits(&hist, total_symbols);
    let folded_bits = estimated_histogram_bits(&folded_hist, total_symbols);
    let channel_bits = channel_hist
        .iter()
        .map(|channel| estimated_histogram_bits(channel, total_symbols / 4))
        .sum::<f64>();
    let folded_channel_bits = folded_channel_hist
        .iter()
        .map(|channel| estimated_histogram_bits(channel, total_symbols / 4))
        .sum::<f64>();
    let context_bits = folded_context_hist
        .iter()
        .map(|context_hist| {
            let context_total = context_hist.iter().map(|&count| count as usize).sum::<usize>();
            estimated_histogram_bits(context_hist, context_total)
        })
        .sum::<f64>();

    Ok(vec![
        (0, residuals.len() as f64),
        (
            1,
            prefix_len as f64 + (interleaved_bits / 8.0) + ZSTD_STREAM_OVERHEAD_ESTIMATE,
        ),
        (2, residuals.len() as f64),
        (
            3,
            prefix_len as f64 + (folded_bits / 8.0) + ZSTD_STREAM_OVERHEAD_ESTIMATE,
        ),
        (
            4,
            prefix_len as f64 + (channel_bits / 8.0) + CHANNEL_SPLIT_OVERHEAD_ESTIMATE,
        ),
        (
            5,
            prefix_len as f64 + (folded_channel_bits / 8.0) + CHANNEL_SPLIT_OVERHEAD_ESTIMATE,
        ),
        (
            6,
            (folded_bits / 8.0) + RANS_OVERHEAD_ESTIMATE,
        ),
        (
            7,
            prefix_len as f64 + (context_bits / 8.0) + CONTEXT_SPLIT_OVERHEAD_ESTIMATE,
        ),
    ])
}

fn estimated_histogram_bits(hist: &[u32; 256], total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let total_f = total as f64;
    hist.iter()
        .copied()
        .filter(|&count| count > 0)
        .map(|count| {
            let count_f = count as f64;
            count_f * (total_f / count_f).log2()
        })
        .sum()
}

fn fold_residual_for_score(residual: u8) -> u8 {
    if residual <= 127 {
        residual.wrapping_mul(2)
    } else {
        (255 - residual).wrapping_mul(2).wrapping_add(1)
    }
}

fn folded_context_id_for_score(
    folded_body: &[u8],
    width: usize,
    pixel_count: usize,
    idx: usize,
) -> usize {
    let channel = idx % 4;
    let pixel_index = idx / 4;
    let x = pixel_index % width;
    let y = pixel_index / width;
    let left = if x > 0 { folded_body[idx - 4] } else { 0 };
    let top = if y > 0 && pixel_count > width {
        folded_body[idx - (width * 4)]
    } else {
        0
    };
    let activity = left.max(top);
    let bin = match activity {
        0..=1 => 0,
        2..=7 => 1,
        8..=31 => 2,
        _ => 3,
    };
    channel * 4 + bin
}

fn photo_guided_applicable(tile: TileBounds, tile_rgba: &[u8]) -> bool {
    let pixel_count = usize::from(tile.width) * usize::from(tile.height);
    if pixel_count < 256 {
        return false;
    }
    tile_rgba.chunks_exact(4).all(|px| px[3] == 255)
}

fn photo_guided_transform_applicable(transform_id: u8) -> bool {
    matches!(transform_id, 0 | 1 | 2 | 4)
}
