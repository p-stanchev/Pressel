use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub const MAGIC_BYTES: &[u8; 5] = b"PRSL1";
pub const CHANNELS_RGBA8: u8 = 4;
pub const DEFAULT_TILE_SIZE: u16 = 64;
pub const TILE_METADATA_SIZE: usize = 19;
pub const MAX_IMAGE_DIMENSION: u32 = 32_768;
pub const MAX_RGBA_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresselHeader {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub tile_size: u16,
    pub tile_count: u32,
    pub original_pixel_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileHeader {
    pub x: u32,
    pub y: u32,
    pub width: u16,
    pub height: u16,
    pub transform_id: u8,
    pub predictor_id: u8,
    pub entropy_backend_id: u8,
    pub compressed_payload_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedTile {
    pub header: TileHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresselFile {
    pub header: PresselHeader,
    pub tiles: Vec<EncodedTile>,
}

pub fn rgba_sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = rgba_sha256(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

impl PresselHeader {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(MAGIC_BYTES)?;
        writer.write_all(&self.width.to_le_bytes())?;
        writer.write_all(&self.height.to_le_bytes())?;
        writer.write_all(&[self.channels])?;
        writer.write_all(&self.tile_size.to_le_bytes())?;
        writer.write_all(&self.tile_count.to_le_bytes())?;
        writer.write_all(&self.original_pixel_hash)?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut magic = [0_u8; 5];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC_BYTES {
            bail!("invalid PRSL magic bytes");
        }

        let width = read_u32(reader)?;
        let height = read_u32(reader)?;
        let channels = read_u8(reader)?;
        let tile_size = read_u16(reader)?;
        let tile_count = read_u32(reader)?;
        let mut original_pixel_hash = [0_u8; 32];
        reader.read_exact(&mut original_pixel_hash)?;

        if channels != CHANNELS_RGBA8 {
            bail!("unsupported channel count: {channels}");
        }
        if width == 0 || height == 0 {
            bail!("invalid zero-sized image: {}x{}", width, height);
        }
        if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
            bail!(
                "image dimensions exceed limit: {}x{} > {}",
                width,
                height,
                MAX_IMAGE_DIMENSION
            );
        }
        if tile_size == 0 {
            bail!("invalid tile size 0");
        }
        let rgba_bytes = rgba_byte_len(width, height)?;
        if rgba_bytes > MAX_RGBA_BYTES {
            bail!("image RGBA buffer exceeds limit: {rgba_bytes} bytes > {MAX_RGBA_BYTES} bytes");
        }

        Ok(Self {
            width,
            height,
            channels,
            tile_size,
            tile_count,
            original_pixel_hash,
        })
    }
}

impl TileHeader {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.x.to_le_bytes())?;
        writer.write_all(&self.y.to_le_bytes())?;
        writer.write_all(&self.width.to_le_bytes())?;
        writer.write_all(&self.height.to_le_bytes())?;
        writer.write_all(&[self.transform_id])?;
        writer.write_all(&[self.predictor_id])?;
        writer.write_all(&[self.entropy_backend_id])?;
        writer.write_all(&self.compressed_payload_len.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let x = read_u32(reader)?;
        let y = read_u32(reader)?;
        let width = read_u16(reader)?;
        let height = read_u16(reader)?;
        let transform_id = read_u8(reader)?;
        let predictor_id = read_u8(reader)?;
        let entropy_backend_id = read_u8(reader)?;
        let compressed_payload_len = read_u32(reader)?;

        Ok(Self {
            x,
            y,
            width,
            height,
            transform_id,
            predictor_id,
            entropy_backend_id,
            compressed_payload_len,
        })
    }
}

impl PresselFile {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.header.write_to(writer)?;
        for tile in &self.tiles {
            tile.header.write_to(writer)?;
            writer.write_all(&tile.payload)?;
        }
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let header = PresselHeader::read_from(reader)?;
        let mut tiles = Vec::with_capacity(header.tile_count as usize);
        for _ in 0..header.tile_count {
            let tile_header = TileHeader::read_from(reader)?;
            if tile_header.width == 0 || tile_header.height == 0 {
                bail!(
                    "invalid zero-sized tile at ({}, {})",
                    tile_header.x,
                    tile_header.y
                );
            }
            let mut payload = vec![0_u8; tile_header.compressed_payload_len as usize];
            reader.read_exact(&mut payload).with_context(|| {
                format!(
                    "failed reading tile payload at ({}, {})",
                    tile_header.x, tile_header.y
                )
            })?;
            tiles.push(EncodedTile {
                header: tile_header,
                payload,
            });
        }
        Ok(Self { header, tiles })
    }
}

fn read_u8<R: Read>(reader: &mut R) -> Result<u8> {
    let mut buf = [0_u8; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16<R: Read>(reader: &mut R) -> Result<u16> {
    let mut buf = [0_u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32> {
    let mut buf = [0_u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn rgba_byte_len(width: u32, height: u32) -> Result<usize> {
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .context("image pixel count overflow")?;
    let rgba_bytes = pixels
        .checked_mul(CHANNELS_RGBA8 as u64)
        .context("image RGBA byte count overflow")?;
    usize::try_from(rgba_bytes).context("image RGBA byte count exceeds platform usize")
}
