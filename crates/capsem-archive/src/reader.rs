//! Resolve a `BodyRef` to bytes, inflating a block segment by segment.
//!
//! The SQLite index is the only source of truth for what exists: a reader
//! never scans the file. It seeks to the block an index row names and walks
//! that block's segments, bounding every header before it allocates and
//! verifying blake3 over each segment's inflated bytes before returning any of
//! them. It stops at the segment that ends the requested span, so it never
//! reads past what the writer had committed when that row became visible --
//! which is what makes reading a block the writer is still appending to safe
//! from another process.
//!
//! **Integrity here is per segment, not per body.** The hash proves a
//! segment's bytes are the bytes that were written; it says nothing about
//! which span of them a given body is. An index row whose `offset` and `len`
//! were edited selects a different, still-valid span, and this reader returns
//! it. The row is the claim, the segment is the evidence, and SQLite is where
//! the claim is protected -- the logger verifies each body against the hash
//! its row records.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use capsem_foundation::unix::fs as unix_fs;
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

use super::format::{
    self, BodyRef, SegmentHeader, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, MAX_BLOCK_RAW_BYTES, SEGMENT_HEADER_BYTES,
    SYNC_FLUSH_TAIL,
};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

/// Reads bodies back out of one `session.bodies`.
///
/// One per thread: the file handle and the block cursor live behind
/// `RefCell`, so this is `Send` but not `Sync`.
pub struct BodyLogReader {
    file: RefCell<File>,
    /// The path this reader was opened on, and the file that was there at the
    /// time. Retention replaces the archive by renaming a compacted copy over
    /// it -- see `file_was_replaced`.
    path: PathBuf,
    identity: FileIdentity,
    /// How far into one block this reader has inflated. Bodies of one
    /// exchange land in one block, and a UI walks rows in order, so the next
    /// read usually continues this cursor rather than starting over.
    cursor: RefCell<Option<Cursor>>,
    blocks: Cell<u64>,
    segments: Cell<u64>,
}

/// One block, inflated up to the end of some segment.
struct Cursor {
    block_offset: u64,
    /// The block's inflater, carrying the dictionary the next segment was
    /// compressed against. Boxed: it is about 40 KiB.
    inflater: Box<InflateState>,
    /// Every raw byte of the block inflated so far.
    raw: Vec<u8>,
    /// File offset of the next segment header.
    next_segment_at: u64,
    /// The FINAL segment has been inflated; the block has no more bytes.
    finished: bool,
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
            cursor: RefCell::new(None),
            blocks: Cell::new(0),
            segments: Cell::new(0),
        })
    }

    /// Resolve one body reference to owned bytes.
    ///
    /// Owned, not borrowed: a view into the cursor would make the next
    /// `read`, which may replace it, a runtime borrow panic in whichever
    /// caller still held the view.
    pub fn read(&self, reference: BodyRef) -> Result<Vec<u8>> {
        let start = reference.offset as usize;
        let end = start
            .checked_add(reference.len as usize)
            .filter(|end| *end <= MAX_BLOCK_RAW_BYTES)
            .ok_or(ArchiveError::RefOutOfRange)?;
        let mut slot = self.cursor.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|cursor| cursor.block_offset != reference.block_offset)
        {
            *slot = None;
            *slot = Some(self.start_block(reference.block_offset)?);
        }
        let cursor = slot.as_mut().expect("set above");
        if let Err(error) = self.inflate_through(cursor, end) {
            // A cursor that failed part-way holds an inflater in an unknown
            // state and possibly unverified bytes. Nothing may read from it.
            *slot = None;
            return Err(error);
        }
        Ok(cursor.raw[start..end].to_vec())
    }

    fn start_block(&self, block_offset: u64) -> Result<Cursor> {
        let mut head = [0u8; BLOCK_HEADER_BYTES];
        {
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(block_offset))?;
            file.read_exact(&mut head)
                .map_err(|_| ArchiveError::BadBlockHeader(block_offset))?;
        }
        format::parse_block_header(&head, block_offset)?;
        self.blocks.set(self.blocks.get() + 1);
        Ok(Cursor {
            block_offset,
            inflater: InflateState::new_boxed(DataFormat::Raw),
            raw: Vec::new(),
            next_segment_at: block_offset + BLOCK_HEADER_BYTES as u64,
            finished: false,
        })
    }

    /// Inflate segments into `cursor` until its raw bytes reach `end`.
    fn inflate_through(&self, cursor: &mut Cursor, end: usize) -> Result<()> {
        while cursor.raw.len() < end {
            if cursor.finished {
                return Err(ArchiveError::RefOutOfRange);
            }
            self.inflate_next_segment(cursor)?;
        }
        Ok(())
    }

    /// Read, bound, inflate and verify the segment at `next_segment_at`.
    fn inflate_next_segment(&self, cursor: &mut Cursor) -> Result<()> {
        let at = cursor.next_segment_at;
        let (header, comp) = {
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(at))?;
            let mut head = [0u8; SEGMENT_HEADER_BYTES];
            // The file ending here means the reference asked for bytes the
            // block never committed: a forged row, or a truncated file.
            file.read_exact(&mut head)
                .map_err(|_| ArchiveError::TruncatedBlock(cursor.block_offset))?;
            let raw_start = u32::try_from(cursor.raw.len()).expect("bounded by MAX_BLOCK_RAW_BYTES");
            // Bounds `comp_len` before it sizes the buffer below.
            let header = format::parse_segment_header(&head, at, raw_start)?;
            let mut comp = vec![0u8; header.comp_len as usize];
            file.read_exact(&mut comp)
                .map_err(|_| ArchiveError::TruncatedBlock(cursor.block_offset))?;
            (header, comp)
        };
        inflate_segment(cursor, &header, &comp, at)?;
        self.segments.set(self.segments.get() + 1);
        cursor.next_segment_at = at + (SEGMENT_HEADER_BYTES + comp.len()) as u64;
        cursor.finished = header.last;
        Ok(())
    }

    /// Blocks this reader has started inflating from their header.
    #[must_use]
    pub fn blocks_inflated(&self) -> u64 {
        self.blocks.get()
    }

    /// Segments this reader has inflated, across every block.
    #[must_use]
    pub fn segments_inflated(&self) -> u64 {
        self.segments.get()
    }

    /// Whether something replaced the archive since this reader opened it.
    ///
    /// Retention renames a compacted copy over the file, so a reader that was
    /// already open keeps a descriptor on the old, now-nameless inode -- where
    /// every block has moved. The owner asks this before a read and reopens
    /// when it is true. A path that cannot be stat'd counts as replaced.
    #[must_use]
    pub fn file_was_replaced(&self) -> bool {
        std::fs::metadata(&self.path).map_or(true, |metadata| FileIdentity::of(&metadata) != self.identity)
    }
}

/// Inflate one segment's `comp` onto the end of `cursor.raw`, exactly.
///
/// Exactly means all four of: every compressed byte consumed, exactly
/// `raw_len` bytes produced (the output buffer has one byte of slack, so an
/// overlong segment is seen rather than silently cut), the stream ended if
/// and only if the segment is FINAL, and blake3 over the produced bytes equal
/// to the header's. A non-final segment must also end on a sync flush, or the
/// next one would not decode after it.
fn inflate_segment(cursor: &mut Cursor, header: &SegmentHeader, comp: &[u8], at: u64) -> Result<()> {
    let bad = |reason: &str| ArchiveError::Inflate(at, reason.to_string());
    if !header.last && !comp.ends_with(&SYNC_FLUSH_TAIL) {
        return Err(ArchiveError::BadSegment(at));
    }
    let start = cursor.raw.len();
    let want = header.raw_len as usize;
    cursor.raw.resize(start + want + 1, 0);
    let (mut consumed, mut written) = (0, 0);
    let mut ended = false;
    loop {
        let result = inflate(
            &mut cursor.inflater,
            &comp[consumed..],
            &mut cursor.raw[start + written..],
            MZFlush::None,
        );
        consumed += result.bytes_consumed;
        written += result.bytes_written;
        match result.status {
            Ok(MZStatus::StreamEnd) => {
                ended = true;
                break;
            }
            // All input taken, or no progress possible (the slack byte is
            // spent, or the stream wants input the segment does not have):
            // the checks below say which.
            Ok(_) if consumed == comp.len() || (result.bytes_consumed == 0 && result.bytes_written == 0) => break,
            Ok(_) => {}
            Err(miniz_oxide::MZError::Buf) => break,
            Err(error) => return Err(bad(&format!("{error:?}"))),
        }
    }
    cursor.raw.truncate(start + written.min(want));
    if consumed != comp.len() || written != want || ended != header.last {
        return Err(ArchiveError::Integrity(at));
    }
    if blake3::hash(&cursor.raw[start..]).as_bytes() != &header.hash {
        return Err(ArchiveError::Integrity(at));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
