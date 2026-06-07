use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileBounds {
    pub x: u32,
    pub y: u32,
    pub width: u16,
    pub height: u16,
}

impl TileBounds {
    pub fn pixel_count(self) -> Result<usize> {
        usize::from(self.width)
            .checked_mul(usize::from(self.height))
            .ok_or_else(|| anyhow::anyhow!("tile pixel count overflow"))
    }

    pub fn byte_len(self) -> Result<usize> {
        self.pixel_count()?
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("tile byte length overflow"))
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
    let mut out = Vec::with_capacity(tile.byte_len()?);
    for row in 0..tile.height as u32 {
        let src_y = tile.y + row;
        let start = rgba_offset(src_y, tile.x, image_width)?;
        let row_bytes = usize::from(tile.width)
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("tile row byte length overflow"))?;
        let end = start
            .checked_add(row_bytes)
            .ok_or_else(|| anyhow::anyhow!("tile extraction end offset overflow"))?;
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
    if tile_rgba.len() != tile.byte_len()? {
        bail!("tile byte length mismatch");
    }
    let row_bytes = usize::from(tile.width)
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("tile row byte length overflow"))?;
    for row in 0..tile.height as u32 {
        let dst_y = tile.y + row;
        let dst_start = rgba_offset(dst_y, tile.x, image_width)?;
        let dst_end = dst_start
            .checked_add(row_bytes)
            .ok_or_else(|| anyhow::anyhow!("tile write end offset overflow"))?;
        let src_start = usize::try_from(row)
            .context("tile row index exceeds platform usize")?
            .checked_mul(row_bytes)
            .ok_or_else(|| anyhow::anyhow!("tile source row offset overflow"))?;
        let src_end = src_start
            .checked_add(row_bytes)
            .ok_or_else(|| anyhow::anyhow!("tile source end offset overflow"))?;
        dst_rgba[dst_start..dst_end].copy_from_slice(&tile_rgba[src_start..src_end]);
    }
    Ok(())
}

fn validate_rgba_len(rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("RGBA buffer expected length overflow"))?;
    let expected =
        usize::try_from(expected).context("RGBA buffer expected length exceeds platform usize")?;
    if rgba.len() != expected {
        bail!(
            "RGBA buffer length mismatch: expected {expected}, got {}",
            rgba.len()
        );
    }
    Ok(())
}

fn validate_tile_bounds(tile: TileBounds, image_width: u32, image_height: u32) -> Result<()> {
    let x2 = tile
        .x
        .checked_add(u32::from(tile.width))
        .ok_or_else(|| anyhow::anyhow!("tile x bound overflow"))?;
    let y2 = tile
        .y
        .checked_add(u32::from(tile.height))
        .ok_or_else(|| anyhow::anyhow!("tile y bound overflow"))?;
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

fn rgba_offset(y: u32, x: u32, image_width: u32) -> Result<usize> {
    let pixel_index = u64::from(y)
        .checked_mul(u64::from(image_width))
        .and_then(|n| n.checked_add(u64::from(x)))
        .context("RGBA pixel offset overflow")?;
    let byte_index = pixel_index
        .checked_mul(4)
        .context("RGBA byte offset overflow")?;
    usize::try_from(byte_index).context("RGBA byte offset exceeds platform usize")
}
