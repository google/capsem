//! Inode table: maps FUSE inode numbers to host filesystem paths.
//!
//! Handles reference counting (LOOKUP increments, FORGET decrements)
//! and path traversal security (all paths must resolve under root).
//!
//! Security model: path traversal protection uses `canonicalize()` to
//! defend against a malicious guest. TOCTOU analysis and threat model
//! details are documented at `site/src/content/docs/security/virtualization.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, Context};

pub struct InodeEntry {
    pub host_path: PathBuf,
    pub refcount: u64,
}

pub struct InodeTable {
    entries: HashMap<u64, InodeEntry>,
    next_ino: u64,
    root_canonical: PathBuf,
}

impl InodeTable {
    /// Create an empty sentinel table (placeholder after state transfer to worker).
    pub fn empty() -> Self {
        Self { entries: HashMap::new(), next_ino: 2, root_canonical: PathBuf::new() }
    }

    pub fn new(root_path: &Path) -> Result<Self> {
        let root_canonical = root_path.canonicalize()
            .with_context(|| format!("canonicalize VirtioFS root: {}", root_path.display()))?;
        let mut entries = HashMap::new();
        entries.insert(1, InodeEntry {
            host_path: root_canonical.clone(),
            refcount: u64::MAX / 2,
        });
        Ok(Self { entries, next_ino: 2, root_canonical })
    }

    pub fn get(&self, ino: u64) -> Option<&PathBuf> {
        self.entries.get(&ino).map(|e| &e.host_path)
    }

    /// Resolve a child name under a parent inode. Returns inode number.
    /// Validates path traversal security: the resolved path must be under root.
    pub fn lookup(&mut self, parent_ino: u64, name: &[u8]) -> Option<u64> {
        let name_str = std::str::from_utf8(name).ok()?;

        if name_str.is_empty()
            || name_str == "."
            || name_str == ".."
            || name_str.contains('/')
            || name_str.contains('\0')
        {
            return None;
        }

        let parent_path = self.entries.get(&parent_ino)?.host_path.clone();
        let child_path = parent_path.join(name_str);
        let canonical = child_path.canonicalize().ok()?;
        if !canonical.starts_with(&self.root_canonical) {
            return None;
        }

        for (&ino, entry) in &self.entries {
            if entry.host_path == canonical {
                if let Some(e) = self.entries.get_mut(&ino) {
                    e.refcount = e.refcount.saturating_add(1);
                }
                return Some(ino);
            }
        }

        let ino = self.next_ino;
        self.next_ino += 1;
        self.entries.insert(ino, InodeEntry {
            host_path: canonical,
            refcount: 1,
        });
        Some(ino)
    }

    pub fn forget(&mut self, ino: u64, nlookup: u64) {
        if ino <= 1 { return; }
        let remove = if let Some(entry) = self.entries.get_mut(&ino) {
            entry.refcount = entry.refcount.saturating_sub(nlookup);
            entry.refcount == 0
        } else {
            false
        };
        if remove {
            self.entries.remove(&ino);
        }
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

    // Inode table operations

    #[test]
    fn root_is_1() {
        let dir = temp_share("inode-root");
        let table = InodeTable::new(&dir).unwrap();
        assert!(table.get(1).is_some());
    }

    #[test]
    fn lookup_creates_inode() {
        let dir = temp_share("inode-lookup");
        std::fs::write(dir.join("hello.txt"), b"world").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let ino = table.lookup(1, b"hello.txt").unwrap();
        assert!(ino >= 2);
        assert!(table.get(ino).is_some());
    }

    #[test]
    fn lookup_same_name_same_inode() {
        let dir = temp_share("inode-same");
        std::fs::write(dir.join("file.txt"), b"data").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let ino1 = table.lookup(1, b"file.txt").unwrap();
        let ino2 = table.lookup(1, b"file.txt").unwrap();
        assert_eq!(ino1, ino2);
    }

    #[test]
    fn lookup_bumps_refcount() {
        let dir = temp_share("inode-refcount");
        std::fs::write(dir.join("file.txt"), b"data").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let ino = table.lookup(1, b"file.txt").unwrap();
        table.lookup(1, b"file.txt").unwrap();
        table.forget(ino, 1);
        assert!(table.get(ino).is_some());
        table.forget(ino, 1);
        assert!(table.get(ino).is_none());
    }

    #[test]
    fn forget_removes_at_zero() {
        let dir = temp_share("inode-forget");
        std::fs::write(dir.join("file.txt"), b"data").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let ino = table.lookup(1, b"file.txt").unwrap();
        table.forget(ino, 1);
        assert!(table.get(ino).is_none());
    }

    #[test]
    fn forget_root_noop() {
        let dir = temp_share("inode-forget-root");
        let mut table = InodeTable::new(&dir).unwrap();
        table.forget(1, u64::MAX);
        assert!(table.get(1).is_some());
    }

    #[test]
    fn forget_saturates() {
        let dir = temp_share("inode-saturate");
        std::fs::write(dir.join("file.txt"), b"data").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let ino = table.lookup(1, b"file.txt").unwrap();
        table.forget(ino, 100);
        assert!(table.get(ino).is_none());
    }

    #[test]
    fn nonexistent_returns_none() {
        let dir = temp_share("inode-noent");
        let mut table = InodeTable::new(&dir).unwrap();
        assert!(table.lookup(1, b"nonexistent.txt").is_none());
    }

    // Path traversal security (adversarial)

    #[test] fn rejects_dotdot() {
        let dir = temp_share("path-dotdot");
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"..").is_none());
    }

    #[test] fn rejects_dot() {
        let dir = temp_share("path-dot");
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b".").is_none());
    }

    #[test] fn rejects_slash() {
        let dir = temp_share("path-slash");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/file.txt"), b"data").unwrap();
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"sub/file.txt").is_none());
    }

    #[test] fn rejects_null() {
        let dir = temp_share("path-null");
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"file\0.txt").is_none());
    }

    #[test] fn rejects_empty() {
        let dir = temp_share("path-empty");
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"").is_none());
    }

    #[test] fn rejects_symlink_escape() {
        let dir = temp_share("path-symlink-escape");
        std::os::unix::fs::symlink("/etc/passwd", dir.join("escape")).unwrap();
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"escape").is_none());
    }

    #[test] fn rejects_symlink_chain_escape() {
        let dir = temp_share("path-chain-escape");
        std::os::unix::fs::symlink("/tmp", dir.join("link")).unwrap();
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"link").is_none());
    }

    #[test] fn allows_regular_file() {
        let dir = temp_share("path-regular");
        std::fs::write(dir.join("ok.txt"), b"fine").unwrap();
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b"ok.txt").is_some());
    }

    #[test] fn allows_dotfile() {
        let dir = temp_share("path-dotfile");
        std::fs::write(dir.join(".hidden"), b"secret").unwrap();
        assert!(InodeTable::new(&dir).unwrap().lookup(1, b".hidden").is_some());
    }

    #[test]
    fn allows_subdirectory_traversal() {
        let dir = temp_share("path-subdir");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/file.txt"), b"data").unwrap();
        let mut table = InodeTable::new(&dir).unwrap();
        let sub_ino = table.lookup(1, b"sub").unwrap();
        assert!(table.lookup(sub_ino, b"file.txt").is_some());
    }
}
