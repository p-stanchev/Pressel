use crate::decode::decode_prsl_bytes;
use crate::decode::run_decode;
use crate::encode::{EncodeOptions, encode_rgba_to_prsl_bytes, run_encode};
use crate::entropy::{
    decode_payload, decode_residual_payload, encode_payload, encode_residual_payload,
};
use crate::format::{CHANNELS_RGBA8, DEFAULT_TILE_SIZE, MAGIC_BYTES, PresselFile};
use crate::png_chunks::{
    PLACEMENT_BEFORE_IDAT, TAG_ORIGINAL_SOURCE_FILE, TAG_PNG_ANCILLARY_CHUNKS,
    TAG_PNG_METADATA_CHUNKS, chunk_record, decode_chunk_records, make_test_png_with_chunks,
};
use crate::predict::{PREDICTOR_COUNT, decode_residuals, encode_residuals, expected_residual_len};
use crate::transform::{
    TRANSFORM_COUNT, apply_transform, decode_special_transform, encode_special_transform,
    is_special_transform, reverse_transform, transform_ids_for_tile,
};
use crate::verify::run_verify;
use image::{ImageBuffer, Rgba, RgbaImage};
use std::fs;
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

fn roundtrip_rgba(width: u32, height: u32, rgba: Vec<u8>) {
    let (bytes, _) = encode_rgba_to_prsl_bytes(width, height, &rgba).unwrap();
    let decoded = decode_prsl_bytes(&bytes).unwrap();
    assert_eq!(decoded.width, width);
    assert_eq!(decoded.height, height);
    assert_eq!(decoded.rgba, rgba);
}

fn checkerboard(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let on = (x + y) % 2 == 0;
            let px = if on {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            };
            out.extend_from_slice(&px);
        }
    }
    out
}

fn gradient(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            out.push((x * 13 + y * 7) as u8);
            out.push((x * 5 + y * 11) as u8);
            out.push((x * 3 + y * 17) as u8);
            out.push(255);
        }
    }
    out
}

fn noise(width: u32, height: u32) -> Vec<u8> {
    let mut state = 0x1234_5678_u32;
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..(width * height * 4) {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        out.push((state >> 24) as u8);
    }
    out
}

#[test]
fn one_by_one_image() {
    roundtrip_rgba(1, 1, vec![17, 23, 29, 31]);
}

#[test]
fn flat_color_image() {
    roundtrip_rgba(32, 16, [42, 42, 42, 255].repeat(32 * 16));
}

#[test]
fn transparent_hidden_rgb_preserved() {
    roundtrip_rgba(
        2,
        2,
        vec![255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 7, 9, 11, 0],
    );
}

#[test]
fn gradient_image() {
    roundtrip_rgba(31, 19, gradient(31, 19));
}

#[test]
fn random_noise_image() {
    roundtrip_rgba(37, 23, noise(37, 23));
}

#[test]
fn small_checkerboard() {
    roundtrip_rgba(8, 8, checkerboard(8, 8));
}

#[test]
fn non_divisible_tile_dimensions() {
    roundtrip_rgba(130, 70, gradient(130, 70));
}

#[test]
fn every_transform_roundtrip() {
    for transform_id in 0..TRANSFORM_COUNT {
        let rgba = if transform_id == 5 {
            checkerboard(9, 7)
        } else {
            gradient(9, 7)
        };
        if !transform_ids_for_tile(&rgba).contains(&transform_id) {
            continue;
        }
        let reversed = if is_special_transform(transform_id) {
            let payload = encode_special_transform(transform_id, &rgba, 9, 7).unwrap();
            decode_special_transform(transform_id, &payload, 9, 7).unwrap()
        } else {
            let transformed = apply_transform(transform_id, &rgba).unwrap();
            reverse_transform(transform_id, &transformed).unwrap()
        };
        assert_eq!(reversed, rgba);
    }
}

#[test]
fn every_predictor_roundtrip() {
    let rgba = gradient(11, 5);
    for predictor_id in 0..PREDICTOR_COUNT {
        let residuals = encode_residuals(&rgba, 11, 5, predictor_id).unwrap();
        let expected_len = expected_residual_len(11, 5, predictor_id).unwrap();
        assert_eq!(residuals.len(), expected_len);
        let decoded = decode_residuals(&residuals, 11, 5, predictor_id).unwrap();
        assert_eq!(decoded, rgba);
    }
}

#[test]
fn raw_entropy_roundtrip() {
    let bytes = noise(7, 3);
    let encoded = encode_payload(0, &bytes).unwrap();
    let decoded = decode_payload(0, &encoded, bytes.len()).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn zstd_entropy_roundtrip() {
    let bytes = gradient(17, 9);
    let encoded = encode_payload(1, &bytes).unwrap();
    let decoded = decode_payload(1, &encoded, bytes.len()).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn folded_raw_entropy_roundtrip() {
    let bytes = gradient(17, 9);
    let encoded = encode_payload(2, &bytes).unwrap();
    let decoded = decode_payload(2, &encoded, bytes.len()).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn folded_zstd_entropy_roundtrip() {
    let bytes = gradient(17, 9);
    let encoded = encode_payload(3, &bytes).unwrap();
    let decoded = decode_payload(3, &encoded, bytes.len()).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn channel_split_entropy_roundtrip() {
    let bytes = gradient(17, 9);
    let encoded = encode_residual_payload(4, &bytes, 17, 9, 0).unwrap();
    let decoded = decode_residual_payload(4, &encoded, 17, 9, 0).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn folded_channel_split_entropy_roundtrip() {
    let bytes = gradient(17, 9);
    let encoded = encode_residual_payload(5, &bytes, 17, 9, 0).unwrap();
    let decoded = decode_residual_payload(5, &encoded, 17, 9, 0).unwrap();
    assert_eq!(decoded, bytes);
}

#[test]
fn photo_guided_channel_split_entropy_roundtrip() {
    let transformed = gradient(17, 9);
    let residuals = encode_residuals(&transformed, 17, 9, 8).unwrap();
    let encoded = encode_residual_payload(5, &residuals, 17, 9, 8).unwrap();
    let decoded = decode_residual_payload(5, &encoded, 17, 9, 8).unwrap();
    assert_eq!(decoded, residuals);
    let reconstructed = decode_residuals(&decoded, 17, 9, 8).unwrap();
    assert_eq!(reconstructed, transformed);
}

#[test]
fn adaptive_channel_split_entropy_roundtrip() {
    let transformed = gradient(19, 13);
    let residuals = encode_residuals(&transformed, 19, 13, 6).unwrap();
    let encoded = encode_residual_payload(4, &residuals, 19, 13, 6).unwrap();
    let decoded = decode_residual_payload(4, &encoded, 19, 13, 6).unwrap();
    assert_eq!(decoded, residuals);
    let reconstructed = decode_residuals(&decoded, 19, 13, 6).unwrap();
    assert_eq!(reconstructed, transformed);
}

#[test]
fn adaptive_folded_channel_split_entropy_roundtrip() {
    let transformed = gradient(19, 13);
    let residuals = encode_residuals(&transformed, 19, 13, 6).unwrap();
    let encoded = encode_residual_payload(5, &residuals, 19, 13, 6).unwrap();
    let decoded = decode_residual_payload(5, &encoded, 19, 13, 6).unwrap();
    assert_eq!(decoded, residuals);
    let reconstructed = decode_residuals(&decoded, 19, 13, 6).unwrap();
    assert_eq!(reconstructed, transformed);
}

#[test]
fn full_encode_decode_verify() {
    let rgba = gradient(23, 17);
    let image: RgbaImage = ImageBuffer::from_fn(23, 17, |x, y| {
        let idx = ((y * 23 + x) * 4) as usize;
        Rgba([rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]])
    });

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("pressel-test-{stamp}"));
    fs::create_dir_all(&base).unwrap();
    let png_path = base.join("input.png");
    let prsl_path = base.join("out.prsl");
    image.save(&png_path).unwrap();

    let (prsl_bytes, _) = encode_rgba_to_prsl_bytes(23, 17, &rgba).unwrap();
    fs::write(&prsl_path, prsl_bytes).unwrap();

    let result = run_verify(&png_path, &prsl_path).unwrap();
    assert!(result.exact_match);

    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
    let _ = fs::remove_dir(&base);
}

#[test]
fn decode_rejects_invalid_tile_count() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC_BYTES);
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(CHANNELS_RGBA8);
    bytes.extend_from_slice(&DEFAULT_TILE_SIZE.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&[0_u8; 32]);

    let err = decode_prsl_bytes(&bytes).unwrap_err();
    assert!(err.to_string().contains("tile count mismatch"));
}

#[test]
fn default_encode_stores_no_png_preservation_sections() {
    let (png_path, _base) = create_png_with_test_chunks();
    let prsl_path = png_path.with_extension("prsl");
    run_encode(
        &png_path,
        &prsl_path,
        EncodeOptions {
            cores: 1,
            preserve_png_metadata: false,
            preserve_png_chunks: false,
            preserve_source_file: false,
        },
    )
    .unwrap();
    let bytes = fs::read(&prsl_path).unwrap();
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor).unwrap();
    assert!(prsl.sections.is_empty());
    let decoded = decode_prsl_bytes(&fs::read(&prsl_path).unwrap()).unwrap();
    assert_eq!(decoded.png_metadata_chunks, Vec::new());
    assert_eq!(decoded.png_ancillary_chunks, Vec::new());
    assert!(decoded.original_source_file.is_none());
    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
}

#[test]
fn png_metadata_preservation_tag_is_stored() {
    let (png_path, _base) = create_png_with_test_chunks();
    let prsl_path = png_path.with_extension("prsl");
    run_encode(
        &png_path,
        &prsl_path,
        EncodeOptions {
            cores: 1,
            preserve_png_metadata: true,
            preserve_png_chunks: false,
            preserve_source_file: false,
        },
    )
    .unwrap();
    let bytes = fs::read(&prsl_path).unwrap();
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor).unwrap();
    let section = prsl
        .sections
        .iter()
        .find(|section| section.tag_type == TAG_PNG_METADATA_CHUNKS)
        .expect("metadata tag should be present");
    let records = decode_chunk_records(&section.payload).unwrap();
    assert!(records.iter().any(|record| record.chunk_type == *b"gAMA"));
    assert!(records.iter().any(|record| record.chunk_type == *b"tEXt"));
    assert!(
        prsl.sections
            .iter()
            .all(|section| section.tag_type != TAG_PNG_ANCILLARY_CHUNKS)
    );
    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
}

#[test]
fn png_chunk_preservation_tag_is_stored() {
    let (png_path, _base) = create_png_with_test_chunks();
    let prsl_path = png_path.with_extension("prsl");
    run_encode(
        &png_path,
        &prsl_path,
        EncodeOptions {
            cores: 1,
            preserve_png_metadata: false,
            preserve_png_chunks: true,
            preserve_source_file: false,
        },
    )
    .unwrap();
    let bytes = fs::read(&prsl_path).unwrap();
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor).unwrap();
    let section = prsl
        .sections
        .iter()
        .find(|section| section.tag_type == TAG_PNG_ANCILLARY_CHUNKS)
        .expect("ancillary chunk tag should be present");
    let records = decode_chunk_records(&section.payload).unwrap();
    assert!(records.iter().any(|record| record.chunk_type == *b"gAMA"));
    assert!(records.iter().any(|record| record.chunk_type == *b"raNd"));
    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
}

#[test]
fn png_chunk_mode_subsumes_metadata_mode_without_duplicate_section() {
    let (png_path, _base) = create_png_with_test_chunks();
    let prsl_path = png_path.with_extension("prsl");
    run_encode(
        &png_path,
        &prsl_path,
        EncodeOptions {
            cores: 1,
            preserve_png_metadata: true,
            preserve_png_chunks: true,
            preserve_source_file: false,
        },
    )
    .unwrap();
    let bytes = fs::read(&prsl_path).unwrap();
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor).unwrap();
    assert!(
        prsl.sections
            .iter()
            .any(|section| section.tag_type == TAG_PNG_ANCILLARY_CHUNKS)
    );
    assert!(
        prsl.sections
            .iter()
            .all(|section| section.tag_type != TAG_PNG_METADATA_CHUNKS)
    );
    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
}

#[test]
fn preserve_source_file_extracts_exact_original_png() {
    let (png_path, original_png_bytes) = create_png_with_test_chunks();
    let prsl_path = png_path.with_extension("prsl");
    let extracted_path = png_path.with_extension("extracted.png");
    run_encode(
        &png_path,
        &prsl_path,
        EncodeOptions {
            cores: 1,
            preserve_png_metadata: false,
            preserve_png_chunks: false,
            preserve_source_file: true,
        },
    )
    .unwrap();
    let bytes = fs::read(&prsl_path).unwrap();
    let mut cursor = Cursor::new(bytes);
    let prsl = PresselFile::read_from(&mut cursor).unwrap();
    assert!(
        prsl.sections
            .iter()
            .any(|section| section.tag_type == TAG_ORIGINAL_SOURCE_FILE)
    );
    run_decode(&prsl_path, None, None, Some(&extracted_path)).unwrap();
    let extracted = fs::read(&extracted_path).unwrap();
    assert_eq!(extracted, original_png_bytes);
    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&prsl_path);
    let _ = fs::remove_file(&extracted_path);
}

fn create_png_with_test_chunks() -> (std::path::PathBuf, Vec<u8>) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pressel-meta-{stamp}.png"));
    let image: RgbaImage = ImageBuffer::from_fn(4, 3, |x, y| {
        Rgba([
            (x * 31 + y * 17) as u8,
            (x * 13 + y * 29) as u8,
            (x * 7 + y * 19) as u8,
            255,
        ])
    });
    image.save(&path).unwrap();
    let base = fs::read(&path).unwrap();
    let enriched = make_test_png_with_chunks(
        &base,
        &[
            chunk_record(
                *b"gAMA",
                PLACEMENT_BEFORE_IDAT,
                vec![0, 0, 0xB1, 0x8F],
                Some(false),
            ),
            chunk_record(
                *b"tEXt",
                PLACEMENT_BEFORE_IDAT,
                b"Author\0Pressel".to_vec(),
                Some(false),
            ),
            chunk_record(
                *b"raNd",
                PLACEMENT_BEFORE_IDAT,
                b"side-data".to_vec(),
                Some(true),
            ),
        ],
    )
    .unwrap();
    fs::write(&path, &enriched).unwrap();
    (path, enriched)
}
