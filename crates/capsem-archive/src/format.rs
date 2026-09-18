//! Byte layout of `session.bodies`.
//!
//! ```text
//! file header (16 bytes):  magic "CAPSEMBL"  u16 version  u16 reserved  u32 reserved
//! block (repeated):        magic "BLK1"  u32 raw_len  u32 comp_len  blake3(raw)[32]  comp
//! ```
//!
//! Every length field here arrives from disk, and the file is written beside
//! bytes an agent's counterparty chose. Each one is therefore bounded before
//! it is used to size an allocation, and the raw bytes are hashed so a block
//! that inflates to the right length but the wrong content is still refused.

use crate::{ArchiveError, Result};

pub const FILE_MAGIC: &[u8; 8] = b"CAPSEMBL";
pub const FILE_VERSION: u16 = 1;
pub const FILE_HEADER_BYTES: usize = 16;
pub const BLOCK_MAGIC: &[u8; 4] = b"BLK1";
/// magic(4) + raw_len(4) + comp_len(4) + blake3(32)
pub const BLOCK_HEADER_BYTES: usize = 44;
/// Raw bytes a block accumulates before it seals. Large enough that
/// same-provider bodies share dictionary context and that identical bodies
/// written close together land in one block, where the logger stores them
/// once; small enough that reading one body inflates about a millisecond of
/// data.
pub const TARGET_BLOCK_BYTES: usize = 1024 * 1024;
/// Hard ceiling on one block's raw size, so a hostile `raw_len` cannot
/// make a reader allocate without bound. One body may be up to 10 MiB
/// (`MAX_BODY_BLOB_BYTES` in capsem-logger), so a block holds at least one.
///
/// This is also the reader's memory bound. Inflating a block holds the
/// compressed payload and the inflated bytes at once -- about twice this
/// ceiling transiently -- and retains one inflated block, about this ceiling,
/// in the one-block cache. In practice blocks seal at `TARGET_BLOCK_BYTES`,
/// sixteen times below, and only a forged header or a body near the
/// 10 MiB cap approaches it.
pub const MAX_BLOCK_RAW_BYTES: usize = 16 * 1024 * 1024;
/// Deflate never expands 16 MiB by more than a few KiB; anything beyond
/// this is a forged header, refused before any allocation.
pub const MAX_BLOCK_COMP_BYTES: usize = MAX_BLOCK_RAW_BYTES + 64 * 1024;
const DEFLATE_LEVEL: u8 = 6;

/// Where one body lives: the block's file offset and its span inside the
/// block's raw bytes. Stored in the SQLite index row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyRef {
    pub block_offset: u64,
    pub offset: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub raw_len: u32,
    pub comp_len: u32,
    pub hash: [u8; 32],
}

#[must_use]
pub fn encode_file_header() -> [u8; FILE_HEADER_BYTES] {
    let mut header = [0u8; FILE_HEADER_BYTES];
    header[..8].copy_from_slice(FILE_MAGIC);
    header[8..10].copy_from_slice(&FILE_VERSION.to_le_bytes());
    header
}

/// Accept only this crate's own magic and version. Anything else is another
/// file that happens to sit at the archive path, not an archive to append to.
pub fn decode_file_header(bytes: &[u8]) -> Result<()> {
    let header = bytes.get(..FILE_HEADER_BYTES).ok_or(ArchiveError::BadFileHeader)?;
    if &header[..8] != FILE_MAGIC {
        return Err(ArchiveError::BadFileHeader);
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if version != FILE_VERSION {
        return Err(ArchiveError::BadFileHeader);
    }
    Ok(())
}

/// Deflate `raw` into a complete block: header followed by compressed bytes.
///
/// # Panics
///
/// If `raw` does not fit a `u32`. The length fields are 32-bit, so a block
/// that large could not be described; silently truncating the header instead
/// would write an archive whose own reader rejects it.
#[must_use]
pub fn encode_block(raw: &[u8]) -> Vec<u8> {
    let raw_len = u32::try_from(raw.len()).expect("a block's raw size fits a u32");
    let comp = miniz_oxide::deflate::compress_to_vec(raw, DEFLATE_LEVEL);
    let comp_len = u32::try_from(comp.len()).expect("a block's compressed size fits a u32");
    let hash = blake3::hash(raw);
    let mut block = Vec::with_capacity(BLOCK_HEADER_BYTES + comp.len());
    block.extend_from_slice(BLOCK_MAGIC);
    block.extend_from_slice(&raw_len.to_le_bytes());
    block.extend_from_slice(&comp_len.to_le_bytes());
    block.extend_from_slice(hash.as_bytes());
    block.extend_from_slice(&comp);
    block
}

/// Parse a block header at `block_offset` and return it with the compressed
/// payload slice. `bytes` must start at the block magic.
pub fn split_block(bytes: &[u8], block_offset: u64) -> Result<(BlockHeader, &[u8])> {
    let head: &[u8; BLOCK_HEADER_BYTES] = bytes
        .get(..BLOCK_HEADER_BYTES)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(ArchiveError::BadBlockHeader(block_offset))?;
    let header = parse_block_header(head, block_offset)?;
    let comp = bytes
        .get(BLOCK_HEADER_BYTES..BLOCK_HEADER_BYTES + header.comp_len as usize)
        .ok_or(ArchiveError::BadBlockHeader(block_offset))?;
    Ok((header, comp))
}

/// Parse just the header fields of a block (no payload), for readers that
/// size their buffer before reading the payload.
pub fn parse_block_header(bytes: &[u8; BLOCK_HEADER_BYTES], block_offset: u64) -> Result<BlockHeader> {
    if &bytes[..4] != BLOCK_MAGIC {
        return Err(ArchiveError::BadBlockHeader(block_offset));
    }
    let raw_len = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    let comp_len = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    if raw_len as usize > MAX_BLOCK_RAW_BYTES || comp_len as usize > MAX_BLOCK_COMP_BYTES {
        return Err(ArchiveError::BadBlockHeader(block_offset));
    }
    let hash: [u8; 32] = bytes[12..BLOCK_HEADER_BYTES].try_into().expect("32 bytes");
    Ok(BlockHeader {
        raw_len,
        comp_len,
        hash,
    })
}

/// Inflate one block and verify its hash.
pub fn decode_block(header: &BlockHeader, comp: &[u8], block_offset: u64) -> Result<Vec<u8>> {
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(comp, header.raw_len as usize)
        .map_err(|error| ArchiveError::Inflate(block_offset, error.to_string()))?;
    if raw.len() != header.raw_len as usize || blake3::hash(&raw).as_bytes() != &header.hash {
        return Err(ArchiveError::Integrity(block_offset));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests;
