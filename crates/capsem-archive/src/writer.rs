//! Stage bodies into a pending block; seal and append when full or asked.
//!
//! Sealing is two-phase so the owner can deflate off its own thread:
//! `take_pending` hands out the raw block, `PendingBlock::encode` deflates
//! it anywhere, `append` writes the encoded bytes back in order. `seal` is
//! the three in sequence for owners that do not care.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, TARGET_BLOCK_BYTES};
use crate::{ArchiveError, Result};

/// One block's raw bytes, detached from the writer so they can be deflated
/// anywhere. `Send`, deliberately: compression is the expensive half and does
/// not belong on the thread that owns the SQLite connection.
pub struct PendingBlock {
    raw: Vec<u8>,
}

/// A deflated block, ready to be appended. Holds the complete on-disk bytes:
/// block header followed by the compressed payload.
pub struct EncodedBlock {
    raw_len: u32,
    bytes: Vec<u8>,
}

/// Where a block landed. The owner stamps `block_offset` into the index rows
/// of every body the block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealedBlock {
    pub block_offset: u64,
    pub raw_len: u32,
    pub comp_len: u32,
}

pub struct BodyLogWriter {
    file: File,
    end: u64,
    pending: Vec<u8>,
}

impl PendingBlock {
    #[must_use]
    pub fn encode(self) -> EncodedBlock {
        let raw_len = u32::try_from(self.raw.len()).expect("a pending block fits a u32");
        EncodedBlock {
            raw_len,
            bytes: format::encode_block(&self.raw),
        }
    }

    #[must_use]
    pub fn raw_len(&self) -> usize {
        self.raw.len()
    }
}

impl EncodedBlock {
    #[must_use]
    pub fn raw_len(&self) -> u32 {
        self.raw_len
    }

    #[must_use]
    pub fn comp_len(&self) -> usize {
        self.bytes.len() - BLOCK_HEADER_BYTES
    }
}

impl BodyLogWriter {
    /// Create or reopen `session.bodies`.
    ///
    /// Refuses a symlink and opens owner-only without following links through
    /// `capsem-foundation::unix::fs`: the archive holds request and response
    /// bodies, which are the most sensitive bytes a session produces.
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
            return Ok(Self {
                file,
                end: FILE_HEADER_BYTES as u64,
                pending: Vec::new(),
            });
        }
        let mut header = [0u8; FILE_HEADER_BYTES];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header).map_err(|_| ArchiveError::BadFileHeader)?;
        format::decode_file_header(&header)?;
        Ok(Self {
            file,
            end,
            pending: Vec::new(),
        })
    }

    /// Append `body` to the pending block.
    ///
    /// The returned reference has a placeholder `block_offset` of 0; the
    /// caller sets it from the next `SealedBlock`, which the logger does
    /// inside one SQLite transaction so no index row is ever visible with a
    /// placeholder in it.
    ///
    /// # Panics
    ///
    /// If the pending block would exceed `u32::MAX` bytes. Owners seal at
    /// `TARGET_BLOCK_BYTES`, four orders of magnitude below that.
    pub fn stage(&mut self, body: &[u8]) -> BodyRef {
        let offset = u32::try_from(self.pending.len()).expect("a pending block fits a u32");
        let len = u32::try_from(body.len()).expect("a body fits a u32");
        self.pending.extend_from_slice(body);
        BodyRef {
            block_offset: 0,
            offset,
            len,
        }
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

    /// Take the pending raw block out (`None` when empty); the writer keeps
    /// accepting `stage` calls into a fresh block meanwhile. Offsets handed
    /// out by `stage` before this call belong to the taken block.
    pub fn take_pending(&mut self) -> Option<PendingBlock> {
        if self.pending.is_empty() {
            return None;
        }
        Some(PendingBlock {
            raw: std::mem::take(&mut self.pending),
        })
    }

    /// Append an encoded block. Blocks must be appended in the order they
    /// were taken; the caller -- one owner thread -- guarantees it.
    pub fn append(&mut self, block: EncodedBlock) -> Result<SealedBlock> {
        let block_offset = self.end;
        self.file.write_all(&block.bytes)?;
        self.end += block.bytes.len() as u64;
        Ok(SealedBlock {
            block_offset,
            raw_len: block.raw_len,
            comp_len: u32::try_from(block.comp_len()).expect("a compressed block fits a u32"),
        })
    }

    /// `take_pending` + `encode` + `append`. Returns `None` when nothing was
    /// pending.
    pub fn seal(&mut self) -> Result<Option<SealedBlock>> {
        match self.take_pending() {
            None => Ok(None),
            Some(pending) => self.append(pending.encode()).map(Some),
        }
    }

    /// Durability barrier for session close.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
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
