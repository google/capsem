//! Open file/directory handle table for FUSE sessions.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use anyhow::{bail, ensure, Context, Result};

use super::inode_table::InodeTable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryData {
    pub name: Vec<u8>,
    pub ino: u64,
    pub type_: u32,
}

pub struct OpenFileHandle {
    file: std::fs::File,
    inode: u64,
    readable: bool,
    writable: bool,
    append: bool,
}

pub struct OpenDirHandle {
    inode: u64,
    device: u64,
    host_inode: u64,
    entries: Vec<DirEntryData>,
}

pub enum OpenHandle {
    File(OpenFileHandle),
    Dir(OpenDirHandle),
}

const DEFAULT_MAX_HANDLES: usize = 4096;
const HOST_FILE_TYPE_MASK: u32 = libc::S_IFMT as _;
const HOST_REGULAR_FILE_TYPE: u32 = libc::S_IFREG as _;
const MAX_CHECKPOINT_DIR_ENTRIES: usize = 1_048_576;
const MAX_CHECKPOINT_NAME_BYTES: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileHandleTableSnapshot {
    pub(crate) handles: Vec<FileHandleSnapshot>,
    pub(crate) next_fh: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileHandleSnapshot {
    pub(crate) fh: u64,
    pub(crate) inode: u64,
    pub(crate) kind: FileHandleKindSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileHandleKindSnapshot {
    File {
        readable: bool,
        writable: bool,
        append: bool,
        offset: u64,
        device: u64,
        inode: u64,
        file_type: u32,
    },
    Dir {
        device: u64,
        host_inode: u64,
        entries: Vec<DirEntryData>,
    },
}

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
    fn alloc(&mut self, handle: OpenHandle) -> Option<u64> {
        if self.handles.len() >= self.max_handles {
            return None;
        }
        let fh = self.next_fh;
        self.next_fh += 1;
        self.handles.insert(fh, handle);
        Some(fh)
    }

    pub fn alloc_file(
        &mut self,
        file: std::fs::File,
        inode: u64,
        readable: bool,
        writable: bool,
        append: bool,
    ) -> Option<u64> {
        self.alloc(OpenHandle::File(OpenFileHandle {
            file,
            inode,
            readable,
            writable,
            append,
        }))
    }

    pub fn alloc_dir(&mut self, inode: u64, device: u64, host_inode: u64, entries: Vec<DirEntryData>) -> Option<u64> {
        self.alloc(OpenHandle::Dir(OpenDirHandle {
            inode,
            device,
            host_inode,
            entries,
        }))
    }

    pub fn get_file(&mut self, fh: u64) -> Option<&mut std::fs::File> {
        match self.handles.get_mut(&fh)? {
            OpenHandle::File(handle) => Some(&mut handle.file),
            _ => None,
        }
    }

    pub fn get_dir(&self, fh: u64) -> Option<&Vec<DirEntryData>> {
        match self.handles.get(&fh)? {
            OpenHandle::Dir(handle) => Some(&handle.entries),
            _ => None,
        }
    }

    pub fn remove(&mut self, fh: u64) {
        self.handles.remove(&fh);
    }

    pub(crate) fn checkpoint(&mut self, inodes: &InodeTable) -> Result<FileHandleTableSnapshot> {
        ensure!(
            self.handles.len() <= self.max_handles,
            "VirtioFS open handle checkpoint exceeds configured limit"
        );
        let mut handles = Vec::with_capacity(self.handles.len());
        for (&fh, handle) in &mut self.handles {
            let (inode, kind) = match handle {
                OpenHandle::File(handle) => {
                    let path = inodes
                        .reopen_path(handle.inode)
                        .with_context(|| format!("open VirtioFS file handle {fh} is not reopenable"))?;
                    let metadata = handle
                        .file
                        .metadata()
                        .with_context(|| format!("read metadata for VirtioFS file handle {fh}"))?;
                    let path_metadata = std::fs::metadata(&path)
                        .with_context(|| format!("read path metadata for VirtioFS file handle {fh}"))?;
                    ensure!(
                        (metadata.mode() & HOST_FILE_TYPE_MASK) == HOST_REGULAR_FILE_TYPE
                            && metadata.dev() == path_metadata.dev()
                            && metadata.ino() == path_metadata.ino()
                            && (metadata.mode() & HOST_FILE_TYPE_MASK) == (path_metadata.mode() & HOST_FILE_TYPE_MASK),
                        "open VirtioFS file handle {fh} is not reopenable by stable identity"
                    );
                    let offset = handle
                        .file
                        .stream_position()
                        .with_context(|| format!("read cursor for VirtioFS file handle {fh}"))?;
                    (
                        handle.inode,
                        FileHandleKindSnapshot::File {
                            readable: handle.readable,
                            writable: handle.writable,
                            append: handle.append,
                            offset,
                            device: metadata.dev(),
                            inode: metadata.ino(),
                            file_type: metadata.mode() & HOST_FILE_TYPE_MASK,
                        },
                    )
                }
                OpenHandle::Dir(handle) => {
                    let path = inodes
                        .reopen_path(handle.inode)
                        .with_context(|| format!("open VirtioFS directory handle {fh} is not reopenable"))?;
                    let path_metadata = std::fs::metadata(&path)
                        .with_context(|| format!("read path metadata for VirtioFS directory handle {fh}"))?;
                    ensure!(
                        path_metadata.is_dir()
                            && path_metadata.dev() == handle.device
                            && path_metadata.ino() == handle.host_inode,
                        "open VirtioFS directory handle {fh} identity changed"
                    );
                    validate_dir_entries(fh, &handle.entries)?;
                    (
                        handle.inode,
                        FileHandleKindSnapshot::Dir {
                            device: handle.device,
                            host_inode: handle.host_inode,
                            entries: handle.entries.clone(),
                        },
                    )
                }
            };
            handles.push(FileHandleSnapshot { fh, inode, kind });
        }
        handles.sort_by_key(|handle| handle.fh);
        Ok(FileHandleTableSnapshot {
            handles,
            next_fh: self.next_fh,
        })
    }

    pub(crate) fn restore(snapshot: &FileHandleTableSnapshot, inodes: &InodeTable, read_only: bool) -> Result<Self> {
        ensure!(
            snapshot.handles.len() <= DEFAULT_MAX_HANDLES,
            "VirtioFS open handle checkpoint count exceeds limit"
        );
        let mut handles = HashMap::with_capacity(snapshot.handles.len());
        let mut max_fh = 0u64;
        for snapshot_handle in &snapshot.handles {
            let fh = snapshot_handle.fh;
            ensure!(fh != 0, "VirtioFS checkpoint contains file handle zero");
            let path = inodes.reopen_path(snapshot_handle.inode)?;
            let handle = match &snapshot_handle.kind {
                FileHandleKindSnapshot::File {
                    readable,
                    writable,
                    append,
                    offset,
                    device,
                    inode,
                    file_type,
                } => {
                    ensure!(*readable || *writable, "VirtioFS file handle {fh} has no access mode");
                    ensure!(!append || *writable, "VirtioFS append handle {fh} is not writable");
                    ensure!(
                        !read_only || !*writable,
                        "writable VirtioFS handle {fh} cannot restore on a read-only share"
                    );
                    ensure!(
                        *file_type == HOST_REGULAR_FILE_TYPE,
                        "VirtioFS file handle {fh} is not a regular file"
                    );
                    let mut file = std::fs::OpenOptions::new()
                        .read(*readable)
                        .write(*writable)
                        .append(*append)
                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                        .open(&path)
                        .with_context(|| format!("reopen VirtioFS file handle {fh}: {}", path.display()))?;
                    let metadata = file
                        .metadata()
                        .with_context(|| format!("read reopened VirtioFS file handle {fh} metadata"))?;
                    ensure!(
                        metadata.dev() == *device
                            && metadata.ino() == *inode
                            && (metadata.mode() & HOST_FILE_TYPE_MASK) == *file_type,
                        "VirtioFS file handle {fh} identity changed"
                    );
                    file.seek(SeekFrom::Start(*offset))
                        .with_context(|| format!("restore VirtioFS file handle {fh} cursor"))?;
                    OpenHandle::File(OpenFileHandle {
                        file,
                        inode: snapshot_handle.inode,
                        readable: *readable,
                        writable: *writable,
                        append: *append,
                    })
                }
                FileHandleKindSnapshot::Dir {
                    device,
                    host_inode,
                    entries,
                } => {
                    let metadata = std::fs::metadata(&path)
                        .with_context(|| format!("read reopened VirtioFS directory handle {fh} metadata"))?;
                    ensure!(
                        metadata.is_dir() && metadata.dev() == *device && metadata.ino() == *host_inode,
                        "VirtioFS directory handle {fh} identity changed"
                    );
                    validate_dir_entries(fh, entries)?;
                    OpenHandle::Dir(OpenDirHandle {
                        inode: snapshot_handle.inode,
                        device: *device,
                        host_inode: *host_inode,
                        entries: entries.clone(),
                    })
                }
            };
            if handles.insert(fh, handle).is_some() {
                bail!("VirtioFS checkpoint contains duplicate file handle {fh}");
            }
            max_fh = max_fh.max(fh);
        }
        ensure!(
            snapshot.next_fh > max_fh && snapshot.next_fh < u64::MAX,
            "invalid VirtioFS next file handle id: {}",
            snapshot.next_fh
        );
        Ok(Self {
            handles,
            next_fh: snapshot.next_fh,
            max_handles: DEFAULT_MAX_HANDLES,
        })
    }
}

fn validate_dir_entries(fh: u64, entries: &[DirEntryData]) -> Result<()> {
    ensure!(
        entries.len() <= MAX_CHECKPOINT_DIR_ENTRIES,
        "VirtioFS directory handle {fh} entry count exceeds limit"
    );
    for entry in entries {
        ensure!(
            !entry.name.is_empty()
                && entry.name.len() <= MAX_CHECKPOINT_NAME_BYTES
                && !entry.name.contains(&b'/')
                && !entry.name.contains(&0),
            "VirtioFS directory handle {fh} contains invalid entry name"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
