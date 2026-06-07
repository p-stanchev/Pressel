use anyhow::{Context, Result, bail};
#[cfg(test)]
use crc32fast::Hasher;

pub const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
pub const TAG_PNG_METADATA_CHUNKS: u16 = 0x0001;
pub const TAG_PNG_ANCILLARY_CHUNKS: u16 = 0x0002;
pub const TAG_ORIGINAL_SOURCE_FILE: u16 = 0x0003;

pub const PLACEMENT_BEFORE_PLTE: u8 = 0;
pub const PLACEMENT_BEFORE_IDAT: u8 = 1;
pub const PLACEMENT_AFTER_IDAT: u8 = 2;
pub const PLACEMENT_BEFORE_IEND: u8 = 3;

pub const CHUNK_FLAG_ANCILLARY: u8 = 1 << 0;
pub const CHUNK_FLAG_SAFE_TO_COPY: u8 = 1 << 1;
pub const CHUNK_FLAG_KNOWN_COMMON_METADATA: u8 = 1 << 2;
pub const CHUNK_FLAG_UNSAFE_TO_RESTORE_WITHOUT_WARNING: u8 = 1 << 3;

const CRITICAL_CHUNKS: [[u8; 4]; 5] = [*b"IHDR", *b"PLTE", *b"IDAT", *b"IEND", *b"tRNS"];
const COMMON_METADATA_CHUNKS: [[u8; 4]; 10] = [
    *b"gAMA", *b"cHRM", *b"sRGB", *b"iCCP", *b"pHYs", *b"tIME", *b"tEXt", *b"zTXt", *b"iTXt",
    *b"eXIf",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngChunkRecord {
    pub chunk_type: [u8; 4],
    pub placement: u8,
    pub flags: u8,
    pub original_crc: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PngPreservationData {
    pub metadata_chunks: Vec<PngChunkRecord>,
    pub ancillary_chunks: Vec<PngChunkRecord>,
    pub source_file: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PngPreservationOptions {
    pub preserve_png_metadata: bool,
    pub preserve_png_chunks: bool,
    pub preserve_source_file: bool,
}

pub fn is_png_file(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_SIGNATURE)
}

pub fn collect_png_preservation(
    bytes: &[u8],
    options: PngPreservationOptions,
) -> Result<PngPreservationData> {
    if !options.preserve_png_metadata
        && !options.preserve_png_chunks
        && !options.preserve_source_file
    {
        return Ok(PngPreservationData::default());
    }
    if !is_png_file(bytes) {
        bail!("PNG preservation flags require a PNG input file");
    }

    let parsed = parse_png_chunks(bytes)?;
    let mut preservation = PngPreservationData::default();
    if options.preserve_png_chunks {
        preservation.ancillary_chunks = parsed
            .into_iter()
            .filter(|chunk| {
                chunk.flags & CHUNK_FLAG_ANCILLARY != 0
                    && !is_critical_chunk(&chunk.chunk_type)
                    && chunk.chunk_type != *b"IDAT"
                    && chunk.chunk_type != *b"IHDR"
                    && chunk.chunk_type != *b"IEND"
            })
            .collect();
    } else if options.preserve_png_metadata {
        preservation.metadata_chunks = parsed
            .into_iter()
            .filter(|chunk| chunk.flags & CHUNK_FLAG_KNOWN_COMMON_METADATA != 0)
            .collect();
    }

    if options.preserve_source_file {
        preservation.source_file = Some(bytes.to_vec());
    }
    Ok(preservation)
}

pub fn encode_chunk_records(records: &[PngChunkRecord]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for record in records {
        out.extend_from_slice(&record.chunk_type);
        out.push(record.placement);
        out.push(record.flags);
        out.extend_from_slice(&record.original_crc.to_le_bytes());
        out.extend_from_slice(&(record.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&record.data);
    }
    Ok(out)
}

pub fn decode_chunk_records(payload: &[u8]) -> Result<Vec<PngChunkRecord>> {
    let mut cursor = 0_usize;
    let count = read_le_u32(payload, &mut cursor)? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        if cursor + 14 > payload.len() {
            bail!("chunk record payload truncated");
        }
        let chunk_type = payload[cursor..cursor + 4]
            .try_into()
            .expect("fixed 4-byte chunk type");
        cursor += 4;
        let placement = payload[cursor];
        cursor += 1;
        let flags = payload[cursor];
        cursor += 1;
        let original_crc = read_le_u32(payload, &mut cursor)?;
        let data_len = read_le_u32(payload, &mut cursor)? as usize;
        let end = cursor
            .checked_add(data_len)
            .context("chunk record data length overflow")?;
        if end > payload.len() {
            bail!("chunk record payload truncated in data");
        }
        let data = payload[cursor..end].to_vec();
        cursor = end;
        records.push(PngChunkRecord {
            chunk_type,
            placement,
            flags,
            original_crc,
            data,
        });
    }
    if cursor != payload.len() {
        bail!("chunk record payload has trailing bytes");
    }
    Ok(records)
}

pub fn restore_preserved_chunks(
    generated_png: &[u8],
    metadata_chunks: &[PngChunkRecord],
    ancillary_chunks: &[PngChunkRecord],
) -> Result<(Vec<u8>, Vec<String>)> {
    if !is_png_file(generated_png) {
        bail!("generated output is not a PNG");
    }
    let chunks = parse_raw_png_chunks(generated_png)?;
    let mut warnings = Vec::new();
    let records = if !ancillary_chunks.is_empty() {
        ancillary_chunks.to_vec()
    } else {
        metadata_chunks.to_vec()
    };

    let mut before_plte = Vec::new();
    let mut before_idat = Vec::new();
    let mut after_idat = Vec::new();
    let mut before_iend = Vec::new();

    for record in records {
        let can_restore = if !ancillary_chunks.is_empty() {
            let unsafe_restore = record.flags & CHUNK_FLAG_UNSAFE_TO_RESTORE_WITHOUT_WARNING != 0;
            let safe_to_copy = record.flags & CHUNK_FLAG_SAFE_TO_COPY != 0;
            let common_metadata = record.flags & CHUNK_FLAG_KNOWN_COMMON_METADATA != 0;
            if unsafe_restore && !safe_to_copy && !common_metadata {
                warnings.push(format!(
                    "skipping unsafe-to-copy chunk {} during PNG export",
                    chunk_type_string(&record.chunk_type)
                ));
                false
            } else {
                true
            }
        } else {
            true
        };
        if !can_restore {
            continue;
        }
        match record.placement {
            PLACEMENT_BEFORE_PLTE => before_plte.push(record),
            PLACEMENT_BEFORE_IDAT => before_idat.push(record),
            PLACEMENT_AFTER_IDAT => after_idat.push(record),
            PLACEMENT_BEFORE_IEND => before_iend.push(record),
            _ => warnings.push(format!(
                "skipping chunk {} with unknown placement {}",
                chunk_type_string(&record.chunk_type),
                record.placement
            )),
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(PNG_SIGNATURE);
    let first_idat = chunks
        .iter()
        .position(|chunk| chunk.chunk_type == *b"IDAT")
        .context("generated PNG missing IDAT")?;
    let first_plte = chunks.iter().position(|chunk| chunk.chunk_type == *b"PLTE");
    let iend_index = chunks
        .iter()
        .position(|chunk| chunk.chunk_type == *b"IEND")
        .context("generated PNG missing IEND")?;
    let idat_end = chunks
        .iter()
        .rposition(|chunk| chunk.chunk_type == *b"IDAT")
        .context("generated PNG missing closing IDAT")?;

    for (idx, chunk) in chunks.iter().enumerate() {
        if idx == 1 {
            for record in &before_plte {
                if first_plte.is_none() {
                    write_png_chunk_record(&mut out, record)?;
                }
            }
        }
        if Some(idx) == first_plte {
            for record in &before_plte {
                write_png_chunk_record(&mut out, record)?;
            }
        }
        if idx == first_idat {
            for record in &before_idat {
                write_png_chunk_record(&mut out, record)?;
            }
        }
        if idx == iend_index {
            for record in &after_idat {
                write_png_chunk_record(&mut out, record)?;
            }
            for record in &before_iend {
                write_png_chunk_record(&mut out, record)?;
            }
        }
        write_raw_png_chunk(&mut out, chunk)?;
        if idx == idat_end && iend_index == chunks.len() {
            for record in &after_idat {
                write_png_chunk_record(&mut out, record)?;
            }
        }
    }

    Ok((out, warnings))
}

#[cfg(test)]
pub fn make_test_png_with_chunks(base_png: &[u8], records: &[PngChunkRecord]) -> Result<Vec<u8>> {
    restore_preserved_chunks(base_png, records, &[]).map(|(bytes, _warnings)| bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawPngChunk {
    chunk_type: [u8; 4],
    data: Vec<u8>,
    crc: u32,
}

fn parse_png_chunks(bytes: &[u8]) -> Result<Vec<PngChunkRecord>> {
    let raw_chunks = parse_raw_png_chunks(bytes)?;
    let mut placement_state = PLACEMENT_BEFORE_PLTE;
    let mut idat_seen = false;
    let mut records = Vec::new();
    for chunk in raw_chunks {
        let chunk_type = chunk.chunk_type;
        match &chunk_type {
            b"PLTE" if !idat_seen => {
                placement_state = PLACEMENT_BEFORE_IDAT;
                continue;
            }
            b"IDAT" => {
                idat_seen = true;
                placement_state = PLACEMENT_AFTER_IDAT;
                continue;
            }
            b"IEND" => {
                placement_state = PLACEMENT_BEFORE_IEND;
                continue;
            }
            b"IHDR" => continue,
            _ => {}
        }
        let ancillary = is_ancillary(&chunk_type);
        if !ancillary {
            continue;
        }
        let safe_to_copy = is_safe_to_copy(&chunk_type);
        let known_common_metadata = is_common_metadata_chunk(&chunk_type);
        let mut flags = CHUNK_FLAG_ANCILLARY;
        if safe_to_copy {
            flags |= CHUNK_FLAG_SAFE_TO_COPY;
        }
        if known_common_metadata {
            flags |= CHUNK_FLAG_KNOWN_COMMON_METADATA;
        }
        if !safe_to_copy && !known_common_metadata {
            flags |= CHUNK_FLAG_UNSAFE_TO_RESTORE_WITHOUT_WARNING;
        }
        records.push(PngChunkRecord {
            chunk_type,
            placement: placement_state,
            flags,
            original_crc: chunk.crc,
            data: chunk.data,
        });
    }
    Ok(records)
}

fn parse_raw_png_chunks(bytes: &[u8]) -> Result<Vec<RawPngChunk>> {
    if !is_png_file(bytes) {
        bail!("input is not a PNG file");
    }
    let mut cursor = PNG_SIGNATURE.len();
    let mut chunks = Vec::new();
    while cursor < bytes.len() {
        let len = read_be_u32(bytes, &mut cursor)? as usize;
        if cursor + 8 + len > bytes.len() {
            bail!("PNG chunk stream truncated");
        }
        let chunk_type = bytes[cursor..cursor + 4]
            .try_into()
            .expect("fixed 4-byte chunk type");
        cursor += 4;
        let data = bytes[cursor..cursor + len].to_vec();
        cursor += len;
        let crc = read_be_u32(bytes, &mut cursor)?;
        chunks.push(RawPngChunk {
            chunk_type,
            data,
            crc,
        });
        if chunks
            .last()
            .is_some_and(|chunk| chunk.chunk_type == *b"IEND")
        {
            break;
        }
    }
    Ok(chunks)
}

fn write_png_chunk_record(out: &mut Vec<u8>, record: &PngChunkRecord) -> Result<()> {
    out.extend_from_slice(&(record.data.len() as u32).to_be_bytes());
    out.extend_from_slice(&record.chunk_type);
    out.extend_from_slice(&record.data);
    out.extend_from_slice(&record.original_crc.to_be_bytes());
    Ok(())
}

fn write_raw_png_chunk(out: &mut Vec<u8>, chunk: &RawPngChunk) -> Result<()> {
    out.extend_from_slice(&(chunk.data.len() as u32).to_be_bytes());
    out.extend_from_slice(&chunk.chunk_type);
    out.extend_from_slice(&chunk.data);
    out.extend_from_slice(&chunk.crc.to_be_bytes());
    Ok(())
}

#[cfg(test)]
pub fn chunk_record(
    chunk_type: [u8; 4],
    placement: u8,
    data: Vec<u8>,
    safe_to_copy_override: Option<bool>,
) -> PngChunkRecord {
    let safe_to_copy = safe_to_copy_override.unwrap_or_else(|| is_safe_to_copy(&chunk_type));
    let known_common_metadata = is_common_metadata_chunk(&chunk_type);
    let mut flags = 0_u8;
    if is_ancillary(&chunk_type) {
        flags |= CHUNK_FLAG_ANCILLARY;
    }
    if safe_to_copy {
        flags |= CHUNK_FLAG_SAFE_TO_COPY;
    }
    if known_common_metadata {
        flags |= CHUNK_FLAG_KNOWN_COMMON_METADATA;
    }
    if !safe_to_copy && !known_common_metadata {
        flags |= CHUNK_FLAG_UNSAFE_TO_RESTORE_WITHOUT_WARNING;
    }
    PngChunkRecord {
        chunk_type,
        placement,
        flags,
        original_crc: png_crc(chunk_type, &data),
        data,
    }
}

#[cfg(test)]
fn png_crc(chunk_type: [u8; 4], data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(&chunk_type);
    hasher.update(data);
    hasher.finalize()
}

fn is_ancillary(chunk_type: &[u8; 4]) -> bool {
    chunk_type[0].is_ascii_lowercase()
}

fn is_safe_to_copy(chunk_type: &[u8; 4]) -> bool {
    chunk_type[3].is_ascii_lowercase()
}

fn is_common_metadata_chunk(chunk_type: &[u8; 4]) -> bool {
    COMMON_METADATA_CHUNKS.contains(chunk_type)
}

fn is_critical_chunk(chunk_type: &[u8; 4]) -> bool {
    CRITICAL_CHUNKS.contains(chunk_type)
}

fn read_be_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let end = cursor.checked_add(4).context("u32 read overflow")?;
    if end > bytes.len() {
        bail!("unexpected end of payload while reading u32");
    }
    let value = u32::from_be_bytes(bytes[*cursor..end].try_into().expect("4-byte slice"));
    *cursor = end;
    Ok(value)
}

fn read_le_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let end = cursor.checked_add(4).context("u32 read overflow")?;
    if end > bytes.len() {
        bail!("unexpected end of payload while reading u32");
    }
    let value = u32::from_le_bytes(bytes[*cursor..end].try_into().expect("4-byte slice"));
    *cursor = end;
    Ok(value)
}

fn chunk_type_string(chunk_type: &[u8; 4]) -> String {
    String::from_utf8_lossy(chunk_type).into_owned()
}
