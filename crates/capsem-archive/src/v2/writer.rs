//! Stage bodies into an open block; flush a segment when asked, close the
//! block when it is full.
//!
//! A block is one deflate stream. `stage` feeds a body to the compressor and
//! hands back a reference naming the block's real offset; `flush_segment`
//! sync-flushes the compressor and appends everything it produced since the
//! last flush, behind a segment header, in one write. The block stays open,
//! so the next segment compresses against the same dictionary. `close_block`
//! finishes the stream with a FINAL segment.
//!
//! Nothing a reader can be told about is ever rewritten: a segment is
//! appended once, whole, and the logger commits the index rows that name it
//! only after the segment is on disk. A write that fails part-way through a
//! segment poisons the writer: the file's end is no longer provably where it
//! was, so every later offset would be a guess, and guessing is how an index
//! comes to point at the wrong bytes.
//!
//! A reopened writer never continues a block it finds in the file. It has no
//! compressor state for it, and the bytes past the last committed segment may
//! be a torn tail. It starts a new block at the end of the file instead; the
//! old block stays readable up to whatever its index committed.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use capsem_foundation::unix::fs as unix_fs;
use flate2::{Compress, Compression, FlushCompress, Status};

use super::format::{
    self, BodyRef, SegmentHeader, BLOCK_HEADER_BYTES, CODEC_DEFLATE, DEFLATE_LEVEL, FILE_HEADER_BYTES,
    MAX_BLOCK_RAW_BYTES, SEGMENT_HEADER_BYTES, TARGET_BLOCK_BYTES,
};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// What one flush or close put on disk. The owner records `raw_len` and
/// `disk_len` against `block_offset` in the same transaction as the index
/// rows the segment covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentWritten {
    pub block_offset: u64,
    /// Raw bytes of the block now on disk, across every segment so far.
    pub raw_len: u32,
    /// Bytes the block occupies in the file: its header and every segment.
    pub disk_len: u64,
    /// The segment was FINAL: the block is closed and nothing more joins it.
    pub closed: bool,
}

/// The block being written: one compressor, and the segment in progress.
struct OpenBlock {
    offset: u64,
    compressor: Compress,
    /// Raw bytes staged into the block, flushed or not.
    raw_len: u32,
    /// Raw bytes already inside a written segment.
    flushed_raw: u32,
    /// Bytes of the block already in the file. Zero until the first segment,
    /// which carries the block header with it.
    disk_len: u64,
    /// blake3 of the raw bytes of the segment in progress.
    hasher: blake3::Hasher,
    /// Compressed bytes of the segment in progress.
    out: Vec<u8>,
}

/// Appends blocks to one `session.bodies`.
///
/// Dropping a writer with staged, unflushed bodies discards them: they were
/// never written, and their index rows were never committed, so the ledger
/// stays consistent -- but the bodies are gone. Owners flush before dropping,
/// and a debug build asserts they did. An open block whose every body was
/// flushed may be dropped: it is readable to the extent that was written.
pub struct BodyLogWriter {
    file: File,
    end: u64,
    block: Option<OpenBlock>,
    /// Set by a write that failed part-way through. See
    /// [`ArchiveError::Poisoned`].
    poisoned: bool,
    /// Write this many bytes of the next segment and then fail, producing a
    /// genuine torn tail on the real file rather than a simulated one.
    #[cfg(test)]
    fail_write_after: Option<usize>,
}

impl Drop for BodyLogWriter {
    fn drop(&mut self) {
        // Not while unwinding, and not when poisoned: a poisoned writer can
        // place nothing, so flushing first is not something the owner could
        // have done.
        debug_assert!(
            std::thread::panicking() || self.poisoned || self.pending_bytes() == 0,
            "BodyLogWriter dropped with {} bytes of unflushed bodies; flush before dropping",
            self.pending_bytes()
        );
    }
}

impl BodyLogWriter {
    /// Create or reopen `session.bodies`.
    ///
    /// Creates the file with mode 0o600, refuses a symlink, and opens without
    /// following links: the archive holds request and response bodies, the
    /// most sensitive bytes a session produces. A reopened file has its mode
    /// set back to 0o600 through the open handle, so a widened mode does not
    /// keep serving group and other for the rest of the session.
    ///
    /// A reopened file must carry this version's header. It is appended after
    /// its current end, torn tail included: no index row points there.
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
            block: None,
            poisoned: false,
            #[cfg(test)]
            fail_write_after: None,
        }
    }

    /// Feed `body` to the open block, opening one at the end of the file if
    /// none is open, and return where it will be once flushed.
    ///
    /// The reference names the block's real offset. It is not readable until
    /// the segment holding it is flushed and synced, which is why the owner
    /// holds the index row until then.
    ///
    /// # Errors
    ///
    /// - [`ArchiveError::BodyTooLarge`] when the body alone exceeds
    ///   `MAX_BLOCK_RAW_BYTES`. No block could hold it.
    /// - [`ArchiveError::BlockFull`] when it would take the open block past
    ///   that ceiling. Close the block and stage it again; it then fits.
    /// - [`ArchiveError::Poisoned`] after a partial write.
    ///
    /// Nothing here panics: a body arrives from the network, and a panic on
    /// the writer thread would take the whole session ledger down.
    pub fn stage(&mut self, body: &[u8]) -> Result<BodyRef> {
        self.check_usable()?;
        if body.len() > MAX_BLOCK_RAW_BYTES {
            return Err(ArchiveError::BodyTooLarge {
                len: body.len(),
                max: MAX_BLOCK_RAW_BYTES,
            });
        }
        if self
            .block
            .as_ref()
            .is_some_and(|block| block.raw_len as usize + body.len() > MAX_BLOCK_RAW_BYTES)
        {
            return Err(ArchiveError::BlockFull);
        }
        let end = self.end;
        let block = self.block.get_or_insert_with(|| OpenBlock {
            offset: end,
            compressor: Compress::new(Compression::new(DEFLATE_LEVEL), false),
            raw_len: 0,
            flushed_raw: 0,
            disk_len: 0,
            hasher: blake3::Hasher::new(),
            out: Vec::new(),
        });
        if let Err(error) = deflate_into(&mut block.compressor, body, &mut block.out, FlushCompress::None) {
            // The compressor may have taken part of the body; nothing it
            // produces from here on can be vouched for.
            self.poisoned = true;
            return Err(error);
        }
        block.hasher.update(body);
        // Both bounded by MAX_BLOCK_RAW_BYTES (16 MiB) above.
        let offset = block.raw_len;
        let len = u32::try_from(body.len()).expect("bounded by MAX_BLOCK_RAW_BYTES");
        block.raw_len += len;
        Ok(BodyRef {
            block_offset: block.offset,
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

    /// Raw bytes staged but not yet inside a written segment.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.block
            .as_ref()
            .map_or(0, |block| (block.raw_len - block.flushed_raw) as usize)
    }

    /// Raw bytes in the open block, flushed or not; `None` when no block is
    /// open.
    #[must_use]
    pub fn open_block_raw_len(&self) -> Option<usize> {
        self.block.as_ref().map(|block| block.raw_len as usize)
    }

    /// True once the open block has reached its target size.
    #[must_use]
    pub fn wants_close(&self) -> bool {
        self.open_block_raw_len()
            .is_some_and(|raw_len| raw_len >= TARGET_BLOCK_BYTES)
    }

    /// Sync-flush the open block and append what it produced as one segment.
    /// `None` when nothing was staged since the last flush: an empty segment
    /// is never written.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::Poisoned`], or [`ArchiveError::Io`] from the write,
    /// which poisons the writer.
    pub fn flush_segment(&mut self) -> Result<Option<SegmentWritten>> {
        self.check_usable()?;
        if self.pending_bytes() == 0 {
            return Ok(None);
        }
        self.write_segment(false).map(Some)
    }

    /// End the open block's stream with a FINAL segment, carrying whatever
    /// was staged since the last flush. `None` when no block is open.
    ///
    /// # Errors
    ///
    /// As [`BodyLogWriter::flush_segment`].
    pub fn close_block(&mut self) -> Result<Option<SegmentWritten>> {
        self.check_usable()?;
        if self.block.is_none() {
            return Ok(None);
        }
        let written = self.write_segment(true)?;
        self.block = None;
        Ok(Some(written))
    }

    fn write_segment(&mut self, last: bool) -> Result<SegmentWritten> {
        #[cfg(test)]
        let fail_after = self.fail_write_after.take();
        #[cfg(not(test))]
        let fail_after = None;
        let block = self.block.as_mut().expect("callers check a block is open");
        let flush = if last {
            FlushCompress::Finish
        } else {
            FlushCompress::Sync
        };
        if let Err(error) = deflate_into(&mut block.compressor, &[], &mut block.out, flush) {
            // The compressor's state is unknown; nothing it produces next can
            // be vouched for.
            self.poisoned = true;
            return Err(error);
        }
        let comp_len = u32::try_from(block.out.len()).expect("a segment's compressed size fits a u32");
        let header = format::encode_segment_header(&SegmentHeader {
            last,
            raw_start: block.flushed_raw,
            raw_len: block.raw_len - block.flushed_raw,
            comp_len,
            hash: *block.hasher.finalize().as_bytes(),
        });
        let mut bytes = Vec::with_capacity(BLOCK_HEADER_BYTES + SEGMENT_HEADER_BYTES + block.out.len());
        if block.disk_len == 0 {
            bytes.extend_from_slice(&format::encode_block_header(CODEC_DEFLATE));
        }
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&block.out);
        if let Err(error) = write_all(&mut self.file, &bytes, fail_after) {
            self.poisoned = true;
            return Err(ArchiveError::Io(error));
        }
        self.end += bytes.len() as u64;
        block.disk_len += bytes.len() as u64;
        block.flushed_raw = block.raw_len;
        block.hasher = blake3::Hasher::new();
        block.out.clear();
        Ok(SegmentWritten {
            block_offset: block.offset,
            raw_len: block.raw_len,
            disk_len: block.disk_len,
            closed: last,
        })
    }

    /// Write only the first `bytes` of the next segment, then fail.
    #[cfg(test)]
    pub(crate) fn fail_next_write_after(&mut self, bytes: usize) {
        self.fail_write_after = Some(bytes);
    }

    /// Take this writer out of service on purpose, discarding whatever was
    /// staged and not yet flushed.
    ///
    /// For an owner that stops archiving because of a failure of its own --
    /// an index it could not commit, an injected fault -- rather than one this
    /// writer reported. Those bodies had no committed row, so discarding them
    /// is the documented cost; what this changes is that the owner says so,
    /// rather than dropping a writer that asserts it was not dropped holding
    /// bodies. The file is left exactly as the last written segment left it.
    pub fn abandon(mut self) {
        self.poisoned = true;
    }

    /// Whether an earlier partial write took this writer out of service.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// The file offset the next segment will be written at.
    #[must_use]
    pub fn end(&self) -> u64 {
        self.end
    }

    /// Durability barrier: every written segment is on the device when this
    /// returns. `sync_data` rather than `sync_all`: a reader needs the bytes
    /// and the length, not the mtime. The owner calls this before committing
    /// the index rows that name those segments.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }
}

/// One `write_all`, or -- in a test that asked for it; `None` outside tests --
/// a real partial write
/// of the first `fail_after` bytes followed by an error, so the torn tail a
/// test reads is the one a dying process would leave.
fn write_all(file: &mut File, bytes: &[u8], fail_after: Option<usize>) -> io::Result<()> {
    if let Some(after) = fail_after {
        file.write_all(&bytes[..after.min(bytes.len())])?;
        return Err(io::Error::other("injected short write"));
    }
    file.write_all(bytes)
}

/// Run `input` through the compressor with `flush`, growing `out` until the
/// compressor has taken all of it and emitted everything the flush owes.
fn deflate_into(compressor: &mut Compress, input: &[u8], out: &mut Vec<u8>, flush: FlushCompress) -> Result<()> {
    let mut consumed = 0;
    loop {
        // Room for the input at worst-case expansion plus the flush's own
        // bytes, so a sync flush almost always completes in one call.
        out.reserve((input.len() - consumed) + (input.len() - consumed) / 1000 + 64 * 1024);
        let before = compressor.total_in();
        let status = compressor
            .compress_vec(&input[consumed..], out, flush)
            .map_err(|error| ArchiveError::Io(io::Error::other(error)))?;
        consumed += usize::try_from(compressor.total_in() - before).expect("bounded by the input");
        let done = match flush {
            FlushCompress::Finish => status == Status::StreamEnd,
            // Output stopped short of the buffer's end: the flush is complete.
            _ => consumed == input.len() && out.len() < out.capacity(),
        };
        if done {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
