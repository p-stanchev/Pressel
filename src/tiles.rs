use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileBounds {
    pub x: u32,
    pub y: u32,
    pub width: u16,
    pub height: u16,
}

impl TileBounds {
    pub fn pixel_count(self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn byte_len(self) -> usize {
        self.pixel_count() * 4
    }
}

pub fn split_into_tiles(width: u32, height: u32, tile_size: u16) -> Vec<TileBounds> {
    let tile_size = tile_size as u32;
    let mut tiles = Vec::new();
    let mut y = 0_u32;
    while y < height {
        let mut x = 0_u32;
        let tile_h = (height - y).min(tile_size) as u16;
        while x < width {
            let tile_w = (width - x).min(tile_size) as u16;
            tiles.push(TileBounds {
                x,
                y,
                width: tile_w,
                height: tile_h,
            });
            x += tile_size;
        }
        y += tile_size;
    }
    tiles
}

pub fn extract_tile_rgba(
    rgba: &[u8],
    image_width: u32,
    image_height: u32,
    tile: TileBounds,
) -> Result<Vec<u8>> {
    validate_rgba_len(rgba, image_width, image_height)?;
    validate_tile_bounds(tile, image_width, image_height)?;
    let mut out = Vec::with_capacity(tile.byte_len());
    for row in 0..tile.height as u32 {
        let src_y = tile.y + row;
        let start = ((src_y * image_width + tile.x) * 4) as usize;
        let end = start + tile.width as usize * 4;
        out.extend_from_slice(&rgba[start..end]);
    }
    Ok(out)
}

pub fn write_tile_rgba(
    dst_rgba: &mut [u8],
    image_width: u32,
    image_height: u32,
    tile: TileBounds,
    tile_rgba: &[u8],
) -> Result<()> {
    validate_rgba_len(dst_rgba, image_width, image_height)?;
    validate_tile_bounds(tile, image_width, image_height)?;
    if tile_rgba.len() != tile.byte_len() {
        bail!("tile byte length mismatch");
    }
    for row in 0..tile.height as u32 {
        let dst_y = tile.y + row;
        let dst_start = ((dst_y * image_width + tile.x) * 4) as usize;
        let dst_end = dst_start + tile.width as usize * 4;
        let src_start = row as usize * tile.width as usize * 4;
        let src_end = src_start + tile.width as usize * 4;
        dst_rgba[dst_start..dst_end].copy_from_slice(&tile_rgba[src_start..src_end]);
    }
    Ok(())
}

fn validate_rgba_len(rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        bail!(
            "RGBA buffer length mismatch: expected {expected}, got {}",
            rgba.len()
        );
    }
    Ok(())
}

fn validate_tile_bounds(tile: TileBounds, image_width: u32, image_height: u32) -> Result<()> {
    let x2 = tile.x + tile.width as u32;
    let y2 = tile.y + tile.height as u32;
    if x2 > image_width || y2 > image_height {
        bail!(
            "tile out of bounds: ({}, {}) {}x{} outside {}x{}",
            tile.x,
            tile.y,
            tile.width,
            tile.height,
            image_width,
            image_height
        );
    }
    Ok(())
}
