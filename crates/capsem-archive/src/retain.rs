//! Retention: dropping blocks whose ledger rows have aged out.
//!
//! A body archive is append-only, so the only way to reclaim the bytes of an
//! expired body is to write the file again without it. `retain_blocks` does
//! exactly that and nothing more: the kept blocks are copied **verbatim**, in
//! ascending offset order, into a sibling temporary that is then renamed over
//! the original.
//!
//! Verbatim matters twice. Re-deflating would spend CPU proportional to the
//! whole surviving archive on every retention pass, and it would re-encode
//! attacker-influenced bytes that were already accepted once -- the retained
//! block keeps the blake3 the writer computed, so a reader's integrity check
//! still proves the bytes are the bytes that were originally written.
//!
//! Blocks move, so their offsets move. The returned map is old offset -> new
//! offset for every kept block; the caller is responsible for remapping the
//! SQLite index rows that name them, in the same breath as deleting the rows
//! of the blocks that went away. Until it does, the index points at stale
//! offsets, which is why the caller does both under one transaction and the
//! archive exposes no half-step between them.
//!
//! The original is never modified. Everything is built in the temporary, and
//! any error removes the temporary and leaves the archive exactly as it was:
//! a failed retention costs disk, never evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// Rewrite the archive at `path` keeping only the blocks at the offsets in
/// `keep`, and return old offset -> new offset for each of them.
///
/// Offsets not present in the file are an error, not a silent skip: the
/// caller's keep set comes from the index, and an offset the index names but
/// the file does not have means the two have already diverged.
///
/// # Errors
///
/// - [`ArchiveError::Symlink`] when something replaced the archive with a
///   link at the path.
/// - [`ArchiveError::BadFileHeader`] when the file is not this crate's
///   archive, or [`ArchiveError::BadBlockHeader`] /
///   [`ArchiveError::TruncatedBlock`] when a kept offset does not begin a
///   parseable, complete block. A block that cannot be parsed is never
///   copied: copying bytes whose length this code could not agree on is how
///   a torn tail becomes a permanent one.
/// - [`ArchiveError::Io`] from any of the reads, writes, the `sync_data`, or
///   the rename.
pub fn retain_blocks(path: &Path, keep: &[u64]) -> Result<BTreeMap<u64, u64>> {
    refuse_symlink(path)?;
    let mut source = unix_fs::open_regular_file_no_follow(path)?;
    let mut header = [0u8; FILE_HEADER_BYTES];
    source
        .read_exact(&mut header)
        .map_err(|_| ArchiveError::BadFileHeader)?;
    format::decode_file_header(&header)?;

    let (mut temporary, temporary_path) = unix_fs::create_private_sibling(path)?;
    let outcome = copy_kept_blocks(&mut source, &mut temporary, keep).and_then(|moved| {
        temporary.sync_data()?;
        drop(temporary);
        std::fs::rename(&temporary_path, path)?;
        Ok(moved)
    });
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    outcome
}

/// Stream the kept blocks into `destination` behind a fresh file header.
///
/// `BTreeSet` rather than the caller's slice: ascending order makes the copy
/// one forward pass over the source, and it removes the question of what a
/// duplicated offset would mean.
fn copy_kept_blocks(source: &mut File, destination: &mut File, keep: &[u64]) -> Result<BTreeMap<u64, u64>> {
    destination.write_all(&format::encode_file_header())?;
    let mut end = FILE_HEADER_BYTES as u64;
    let mut moved = BTreeMap::new();
    let ascending: BTreeSet<u64> = keep.iter().copied().collect();
    for block_offset in ascending {
        let bytes = read_whole_block(source, block_offset)?;
        destination.write_all(&bytes)?;
        moved.insert(block_offset, end);
        end += bytes.len() as u64;
    }
    Ok(moved)
}

/// Read one complete block -- header and compressed payload -- as it sits on
/// disk. The header is parsed only to learn how long the block is and to
/// refuse one that is not a block at all; the bytes handed back are the
/// file's own.
fn read_whole_block(source: &mut File, block_offset: u64) -> Result<Vec<u8>> {
    source.seek(SeekFrom::Start(block_offset))?;
    let mut head = [0u8; BLOCK_HEADER_BYTES];
    source
        .read_exact(&mut head)
        .map_err(|_| ArchiveError::BadBlockHeader(block_offset))?;
    // Bounds `comp_len` before it sizes the buffer below.
    let header = format::parse_block_header(&head, block_offset)?;
    let mut bytes = Vec::with_capacity(BLOCK_HEADER_BYTES + header.comp_len as usize);
    bytes.extend_from_slice(&head);
    bytes.resize(BLOCK_HEADER_BYTES + header.comp_len as usize, 0);
    source
        .read_exact(&mut bytes[BLOCK_HEADER_BYTES..])
        .map_err(|_| ArchiveError::TruncatedBlock(block_offset))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
