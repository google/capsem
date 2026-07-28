//! Open file/directory handle table for FUSE sessions.

use std::collections::HashMap;

pub struct DirEntryData {
    pub name: Vec<u8>,
    pub ino: u64,
    pub type_: u32,
}

pub enum OpenHandle {
    File(std::fs::File),
    Dir(Vec<DirEntryData>),
}

const DEFAULT_MAX_HANDLES: usize = 4096;

pub struct FileHandleTable {
    handles: HashMap<u64, OpenHandle>,
    next_fh: u64,
    max_handles: usize,
}

impl FileHandleTable {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_HANDLES)
    }

    pub fn with_limit(max_handles: usize) -> Self {
        Self {
            handles: HashMap::new(),
            next_fh: 1,
            max_handles,
        }
    }

    /// Allocate a new handle. Returns `None` (EMFILE) if at capacity.
    pub fn alloc(&mut self, handle: OpenHandle) -> Option<u64> {
        if self.handles.len() >= self.max_handles {
            return None;
        }
        let fh = self.next_fh;
        self.next_fh += 1;
        self.handles.insert(fh, handle);
        Some(fh)
    }

    pub fn get_file(&mut self, fh: u64) -> Option<&mut std::fs::File> {
        match self.handles.get_mut(&fh)? {
            OpenHandle::File(f) => Some(f),
            _ => None,
        }
    }

    pub fn get_dir(&self, fh: u64) -> Option<&Vec<DirEntryData>> {
        match self.handles.get(&fh)? {
            OpenHandle::Dir(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn remove(&mut self, fh: u64) {
        self.handles.remove(&fh);
    }
}

#[cfg(test)]
mod tests;
