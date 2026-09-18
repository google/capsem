//! Resolve a `BodyRef` to bytes, inflating one block at a time.
//!
//! The SQLite index is the only source of truth for what exists: a reader
//! never scans the file. It seeks to the offset an index row names, bounds
//! every length in the header it finds there before allocating, and verifies
//! blake3 over the inflated bytes before returning any of them.
//!
//! **Integrity here is per block, not per body.** The hash proves that a
//! block's bytes are the bytes that were written; it says nothing about which
//! span of them a given body is. An index row whose `offset` and `len` were
//! edited selects a different, still-valid span of the same block and this
//! reader returns it without complaint -- see
//! `reader_returns_the_wrong_body_for_a_wrong_but_in_range_ref`, which locks
//! that in deliberately. The row is the claim, the block is the evidence, and
//! SQLite is where the claim is protected. A ledger whose index rows an
//! attacker can rewrite is already lost, and a per-body hash would not save
//! it: the same edit would move the hash.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// Reads bodies back out of one `session.bodies`.
///
/// One per thread: the file handle and the one-block cache live behind
/// `RefCell`, so this is `Send` but not `Sync`. Sharing one across threads
/// does not compile; the logger keeps a reader in a mutex slot and takes it
/// out for the duration of a read.
pub struct BodyLogReader {
    file: RefCell<File>,
    /// The path this reader was opened on, and the file that was there at the
    /// time. Retention replaces the archive by renaming a compacted copy over
    /// it, which leaves this handle reading a file that no longer has a name
    /// -- see `file_was_replaced`.
    path: PathBuf,
    identity: FileIdentity,
    /// The last inflated block. Bodies from one exchange land in one block,
    /// so a UI walking a session's rows in order hits this nearly every time.
    last: RefCell<Option<(u64, Vec<u8>)>>,
    inflated: Cell<u64>,
}

/// Which file a handle is on, as the filesystem answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

impl BodyLogReader {
    pub fn open(path: &Path) -> Result<Self> {
        refuse_symlink(path)?;
        let mut file = unix_fs::open_regular_file_no_follow(path)?;
        let identity = FileIdentity::of(&file.metadata()?);
        let mut header = [0u8; FILE_HEADER_BYTES];
        file.read_exact(&mut header).map_err(|_| ArchiveError::BadFileHeader)?;
        format::decode_file_header(&header)?;
        Ok(Self {
            file: RefCell::new(file),
            path: path.to_path_buf(),
            identity,
            last: RefCell::new(None),
            inflated: Cell::new(0),
        })
    }

    /// Resolve one body reference to owned bytes.
    ///
    /// Owned, not borrowed: handing out a view into the one-block cache would
    /// make the next `read` -- which may replace that block -- a runtime
    /// borrow panic in whichever caller happened to hold the view. A body is
    /// kilobytes and the copy is not worth an API that can panic.
    pub fn read(&self, reference: BodyRef) -> Result<Vec<u8>> {
        self.ensure_block(reference.block_offset)?;
        let cached = self.last.borrow();
        let block = &cached.as_ref().expect("the block was just inflated").1;
        let start = reference.offset as usize;
        let end = start
            .checked_add(reference.len as usize)
            .ok_or(ArchiveError::RefOutOfRange)?;
        block
            .get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(ArchiveError::RefOutOfRange)
    }

    /// Inflate the block at `block_offset` into the cache, unless it is
    /// already the cached one.
    fn ensure_block(&self, block_offset: u64) -> Result<()> {
        let cached = self
            .last
            .borrow()
            .as_ref()
            .is_some_and(|(offset, _)| *offset == block_offset);
        if !cached {
            let raw = self.inflate(block_offset)?;
            *self.last.borrow_mut() = Some((block_offset, raw));
        }
        Ok(())
    }

    /// Read and verify one block. Every length used to size an allocation
    /// here comes from `parse_block_header`, which bounds it first: a forged
    /// `comp_len` must be refused before it becomes a `vec![0; comp_len]`.
    fn inflate(&self, block_offset: u64) -> Result<Vec<u8>> {
        let (header, comp) = {
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(block_offset))?;
            let mut head = [0u8; BLOCK_HEADER_BYTES];
            file.read_exact(&mut head)
                .map_err(|_| ArchiveError::BadBlockHeader(block_offset))?;
            let header = format::parse_block_header(&head, block_offset)?;
            let mut comp = vec![0u8; header.comp_len as usize];
            file.read_exact(&mut comp)
                .map_err(|_| ArchiveError::TruncatedBlock(block_offset))?;
            (header, comp)
        };
        self.inflated.set(self.inflated.get() + 1);
        format::decode_block(&header, &comp, block_offset)
    }

    #[must_use]
    pub fn blocks_inflated(&self) -> u64 {
        self.inflated.get()
    }

    /// Whether something replaced the archive since this reader opened it.
    ///
    /// Retention renames a compacted copy over the file, so a reader that was
    /// already open keeps a descriptor on the old, now-nameless inode -- and
    /// every block in it has moved. Its cached block and its offsets are both
    /// stale, and the index it is being asked about is the new one, so the
    /// bodies it returns would fail their hash check. The owner asks this
    /// before a read and reopens when it is true.
    ///
    /// Identity rather than a version counter kept beside the file: this is
    /// the filesystem answering which file the handle is on, so it cannot
    /// disagree with the file the way a counter someone forgot to bump can.
    /// A path that cannot be stat'd counts as replaced -- the reopen that
    /// follows is where that failure belongs, with the path in its message.
    #[must_use]
    pub fn file_was_replaced(&self) -> bool {
        std::fs::metadata(&self.path).map_or(true, |metadata| FileIdentity::of(&metadata) != self.identity)
    }
}

#[cfg(test)]
mod tests;
