//! Stage bodies into a pending block; seal and append when full or asked.
//!
//! Sealing is two-phase so the owner can deflate off its own thread:
//! `take_pending` hands out the raw block, `PendingBlock::encode` deflates
//! it anywhere, `append` writes the encoded bytes back in order. `seal` is
//! the three in sequence for owners that do not care.
//!
//! Two invariants survive that detour. Every block carries the sequence
//! number it was taken with, and `append` refuses anything but the next one,
//! so a block that came back late -- or came from a different writer -- is
//! rejected instead of landing at an offset its index rows do not name. And
//! a write that fails part-way through a block poisons the writer: the file's
//! end is no longer provably where it was, so every later offset would be a
//! guess, and guessing is how an index comes to point at the wrong bytes.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use std::os::unix::fs::PermissionsExt;

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, MAX_BLOCK_RAW_BYTES, TARGET_BLOCK_BYTES};
use crate::{ArchiveError, Result};

/// One block's raw bytes, detached from the writer so they can be deflated
/// anywhere. `Send`, deliberately: compression is the expensive half and does
/// not belong on the thread that owns the SQLite connection.
pub struct PendingBlock {
    seq: u64,
    raw: Vec<u8>,
}

/// A deflated block, ready to be appended. Holds the complete on-disk bytes:
/// block header followed by the compressed payload.
pub struct EncodedBlock {
    seq: u64,
    raw_len: u32,
    bytes: Vec<u8>,
}

/// Both halves of a two-phase seal cross a thread boundary by design, so
/// losing `Send` must be a compile error rather than a discovery. A const
/// block rather than a runtime closure: the check belongs to compilation, and
/// writing it as a function body would leave it reading as an untested line
/// forever after.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<PendingBlock>();
    assert_send::<EncodedBlock>();
};

/// Where a block landed. The owner stamps `block_offset` into the index rows
/// of every body the block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealedBlock {
    pub block_offset: u64,
    pub raw_len: u32,
    pub comp_len: u32,
}

/// Appends blocks to one `session.bodies`.
///
/// Dropping a writer with a pending block discards those bodies: they were
/// never written, and their index rows were never committed, so the ledger
/// stays consistent -- but the bodies are gone. Owners seal before dropping,
/// and a debug build asserts they did.
pub struct BodyLogWriter {
    file: File,
    end: u64,
    pending: Vec<u8>,
    /// Sequence number the next `take_pending` hands out.
    next_block_seq: u64,
    /// Sequence number `append` will accept next.
    next_append_seq: u64,
    /// Set by a write that failed part-way through a block. See
    /// [`ArchiveError::Poisoned`].
    poisoned: bool,
    /// Write this many bytes of the next block and then fail, producing a
    /// genuine torn tail on the real file rather than a simulated one.
    #[cfg(test)]
    fail_write_after: Option<usize>,
}

impl PendingBlock {
    #[must_use]
    pub fn encode(self) -> EncodedBlock {
        // The writer refuses a body that would take the pending block past
        // MAX_BLOCK_RAW_BYTES, so this length is bounded well inside a u32.
        let raw_len = u32::try_from(self.raw.len()).expect("a pending block fits a u32");
        EncodedBlock {
            seq: self.seq,
            raw_len,
            bytes: format::encode_block(&self.raw),
        }
    }

    #[must_use]
    pub fn raw_len(&self) -> usize {
        self.raw.len()
    }

    /// The order this block must be appended in.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

impl EncodedBlock {
    #[must_use]
    pub fn comp_len(&self) -> usize {
        self.bytes.len() - BLOCK_HEADER_BYTES
    }

    /// The order this block must be appended in.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

impl Drop for BodyLogWriter {
    fn drop(&mut self) {
        // Not while unwinding: a panic that happens to leave a block pending
        // would abort the process instead of surfacing its own cause.
        debug_assert!(
            std::thread::panicking() || self.pending.is_empty(),
            "BodyLogWriter dropped with {} bytes of unsealed bodies; seal before dropping",
            self.pending.len()
        );
    }
}

impl BodyLogWriter {
    /// Create or reopen `session.bodies`.
    ///
    /// Creates the file with mode 0o600, refuses a symlink, and opens without
    /// following links through `capsem-foundation::unix::fs`: the archive
    /// holds request and response bodies, which are the most sensitive bytes
    /// a session produces.
    ///
    /// A reopened file has its mode set back to 0o600 through the open handle
    /// rather than the path. A file created before this rule, or one whose
    /// mode was widened afterwards, would otherwise keep serving group and
    /// other a session's bodies for the rest of its life; the mode is a
    /// property this writer maintains, not one it checks once at creation.
    ///
    /// That mode repair is the one way this can refuse a file it could
    /// otherwise use: an `fchmod` the filesystem does not permit fails the
    /// open, and the logger then warns and stores no bodies for the session
    /// rather than writing to a file whose permissions it cannot vouch for.
    ///
    /// A reopened file is validated and appended after its current end. A
    /// torn tail from an earlier crash is left exactly where it is, because
    /// no SQLite index row points at it: unreachable bytes, never corruption.
    pub fn open(path: &Path) -> Result<Self> {
        refuse_symlink(path)?;
        let mut file = unix_fs::open_private_append_no_follow(path)?;

        let end = file.metadata()?.len();
        if end == 0 {
            file.write_all(&format::encode_file_header())?;
            return Ok(Self::at(file, FILE_HEADER_BYTES as u64));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let mut header = [0u8; FILE_HEADER_BYTES];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header).map_err(|_| ArchiveError::BadFileHeader)?;
        format::decode_file_header(&header)?;
        Ok(Self::at(file, end))
    }

    fn at(file: File, end: u64) -> Self {
        Self {
            file,
            end,
            pending: Vec::new(),
            next_block_seq: 0,
            next_append_seq: 0,
            poisoned: false,
            #[cfg(test)]
            fail_write_after: None,
        }
    }

    /// Append `body` to the pending block.
    ///
    /// The returned reference has a placeholder `block_offset` of 0; the
    /// caller sets it from the next `SealedBlock`, which the logger does
    /// inside one SQLite transaction so no index row is ever visible with a
    /// placeholder in it.
    ///
    /// # Errors
    ///
    /// - [`ArchiveError::BodyTooLarge`] when the body alone exceeds
    ///   `MAX_BLOCK_RAW_BYTES`. No block could hold it, so sealing does not
    ///   help and the owner must truncate or drop it.
    /// - [`ArchiveError::BlockFull`] when the body would take the *pending*
    ///   block past that ceiling. The contract is seal-and-retry: call
    ///   `seal` (or `take_pending`/`append`) and stage the same body again,
    ///   which is then guaranteed to fit an empty block.
    /// - [`ArchiveError::Poisoned`] after a partial write.
    ///
    /// Nothing here panics: an oversized body arrives from the network, and a
    /// panic on the writer thread would take the whole session ledger down.
    pub fn stage(&mut self, body: &[u8]) -> Result<BodyRef> {
        self.check_usable()?;
        if body.len() > MAX_BLOCK_RAW_BYTES {
            return Err(ArchiveError::BodyTooLarge {
                len: body.len(),
                max: MAX_BLOCK_RAW_BYTES,
            });
        }
        if self.pending.len() + body.len() > MAX_BLOCK_RAW_BYTES {
            return Err(ArchiveError::BlockFull);
        }
        // Both lengths are now bounded by MAX_BLOCK_RAW_BYTES, which is 16 MiB.
        let offset = u32::try_from(self.pending.len()).expect("bounded by MAX_BLOCK_RAW_BYTES");
        let len = u32::try_from(body.len()).expect("bounded by MAX_BLOCK_RAW_BYTES");
        self.pending.extend_from_slice(body);
        Ok(BodyRef {
            block_offset: 0,
            offset,
            len,
        })
    }

    fn check_usable(&self) -> Result<()> {
        if self.poisoned {
            return Err(ArchiveError::Poisoned);
        }
        Ok(())
    }

    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.pending.len()
    }

    /// True once the pending block has reached its target size.
    #[must_use]
    pub fn wants_seal(&self) -> bool {
        self.pending.len() >= TARGET_BLOCK_BYTES
    }

    /// Take the pending raw block out (`None` when empty, or when the writer
    /// is poisoned); the writer keeps accepting `stage` calls into a fresh
    /// block meanwhile. Offsets handed out by `stage` before this call belong
    /// to the taken block, and so does the sequence number it carries.
    pub fn take_pending(&mut self) -> Option<PendingBlock> {
        if self.poisoned || self.pending.is_empty() {
            return None;
        }
        let seq = self.next_block_seq;
        self.next_block_seq += 1;
        Some(PendingBlock {
            seq,
            raw: std::mem::take(&mut self.pending),
        })
    }

    /// Append an encoded block, which must be the next one this writer handed
    /// out.
    ///
    /// # Errors
    ///
    /// - [`ArchiveError::OutOfOrderBlock`] when the block's sequence number is
    ///   not the expected one. Appending out of order would put a block at an
    ///   offset the other block's index rows already claim, so two-phase
    ///   sealing is checked rather than trusted -- this also rejects a block
    ///   encoded by a different writer, whose sequence space is its own.
    /// - [`ArchiveError::Poisoned`] after an earlier partial write.
    /// - [`ArchiveError::Io`] from the write itself, which poisons the
    ///   writer: a block that reached the file in part leaves the end
    ///   somewhere this writer cannot compute, and `end` is deliberately not
    ///   advanced.
    pub fn append(&mut self, block: EncodedBlock) -> Result<SealedBlock> {
        self.check_usable()?;
        if block.seq != self.next_append_seq {
            return Err(ArchiveError::OutOfOrderBlock {
                expected: self.next_append_seq,
                got: block.seq,
            });
        }
        let block_offset = self.end;
        if let Err(error) = self.write_block_bytes(&block.bytes) {
            self.poisoned = true;
            return Err(ArchiveError::Io(error));
        }
        self.end += block.bytes.len() as u64;
        self.next_append_seq += 1;
        Ok(SealedBlock {
            block_offset,
            raw_len: block.raw_len,
            comp_len: u32::try_from(block.comp_len()).expect("a compressed block fits a u32"),
        })
    }

    fn write_block_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        if let Some(after) = self.fail_write_after.take() {
            // A real partial write, not a simulated one: the prefix reaches
            // the real file, so the test that follows is reading the same
            // torn tail a dying process would leave.
            self.file.write_all(&bytes[..after.min(bytes.len())])?;
            return Err(io::Error::other("injected short write"));
        }
        self.file.write_all(bytes)
    }

    /// Write only the first `bytes` of the next block, then fail.
    #[cfg(test)]
    pub(crate) fn fail_next_write_after(&mut self, bytes: usize) {
        self.fail_write_after = Some(bytes);
    }

    /// Whether an earlier partial write took this writer out of service.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// The file offset the next appended block will take.
    #[must_use]
    pub fn end(&self) -> u64 {
        self.end
    }

    /// `take_pending` + `encode` + `append`. Returns `None` when nothing was
    /// pending.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::Poisoned`] after a partial write, and whatever
    /// `append` returns otherwise.
    pub fn seal(&mut self) -> Result<Option<SealedBlock>> {
        self.check_usable()?;
        match self.take_pending() {
            None => Ok(None),
            Some(pending) => self.append(pending.encode()).map(Some),
        }
    }

    /// Durability barrier: the appended blocks are on the device when this
    /// returns.
    ///
    /// `sync_data` rather than `sync_all`: a reader needs the bytes and the
    /// file's length, both of which `fdatasync` flushes, and not the mtime,
    /// which is the extra metadata write `fsync` pays for on every call.
    ///
    /// The owner calls this before committing the index rows that name those
    /// blocks -- see `capsem-logger`'s `commit_index_rows`.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }
}

/// The archive path must be the archive, not a pointer at someone else's
/// file. Checked before the open so the refusal names the path rather than
/// surfacing as an `ELOOP` from `O_NOFOLLOW`.
pub(crate) fn refuse_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ArchiveError::Symlink(path.to_path_buf())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
