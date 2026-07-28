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

use anyhow::{Context, Result};

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
        Self {
            entries: HashMap::new(),
            next_ino: 2,
            root_canonical: PathBuf::new(),
        }
    }

    pub fn new(root_path: &Path) -> Result<Self> {
        let root_canonical = root_path
            .canonicalize()
            .with_context(|| format!("canonicalize VirtioFS root: {}", root_path.display()))?;
        let mut entries = HashMap::new();
        entries.insert(
            1,
            InodeEntry {
                host_path: root_canonical.clone(),
                refcount: u64::MAX / 2,
            },
        );
        Ok(Self {
            entries,
            next_ino: 2,
            root_canonical,
        })
    }

    pub fn get(&self, ino: u64) -> Option<&PathBuf> {
        self.entries.get(&ino).map(|e| &e.host_path)
    }

    pub fn child_path(&self, parent_ino: u64, name: &[u8]) -> Option<PathBuf> {
        let name_str = valid_child_name(name)?;
        Some(self.entries.get(&parent_ino)?.host_path.join(name_str))
    }

    /// Resolve a child name under a parent inode. Returns inode number.
    /// Validates path traversal security: the resolved path must be under root.
    pub fn lookup(&mut self, parent_ino: u64, name: &[u8]) -> Option<u64> {
        let name_str = valid_child_name(name)?;

        let parent_path = self.entries.get(&parent_ino)?.host_path.clone();
        let child_path = parent_path.join(name_str);
        let meta = std::fs::symlink_metadata(&child_path).ok()?;
        let entry_path = if meta.file_type().is_symlink() {
            child_path
        } else {
            let canonical = child_path.canonicalize().ok()?;
            if !canonical.starts_with(&self.root_canonical) {
                return None;
            }
            canonical
        };

        for (&ino, entry) in &self.entries {
            if entry.host_path == entry_path {
                if let Some(e) = self.entries.get_mut(&ino) {
                    e.refcount = e.refcount.saturating_add(1);
                }
                return Some(ino);
            }
        }

        let ino = self.next_ino;
        self.next_ino += 1;
        self.entries.insert(
            ino,
            InodeEntry {
                host_path: entry_path,
                refcount: 1,
            },
        );
        Some(ino)
    }

    pub fn forget(&mut self, ino: u64, nlookup: u64) {
        if ino <= 1 {
            return;
        }
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

    pub fn rename_path(&mut self, old_path: &Path, new_path: &Path) {
        let moved: Vec<u64> = self
            .entries
            .iter()
            .filter_map(|(&ino, entry)| {
                same_or_descendant(&entry.host_path, old_path).then_some(ino)
            })
            .collect();

        self.entries.retain(|ino, entry| {
            moved.contains(ino) || !same_or_descendant(&entry.host_path, new_path)
        });

        for ino in moved {
            if let Some(entry) = self.entries.get_mut(&ino) {
                if let Ok(suffix) = entry.host_path.strip_prefix(old_path) {
                    entry.host_path = if suffix.as_os_str().is_empty() {
                        new_path.to_path_buf()
                    } else {
                        new_path.join(suffix)
                    };
                }
            }
        }
    }
}

fn valid_child_name(name: &[u8]) -> Option<&str> {
    let name_str = std::str::from_utf8(name).ok()?;
    if name_str.is_empty()
        || name_str == "."
        || name_str == ".."
        || name_str.contains('/')
        || name_str.contains('\0')
    {
        return None;
    }
    Some(name_str)
}

fn same_or_descendant(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.strip_prefix(prefix).is_ok()
}

#[cfg(test)]
mod tests;
