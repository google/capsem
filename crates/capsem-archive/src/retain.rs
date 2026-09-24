//! Retention: dropping blocks whose ledger rows have aged out.
//!
//! An archive is append-only, so the only way to reclaim an expired block is
//! to write the file again without it. Kept blocks are copied **verbatim**,
//! in ascending offset order, into a sibling temporary: no re-deflating, so a
//! retention pass costs I/O rather than CPU over the whole surviving archive,
//! and every segment keeps the blake3 its writer computed.
//!
//! A block is kept to its **committed extent** -- `body_blocks.disk_len`, the
//! bytes the index vouches for -- not to the end of whatever the file holds
//! after it. The segment headers inside that extent are walked (not
//! inflated) first, so an extent that does not end exactly on a segment
//! boundary is refused rather than copied: a torn tail past a stranded
//! block's last committed segment is dropped here, and a wrong `disk_len` is
//! caught before it becomes a permanent part of the file.
//!
//! Two phases, because the caller's index has to move with the file and the
//! two cannot commit together. `stage_retained_blocks` writes and flushes the
//! replacement while the original is still live; `commit_retained` is one
//! `rename(2)`. The original is never modified before the rename, and any
//! error -- including an abandoned staging, which removes its temporary --
//! leaves the archive exactly as it was.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use capsem_foundation::unix::fs as unix_fs;

use super::format::{self, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, SEGMENT_HEADER_BYTES};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// A compacted archive, written and flushed but not yet in place. Dropping it
/// removes the replacement; nothing is visible to a reader until
/// `commit_retained` renames it over the original.
#[derive(Debug)]
pub struct RetainedStaging {
    destination: PathBuf,
    temporary: unix_fs::PrivateSibling,
    map: BTreeMap<u64, u64>,
    bytes_freed: u64,
}

impl RetainedStaging {
    /// Old offset -> new offset for every kept block, once committed.
    #[must_use]
    pub fn map(&self) -> &BTreeMap<u64, u64> {
        &self.map
    }

    /// How much shorter the archive becomes when this is committed.
    #[must_use]
    pub fn bytes_freed(&self) -> u64 {
        self.bytes_freed
    }

    /// Where the replacement is waiting, for tests and diagnostics.
    #[must_use]
    pub fn temporary_path(&self) -> &Path {
        self.temporary.path()
    }
}

/// Write a compacted copy of the archive at `path` holding only the blocks in
/// `keep`, each `(block_offset, disk_len)`, and flush it. **The original is
/// not modified.**
///
/// # Errors
///
/// - [`ArchiveError::Symlink`] when the path is a link.
/// - [`ArchiveError::BadFileHeader`] when the file is not this version's
///   archive.
/// - [`ArchiveError::BadBlockHeader`] / [`ArchiveError::UnsupportedCodec`]
///   when a kept offset does not begin a block, [`ArchiveError::BadSegment`]
///   when its extent does not end on one of its segment boundaries, and
///   [`ArchiveError::TruncatedBlock`] when the file ends inside the extent.
///   An extent this code could not agree on is never copied.
/// - [`ArchiveError::Io`] from any read, write or the `sync_data`.
pub fn stage_retained_blocks(path: &Path, keep: &[(u64, u64)]) -> Result<RetainedStaging> {
    refuse_symlink(path)?;
    let mut source = unix_fs::open_regular_file_no_follow(path)?;
    let before = source.metadata()?.len();
    let mut header = [0u8; FILE_HEADER_BYTES];
    source
        .read_exact(&mut header)
        .map_err(|_| ArchiveError::BadFileHeader)?;
    let header = format::decode_file_header(&header)?;

    let mut temporary = unix_fs::create_private_sibling(path)?;
    let map = copy_kept_blocks(&mut source, temporary.file(), keep, header)?;
    // Flushed here, not at the rename: once `commit_retained` returns, the
    // bytes the index names must already be on the device.
    temporary.file().sync_data()?;
    let after = temporary.file().metadata()?.len();
    Ok(RetainedStaging {
        destination: path.to_path_buf(),
        temporary,
        map,
        bytes_freed: before.saturating_sub(after),
    })
}

/// Put the compacted copy in place with one `rename(2)`, and fsync the parent
/// directory so the rename is durable before the caller relies on it.
///
/// # Errors
///
/// [`ArchiveError::Io`] from the rename or the directory sync. The staging is
/// removed either way.
pub fn commit_retained(staging: RetainedStaging) -> Result<()> {
    let RetainedStaging {
        destination, temporary, ..
    } = staging;
    unix_fs::rename_private_sibling(temporary, &destination).map_err(ArchiveError::Io)
}

/// Stream the kept extents into `destination` behind a fresh file header, in
/// ascending offset order: one forward pass over the source.
fn copy_kept_blocks(
    source: &mut File,
    destination: &mut File,
    keep: &[(u64, u64)],
    header: format::FileHeader,
) -> Result<BTreeMap<u64, u64>> {
    destination.write_all(&format::encode_file_header(header.archive_id, header.generation_id))?;
    let mut end = FILE_HEADER_BYTES as u64;
    let mut moved = BTreeMap::new();
    let ascending: BTreeMap<u64, u64> = keep.iter().copied().collect();
    for (block_offset, disk_len) in ascending {
        validate_block_extent(source, block_offset, disk_len)?;
        source.seek(SeekFrom::Start(block_offset))?;
        let copied = io::copy(&mut (&mut *source).take(disk_len), destination)?;
        if copied != disk_len {
            return Err(ArchiveError::TruncatedBlock(block_offset));
        }
        moved.insert(block_offset, end);
        end += disk_len;
    }
    Ok(moved)
}

/// Prove `disk_len` bytes from `block_offset` are one block header followed
/// by whole segments, by walking the segment headers without inflating.
pub fn validate_block_extent(source: &mut File, block_offset: u64, disk_len: u64) -> Result<u32> {
    source.seek(SeekFrom::Start(block_offset))?;
    let mut head = [0u8; BLOCK_HEADER_BYTES];
    source
        .read_exact(&mut head)
        .map_err(|_| ArchiveError::BadBlockHeader(block_offset))?;
    format::parse_block_header(&head, block_offset)?;
    let mut at = block_offset + BLOCK_HEADER_BYTES as u64;
    let extent_end = block_offset
        .checked_add(disk_len)
        .ok_or(ArchiveError::CommittedExtent)?;
    let mut raw_start = 0u32;
    let mut last = false;
    while at < extent_end {
        if last {
            // Bytes after a FINAL segment are not part of this block.
            return Err(ArchiveError::BadSegment(at));
        }
        source.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; SEGMENT_HEADER_BYTES];
        source
            .read_exact(&mut head)
            .map_err(|_| ArchiveError::TruncatedBlock(block_offset))?;
        let segment = format::parse_segment_header(&head, at, raw_start)?;
        raw_start = raw_start
            .checked_add(segment.raw_len)
            .ok_or(ArchiveError::BadSegment(at))?;
        last = segment.last;
        at = at
            .checked_add((SEGMENT_HEADER_BYTES + segment.comp_len as usize) as u64)
            .ok_or(ArchiveError::CommittedExtent)?;
    }
    if at != extent_end || raw_start == 0 {
        return Err(ArchiveError::BadSegment(at.min(extent_end)));
    }
    Ok(raw_start)
}

#[cfg(test)]
mod tests;
