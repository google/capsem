//! Resolve a `BodyRef` to bytes, inflating one block at a time.
//!
//! The SQLite index is the only source of truth for what exists: a reader
//! never scans the file. It seeks to the offset an index row names, bounds
//! every length in the header it finds there before allocating, and verifies
//! blake3 over the inflated bytes before returning any of them.

use std::cell::{Cell, Ref, RefCell};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use capsem_foundation::unix::fs as unix_fs;

use crate::format::{self, BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES};
use crate::writer::refuse_symlink;
use crate::{ArchiveError, Result};

pub struct BodyLogReader {
    file: RefCell<File>,
    /// The last inflated block. Bodies from one exchange land in one block,
    /// so a UI walking a session's rows in order hits this nearly every time.
    last: RefCell<Option<(u64, Vec<u8>)>>,
    inflated: Cell<u64>,
}

impl BodyLogReader {
    pub fn open(path: &Path) -> Result<Self> {
        refuse_symlink(path)?;
        let mut file = unix_fs::open_regular_file_no_follow(path)?;
        let mut header = [0u8; FILE_HEADER_BYTES];
        file.read_exact(&mut header).map_err(|_| ArchiveError::BadFileHeader)?;
        format::decode_file_header(&header)?;
        Ok(Self {
            file: RefCell::new(file),
            last: RefCell::new(None),
            inflated: Cell::new(0),
        })
    }

    pub fn read(&self, reference: BodyRef) -> Result<Vec<u8>> {
        let block = self.block(reference.block_offset)?;
        let start = reference.offset as usize;
        let end = start
            .checked_add(reference.len as usize)
            .ok_or(ArchiveError::RefOutOfRange)?;
        block
            .get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(ArchiveError::RefOutOfRange)
    }

    /// Inflate the whole block at `block_offset` (cached for the last one).
    pub fn block(&self, block_offset: u64) -> Result<Ref<'_, [u8]>> {
        let cached = self
            .last
            .borrow()
            .as_ref()
            .is_some_and(|(offset, _)| *offset == block_offset);
        if !cached {
            let raw = self.inflate(block_offset)?;
            *self.last.borrow_mut() = Some((block_offset, raw));
        }
        Ok(Ref::map(self.last.borrow(), |slot| {
            slot.as_ref().expect("the block was just inflated").1.as_slice()
        }))
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
                .map_err(|_| ArchiveError::BadBlockHeader(block_offset))?;
            (header, comp)
        };
        self.inflated.set(self.inflated.get() + 1);
        format::decode_block(&header, &comp, block_offset)
    }

    #[must_use]
    pub fn blocks_inflated(&self) -> u64 {
        self.inflated.get()
    }
}

#[cfg(test)]
mod tests;
