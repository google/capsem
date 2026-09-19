//! Byte layout of a version 2 `session.bodies`.
//!
//! ```text
//! file header (16):  "CAPSEMBL" | u16 version = 2 | u16 0 | u32 0
//! block header (8):  "BLK2" | u8 codec | u8 flags = 0 | u16 0
//! segment (52+comp): "SGMT" | u8 flags (bit 0 = FINAL) | u8[3] 0
//!                    | u32 raw_start | u32 raw_len | u32 comp_len
//!                    | blake3(segment raw)[32] | comp
//! ```
//!
//! A block is one compression stream that stays open across the writer's
//! disk flushes. Each flush ends a **segment**: the compressor is
//! sync-flushed, which byte-aligns its output without resetting its
//! dictionary, and the segment's bytes are appended behind a header of their
//! own. A reader inflates a block segment by segment with one inflater, so a
//! body in segment `k` is readable the moment segment `k` is on disk, while
//! the block goes on compressing against everything before it. The last
//! segment of a closed block carries the FINAL flag and ends the stream.
//!
//! Before this, every five-second flush sealed its block, so real blocks
//! averaged about 85 KiB and the flush timer, not the data, capped the
//! compression ratio.
//!
//! The codec is recorded per block so a second one (zstd, with
//! `ZSTD_e_flush` as its sync point) is a new codec id rather than a new file
//! version. A reader refuses a codec it does not know by name.
//!
//! Every length here arrives from disk, beside bytes an agent's counterparty
//! chose, so each is bounded before it sizes an allocation, and each
//! segment's raw bytes are hashed so a segment that inflates to the right
//! length but the wrong content is still refused.

use crate::{ArchiveError, Result};

pub const FILE_MAGIC: &[u8; 8] = b"CAPSEMBL";
pub const FILE_VERSION: u16 = 2;
pub const FILE_HEADER_BYTES: usize = 16;
pub const BLOCK_MAGIC: &[u8; 4] = b"BLK2";
/// magic(4) + codec(1) + flags(1) + reserved(2)
pub const BLOCK_HEADER_BYTES: usize = 8;
pub const SEGMENT_MAGIC: &[u8; 4] = b"SGMT";
/// magic(4) + flags(1) + reserved(3) + raw_start(4) + raw_len(4) +
/// comp_len(4) + blake3(32)
pub const SEGMENT_HEADER_BYTES: usize = 52;
/// Raw deflate (RFC 1951), no zlib or gzip framing; every segment but the
/// last ends in a sync flush, the last in the stream's end.
pub const CODEC_DEFLATE: u8 = 1;
/// Flag bit 0 of a segment: this segment ends its block's stream.
pub const SEGMENT_FINAL: u8 = 0x01;
/// The four bytes a sync flush ends with: an empty stored block's length and
/// its complement. A non-final segment that does not end in them was not cut
/// at a flush point, and the next segment would not decode after it.
pub const SYNC_FLUSH_TAIL: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
/// Raw bytes a block accumulates before the writer closes it. Large enough
/// that same-provider bodies share dictionary context; small enough that
/// reading the last body of a block inflates about a millisecond of data.
pub const TARGET_BLOCK_BYTES: usize = 1024 * 1024;
/// Hard ceiling on one block's raw size, so a hostile header cannot make a
/// reader allocate without bound. One body may be up to 10 MiB
/// (`MAX_BODY_BLOB_BYTES` in capsem-logger), so a block holds at least one.
///
/// This is also the reader's memory bound: it keeps the inflated prefix of
/// one block, which in practice stops near `TARGET_BLOCK_BYTES`.
pub const MAX_BLOCK_RAW_BYTES: usize = 16 * 1024 * 1024;
/// Deflate never expands its input by more than a few bytes per 64 KiB
/// stored block, plus the flush's own few bytes. A segment whose compressed
/// length exceeds its raw length by more than this is a forged header,
/// refused before anything is read into memory.
pub const MAX_SEGMENT_EXPANSION: usize = 64 * 1024;
pub(crate) const DEFLATE_LEVEL: u32 = 6;

/// Where one body lives: the block's file offset and its span inside the
/// block's raw bytes. Stored in the SQLite index row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyRef {
    pub block_offset: u64,
    pub offset: u32,
    pub len: u32,
}

/// One segment's header, bounded and checked against where it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentHeader {
    pub last: bool,
    pub raw_start: u32,
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
/// file that happens to sit at the archive path -- including an archive of an
/// earlier version, which this reader does not decode.
pub fn decode_file_header(bytes: &[u8]) -> Result<()> {
    let header = bytes.get(..FILE_HEADER_BYTES).ok_or(ArchiveError::BadFileHeader)?;
    if &header[..8] != FILE_MAGIC || u16::from_le_bytes([header[8], header[9]]) != FILE_VERSION {
        return Err(ArchiveError::BadFileHeader);
    }
    Ok(())
}

#[must_use]
pub fn encode_block_header(codec: u8) -> [u8; BLOCK_HEADER_BYTES] {
    let mut header = [0u8; BLOCK_HEADER_BYTES];
    header[..4].copy_from_slice(BLOCK_MAGIC);
    header[4] = codec;
    header
}

/// Parse a block header and return its codec.
///
/// # Errors
///
/// [`ArchiveError::BadBlockHeader`] when the magic is wrong, and
/// [`ArchiveError::UnsupportedCodec`] for a codec, flag or reserved byte this
/// reader does not know: a later writer's block is refused by name rather
/// than inflated as something it is not.
pub fn parse_block_header(bytes: &[u8; BLOCK_HEADER_BYTES], block_offset: u64) -> Result<u8> {
    if &bytes[..4] != BLOCK_MAGIC {
        return Err(ArchiveError::BadBlockHeader(block_offset));
    }
    let codec = bytes[4];
    if codec != CODEC_DEFLATE || bytes[5] != 0 || bytes[6..8] != [0, 0] {
        return Err(ArchiveError::UnsupportedCodec { block_offset, codec });
    }
    Ok(codec)
}

#[must_use]
pub fn encode_segment_header(header: &SegmentHeader) -> [u8; SEGMENT_HEADER_BYTES] {
    let mut bytes = [0u8; SEGMENT_HEADER_BYTES];
    bytes[..4].copy_from_slice(SEGMENT_MAGIC);
    bytes[4] = if header.last { SEGMENT_FINAL } else { 0 };
    bytes[8..12].copy_from_slice(&header.raw_start.to_le_bytes());
    bytes[12..16].copy_from_slice(&header.raw_len.to_le_bytes());
    bytes[16..20].copy_from_slice(&header.comp_len.to_le_bytes());
    bytes[20..].copy_from_slice(&header.hash);
    bytes
}

/// Parse and bound the segment header at file offset `at`, which must
/// continue the block's raw bytes at `expected_raw_start`.
///
/// Every check here happens before a byte of the payload is read:
///
/// - the magic, and flag and reserved bits this reader knows;
/// - `raw_start` equal to the raw bytes before it, so segments cannot
///   overlap, skip or reorder;
/// - the block's raw extent within `MAX_BLOCK_RAW_BYTES`;
/// - `comp_len` within `raw_len + MAX_SEGMENT_EXPANSION`;
/// - `raw_len` non-zero unless FINAL. The writer never cuts an empty
///   segment, and a forged zero would otherwise be an unbounded inflate.
///
/// # Errors
///
/// [`ArchiveError::BadSegment`] naming `at` for any of them.
pub fn parse_segment_header(
    bytes: &[u8; SEGMENT_HEADER_BYTES],
    at: u64,
    expected_raw_start: u32,
) -> Result<SegmentHeader> {
    let bad = || ArchiveError::BadSegment(at);
    if &bytes[..4] != SEGMENT_MAGIC || bytes[4] & !SEGMENT_FINAL != 0 || bytes[5..8] != [0, 0, 0] {
        return Err(bad());
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
    let header = SegmentHeader {
        last: bytes[4] & SEGMENT_FINAL != 0,
        raw_start: word(8),
        raw_len: word(12),
        comp_len: word(16),
        hash: bytes[20..].try_into().expect("32 bytes"),
    };
    let raw_end = header.raw_start as usize + header.raw_len as usize;
    if header.raw_start != expected_raw_start
        || raw_end > MAX_BLOCK_RAW_BYTES
        || header.comp_len as usize > header.raw_len as usize + MAX_SEGMENT_EXPANSION
        || (header.raw_len == 0 && !header.last)
    {
        return Err(bad());
    }
    Ok(header)
}

#[cfg(test)]
mod tests;
