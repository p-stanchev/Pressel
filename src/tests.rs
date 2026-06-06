use crate::decode::decode_prsl_bytes;
use crate::encode::encode_rgba_to_prsl_bytes;
use crate::entropy::{decode_payload, encode_payload};
use crate::format::{CHANNELS_RGBA8, DEFAULT_TILE_SIZE, MAGIC_BYTES};
use crate::predict::{PREDICTOR_COUNT, decode_residuals, encode_residuals};
use crate::transform::{TRANSFORM_COUNT, apply_transform, reverse_transform};
use crate::verify::run_verify;
use image::{ImageBuffer, Rgba, RgbaImage};
use std::fs;
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
    let rgba = gradient(9, 7);
    for transform_id in 0..TRANSFORM_COUNT {
        let transformed = apply_transform(transform_id, &rgba).unwrap();
        let reversed = reverse_transform(transform_id, &transformed).unwrap();
        assert_eq!(reversed, rgba);
    }
}

#[test]
fn every_predictor_roundtrip() {
    let rgba = gradient(11, 5);
    for predictor_id in 0..PREDICTOR_COUNT {
        let residuals = encode_residuals(&rgba, 11, 5, predictor_id).unwrap();
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
