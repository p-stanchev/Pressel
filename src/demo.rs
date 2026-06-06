use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba, RgbaImage};
use std::fs;
use std::path::Path;

pub fn run_make_demo_image(output_png: &Path, seed: u64) -> Result<()> {
    if let Some(parent) = output_png.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let image = build_demo_image(seed);
    image
        .save(output_png)
        .with_context(|| format!("writing demo image {}", output_png.display()))?;
    Ok(())
}

pub fn build_demo_image(seed: u64) -> RgbaImage {
    let width = 192;
    let height = 128;
    let s0 = (seed & 0xff) as u32;
    let s1 = ((seed >> 8) & 0xff) as u32;
    let s2 = ((seed >> 16) & 0xff) as u32;
    let s3 = ((seed >> 24) & 0xff) as u32;
    let stripe_span = 8 + (s0 % 24);
    let alpha_period = 11 + (s1 % 29);
    let alpha_block = 16 + (s2 % 24);
    ImageBuffer::from_fn(width, height, |x, y| {
        let stripe = ((x / stripe_span) + (y / stripe_span)).is_multiple_of(2);
        let alpha = if (x + y + s3).is_multiple_of(alpha_period) {
            0
        } else if (x / alpha_block + y / alpha_block + s0).is_multiple_of(3) {
            128
        } else {
            255
        };

        let r = ((x * (7 + (s0 % 5)) + y * (3 + (s1 % 7)) + s2) % 256) as u8;
        let g = ((x * (5 + (s2 % 9)) + y * (11 + (s3 % 5)) + s0) % 256) as u8;
        let b = if stripe {
            ((x * (13 + (s1 % 11)) + y * (9 + (s2 % 7)) + s3) % 256) as u8
        } else {
            ((255_i32 + (x as i32 * (2 + (s0 % 5) as i32)) - (y as i32 * (3 + (s1 % 5) as i32))
                + s2 as i32)
                .rem_euclid(256)) as u8
        };

        // Keep nontrivial RGB values even when alpha is zero to exercise strict losslessness.
        let rgba = if alpha == 0 {
            [r, g, b, 0]
        } else {
            [r, g, b, alpha as u8]
        };
        Rgba(rgba)
    })
}
