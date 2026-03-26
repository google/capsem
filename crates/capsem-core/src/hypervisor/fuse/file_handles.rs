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
        Self { handles: HashMap::new(), next_fh: 1, max_handles }
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_share(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("capsem-fuse-test").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn alloc_and_remove() {
        let dir = temp_share("fh-alloc");
        std::fs::write(dir.join("f.txt"), b"x").unwrap();
        let file = std::fs::File::open(dir.join("f.txt")).unwrap();
        let mut fht = FileHandleTable::new();
        let fh = fht.alloc(OpenHandle::File(file)).unwrap();
        assert!(fht.get_file(fh).is_some());
        fht.remove(fh);
        assert!(fht.get_file(fh).is_none());
    }

    #[test]
    fn sequential_ids() {
        let dir = temp_share("fh-seq");
        std::fs::write(dir.join("a"), b"").unwrap();
        std::fs::write(dir.join("b"), b"").unwrap();
        let mut fht = FileHandleTable::new();
        let fh1 = fht.alloc(OpenHandle::File(std::fs::File::open(dir.join("a")).unwrap())).unwrap();
        let fh2 = fht.alloc(OpenHandle::File(std::fs::File::open(dir.join("b")).unwrap())).unwrap();
        assert_eq!(fh2, fh1 + 1);
    }

    #[test]
    fn alloc_respects_limit() {
        let mut fht = FileHandleTable::with_limit(2);
        assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
        assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
        assert!(fht.alloc(OpenHandle::Dir(vec![])).is_none());
    }

    #[test]
    fn alloc_after_remove_under_limit() {
        let mut fht = FileHandleTable::with_limit(1);
        let fh = fht.alloc(OpenHandle::Dir(vec![])).unwrap();
        fht.remove(fh);
        assert!(fht.alloc(OpenHandle::Dir(vec![])).is_some());
    }
}
