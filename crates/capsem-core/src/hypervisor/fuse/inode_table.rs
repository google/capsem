//! Inode table: maps FUSE inode numbers to host filesystem paths.
//!
//! Handles reference counting (LOOKUP increments, FORGET decrements)
//! and path traversal security (all paths must resolve under root).
//!
//! Security model: path traversal protection uses `canonicalize()` to
//! defend against a malicious guest. TOCTOU analysis and threat model
//! details are documented at `web/docs/src/content/docs/security/virtualization.md`.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

const MAX_CHECKPOINT_INODES: usize = 1_048_576;
const MAX_CHECKPOINT_PATH_BYTES: usize = 4096;
const HOST_FILE_TYPE_MASK: u32 = libc::S_IFMT as _;

pub struct InodeEntry {
    pub host_path: PathBuf,
    pub refcount: u64,
}

pub struct InodeTable {
    entries: HashMap<u64, InodeEntry>,
    next_ino: u64,
    root_canonical: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InodeSnapshot {
    pub(crate) ino: u64,
    pub(crate) relative_path: Vec<u8>,
    pub(crate) refcount: u64,
    pub(crate) device: u64,
    pub(crate) host_inode: u64,
    pub(crate) file_type: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InodeTableSnapshot {
    pub(crate) entries: Vec<InodeSnapshot>,
    pub(crate) next_ino: u64,
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

    pub(crate) fn reopen_path(&self, ino: u64) -> Result<PathBuf> {
        let path = self
            .get(ino)
            .with_context(|| format!("VirtioFS handle references missing inode {ino}"))?;
        let canonical = path
            .canonicalize()
            .with_context(|| format!("canonicalize reopened VirtioFS inode {ino}"))?;
        ensure!(
            canonical.starts_with(&self.root_canonical),
            "reopened inode {ino} is outside VirtioFS root: {}",
            canonical.display()
        );
        Ok(canonical)
    }

    pub fn child_path(&self, parent_ino: u64, name: &[u8]) -> Option<PathBuf> {
        let name_str = valid_child_name(name)?;
        Some(self.entries.get(&parent_ino)?.host_path.join(name_str))
    }

    /// Canonicalize `path` and return it only if it resolves inside the share.
    ///
    /// A symlink inode stores its own in-root path (so readlink et al. operate
    /// on the link, not its target), so any operation that *follows* the link
    /// to a real file must first prove the target stays within the share. Guest
    /// symlinks can point anywhere; without this a `ln -s /etc/passwd escape`
    /// inside the workspace would be followed straight to the host file.
    pub(crate) fn contained_target(&self, path: &Path) -> Option<PathBuf> {
        let canonical = path.canonicalize().ok()?;
        canonical.starts_with(&self.root_canonical).then_some(canonical)
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
            .filter_map(|(&ino, entry)| same_or_descendant(&entry.host_path, old_path).then_some(ino))
            .collect();

        self.entries
            .retain(|ino, entry| moved.contains(ino) || !same_or_descendant(&entry.host_path, new_path));

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

    pub(crate) fn checkpoint(&self, handle_inodes: &HashSet<u64>) -> Result<InodeTableSnapshot> {
        ensure!(
            self.entries.len() <= MAX_CHECKPOINT_INODES,
            "VirtioFS inode checkpoint count exceeds limit"
        );
        let mut entries = Vec::with_capacity(self.entries.len());
        for (&ino, entry) in &self.entries {
            let relative = entry.host_path.strip_prefix(&self.root_canonical).with_context(|| {
                format!(
                    "VirtioFS inode {ino} path is outside VirtioFS root: {}",
                    entry.host_path.display()
                )
            })?;
            let relative_path = relative.as_os_str().as_bytes().to_vec();
            ensure!(
                relative_path.len() <= MAX_CHECKPOINT_PATH_BYTES,
                "VirtioFS inode {ino} checkpoint path exceeds limit"
            );
            let metadata = match std::fs::symlink_metadata(&entry.host_path) {
                Ok(metadata) => metadata,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && ino != 1 && !handle_inodes.contains(&ino) =>
                {
                    tracing::debug!(
                        inode = ino,
                        path = %entry.host_path.display(),
                        "omitting unlinked cache-only VirtioFS inode from checkpoint"
                    );
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("VirtioFS inode {ino} is not reopenable: {}", entry.host_path.display())
                    });
                }
            };
            entries.push(InodeSnapshot {
                ino,
                relative_path,
                refcount: entry.refcount,
                device: metadata.dev(),
                host_inode: metadata.ino(),
                file_type: metadata.mode() & HOST_FILE_TYPE_MASK,
            });
        }
        entries.sort_by_key(|entry| entry.ino);
        Ok(InodeTableSnapshot {
            entries,
            next_ino: self.next_ino,
        })
    }

    pub(crate) fn restore(root_path: &Path, snapshot: &InodeTableSnapshot) -> Result<Self> {
        ensure!(
            !snapshot.entries.is_empty() && snapshot.entries.len() <= MAX_CHECKPOINT_INODES,
            "invalid VirtioFS inode checkpoint count: {}",
            snapshot.entries.len()
        );
        let root_canonical = root_path
            .canonicalize()
            .with_context(|| format!("canonicalize VirtioFS root: {}", root_path.display()))?;
        let mut entries = HashMap::with_capacity(snapshot.entries.len());
        let mut paths = HashSet::with_capacity(snapshot.entries.len());
        let mut max_ino = 0u64;
        for entry in &snapshot.entries {
            ensure!(entry.ino != 0, "VirtioFS inode checkpoint contains inode zero");
            ensure!(entry.refcount != 0, "VirtioFS inode {} has zero refcount", entry.ino);
            ensure!(
                entry.relative_path.len() <= MAX_CHECKPOINT_PATH_BYTES,
                "VirtioFS inode {} checkpoint path exceeds limit",
                entry.ino
            );
            let host_path = restore_path(&root_canonical, &entry.relative_path)
                .with_context(|| format!("restore VirtioFS inode {}", entry.ino))?;
            let metadata = std::fs::symlink_metadata(&host_path)
                .with_context(|| format!("restored VirtioFS inode {} is missing", entry.ino))?;
            ensure!(
                metadata.dev() == entry.device
                    && metadata.ino() == entry.host_inode
                    && (metadata.mode() & HOST_FILE_TYPE_MASK) == entry.file_type,
                "restored VirtioFS inode {} identity changed",
                entry.ino
            );
            ensure!(
                paths.insert(host_path.clone()),
                "VirtioFS inode checkpoint contains duplicate path: {}",
                host_path.display()
            );
            if entries
                .insert(
                    entry.ino,
                    InodeEntry {
                        host_path,
                        refcount: entry.refcount,
                    },
                )
                .is_some()
            {
                bail!("VirtioFS inode checkpoint contains duplicate inode {}", entry.ino);
            }
            max_ino = max_ino.max(entry.ino);
        }
        let root = entries
            .get(&1)
            .context("VirtioFS inode checkpoint is missing root inode")?;
        ensure!(
            root.host_path == root_canonical,
            "VirtioFS root inode path does not identify the share root"
        );
        ensure!(
            snapshot.next_ino > max_ino && snapshot.next_ino < u64::MAX,
            "invalid VirtioFS next inode id: {}",
            snapshot.next_ino
        );
        Ok(Self {
            entries,
            next_ino: snapshot.next_ino,
            root_canonical,
        })
    }
}

fn restore_path(root: &Path, relative_bytes: &[u8]) -> Result<PathBuf> {
    if relative_bytes.is_empty() {
        return Ok(root.to_path_buf());
    }
    let relative = PathBuf::from(OsString::from_vec(relative_bytes.to_vec()));
    ensure!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "checkpoint path contains a non-child component"
    );

    let components: Vec<_> = relative.components().collect();
    let mut resolved = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            unreachable!("components validated above")
        };
        let candidate = resolved.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() && index + 1 == components.len() => {
                resolved = candidate;
            }
            Ok(_) => {
                let canonical = candidate
                    .canonicalize()
                    .with_context(|| format!("canonicalize restored VirtioFS path: {}", candidate.display()))?;
                ensure!(
                    canonical.starts_with(root),
                    "restored path is outside VirtioFS root: {}",
                    canonical.display()
                );
                resolved = canonical;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                resolved = candidate;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("inspect restored VirtioFS path: {}", candidate.display()))
            }
        }
    }
    ensure!(
        resolved.starts_with(root),
        "restored path is outside VirtioFS root: {}",
        resolved.display()
    );
    Ok(resolved)
}

fn valid_child_name(name: &[u8]) -> Option<&str> {
    let name_str = std::str::from_utf8(name).ok()?;
    if name_str.is_empty() || name_str == "." || name_str == ".." || name_str.contains('/') || name_str.contains('\0') {
        return None;
    }
    Some(name_str)
}

fn same_or_descendant(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.strip_prefix(prefix).is_ok()
}

#[cfg(test)]
mod tests;
