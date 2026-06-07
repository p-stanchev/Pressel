use crate::decode::decode_prsl_bytes;
use crate::encode::{EncodeOptions, EncodeStats, encode_rgba_to_prsl_bytes_with_options};
use anyhow::{Context, Result};
use csv::Writer;
use image::ImageReader;
use std::fs;
use std::path::Path;
use std::time::Instant;
use walkdir::WalkDir;

pub fn run_bench(folder: &Path, cores: usize) -> Result<()> {
    let mut writer = Writer::from_path("bench.csv").context("creating bench.csv")?;
    writer.write_record([
        "filename",
        "width",
        "height",
        "original_file_size",
        "prsl_size",
        "compression_ratio",
        "encode_time_ms",
        "decode_time_ms",
        "selected_transform_counts",
        "selected_predictor_counts",
        "selected_entropy_backend_counts",
        "verification_result",
    ])?;

    let options = EncodeOptions {
        cores: cores.max(1),
        preserve_png_metadata: false,
        preserve_png_chunks: false,
        preserve_source_file: false,
    };

    for entry in WalkDir::new(folder)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(reader) = ImageReader::open(path) else {
            continue;
        };
        let Ok(decoded) = reader.decode() else {
            continue;
        };
        let rgba = decoded.to_rgba8();
        let original_file_size = fs::metadata(path)?.len();

        let encode_start = Instant::now();
        let (prsl_bytes, stats) = encode_rgba_to_prsl_bytes_with_options(
            rgba.width(),
            rgba.height(),
            rgba.as_raw(),
            options,
            Vec::new(),
        )
        .with_context(|| format!("encoding {}", path.display()))?;
        let encode_time_ms = encode_start.elapsed().as_secs_f64() * 1000.0;

        let decode_start = Instant::now();
        let decoded_prsl = decode_prsl_bytes(&prsl_bytes)
            .with_context(|| format!("decoding {}", path.display()))?;
        let decode_time_ms = decode_start.elapsed().as_secs_f64() * 1000.0;

        let verified = decoded_prsl.rgba == *rgba.as_raw();
        let prsl_size = prsl_bytes.len() as u64;
        let compression_ratio = if original_file_size == 0 {
            0.0
        } else {
            prsl_size as f64 / original_file_size as f64
        };

        writer.write_record([
            path.display().to_string(),
            rgba.width().to_string(),
            rgba.height().to_string(),
            original_file_size.to_string(),
            prsl_size.to_string(),
            format!("{compression_ratio:.6}"),
            format!("{encode_time_ms:.3}"),
            format!("{decode_time_ms:.3}"),
            counts_to_string(&stats.transform_counts),
            counts_to_string(&stats.predictor_counts),
            counts_to_string(&stats.entropy_counts),
            verified.to_string(),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn counts_to_string(counts: &[u64]) -> String {
    counts
        .iter()
        .enumerate()
        .map(|(idx, count)| format!("{idx}:{count}"))
        .collect::<Vec<_>>()
        .join("|")
}

#[allow(dead_code)]
fn _stats_identity(stats: &EncodeStats) -> (&[u64], &[u64], &[u64]) {
    (
        &stats.transform_counts,
        &stats.predictor_counts,
        &stats.entropy_counts,
    )
}
