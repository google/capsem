//! Retention: dropping blocks whose ledger rows have aged out.
//!
//! A body archive is append-only, so the only way to reclaim the bytes of an
//! expired body is to write the file again without it. The kept blocks are
//! copied **verbatim**, in ascending offset order, into a sibling temporary.
//!
//! Two phases, because the caller's SQLite index has to move with the file and
//! the two cannot commit together. `stage_retained_blocks` writes and flushes
//! the replacement while the original is still the live archive;
//! `commit_retained` is one `rename(2)`. The caller puts its index rows in
//! order between them, so the moment where the index and the file could
//! disagree is a single syscall rather than a whole transaction -- and until
//! the rename, a caller that hits trouble can still put everything back.
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
//! The original is never modified before the rename. Everything is built in
//! the temporary, and any error -- including an abandoned staging, which drops
//! it -- leaves the archive exactly as it was: a failed retention costs disk,
//! never evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// A compacted archive, written and flushed but not yet in place.
///
/// The original file is untouched while this exists, and dropping it removes
/// the replacement. Nothing is visible to a reader until `commit_retained`
/// renames it over the original, which is why the caller can put its index in
/// order first and still be able to walk away.
#[derive(Debug)]
pub struct RetainedStaging {
    destination: PathBuf,
    /// Removes itself unless `commit_retained` renames it, including when a
    /// panic unwinds through whoever was holding this.
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

    /// Where the replacement is waiting. For tests and diagnostics; a caller
    /// commits through `commit_retained` rather than renaming this itself.
    #[must_use]
    pub fn temporary_path(&self) -> &Path {
        self.temporary.path()
    }
}

/// Write a compacted copy of the archive at `path`, keeping only the blocks at
/// the offsets in `keep`, and flush it. **The original is not modified.**
///
/// This is the half of retention that can fail in many ways and the half that
/// takes time. Splitting it from the rename is what lets the caller commit its
/// index rows while the old file is still the live one, leaving a single
/// `rename(2)` between the two states rather than a whole transaction.
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
/// - [`ArchiveError::Io`] from any of the reads, the writes, or the
///   `sync_data`.
pub fn stage_retained_blocks(path: &Path, keep: &[u64]) -> Result<RetainedStaging> {
    refuse_symlink(path)?;
    let mut source = unix_fs::open_regular_file_no_follow(path)?;
    let before = source.metadata()?.len();
    let mut header = [0u8; FILE_HEADER_BYTES];
    source
        .read_exact(&mut header)
        .map_err(|_| ArchiveError::BadFileHeader)?;
    format::decode_file_header(&header)?;

    let mut temporary = unix_fs::create_private_sibling(path)?;
    let map = copy_kept_blocks(&mut source, temporary.file(), keep)?;
    // Flushed here, not at the rename: after `commit_retained` returns, the
    // bytes the index now names must already be on the device.
    temporary.file().sync_data()?;
    let after = temporary.file().metadata()?.len();
    // Every `?` above drops the staging, which removes the temporary.
    Ok(RetainedStaging {
        destination: path.to_path_buf(),
        temporary,
        map,
        bytes_freed: before.saturating_sub(after),
    })
}

/// Put the compacted copy in place, atomically.
///
/// One `rename(2)`: a reader sees either the whole old archive or the whole
/// new one. This is the only step of retention that changes what is at the
/// archive path, and it is deliberately the last thing that happens, so
/// everything that can fail has already failed with the original still live.
///
/// **The parent directory is fsynced before this returns**, so the rename is
/// durable and not merely done. The caller commits an index naming the new
/// offsets, and SQLite makes *that* durable at commit; if the rename were
/// still only in the page cache, a power loss could leave the durable index
/// pointing into a file the directory entry still names as the old one, with
/// nothing afterwards to notice. Ordering two durable things requires both to
/// be durable.
///
/// # Errors
///
/// [`ArchiveError::Io`] from the rename or the directory sync. The staging is
/// removed, so a refused commit leaves exactly the state that existed before
/// staging.
pub fn commit_retained(staging: RetainedStaging) -> Result<()> {
    let RetainedStaging {
        destination, temporary, ..
    } = staging;
    unix_fs::rename_private_sibling(temporary, &destination).map_err(ArchiveError::Io)
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
