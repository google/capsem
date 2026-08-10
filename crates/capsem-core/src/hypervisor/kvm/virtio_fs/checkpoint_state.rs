//! Bounded, versioned serialization for process-local VirtioFS state.

use std::io::{Cursor, Read};

use anyhow::{bail, ensure, Context, Result};

use super::TAG_LEN;
use crate::hypervisor::fuse::file_handles::{
    FileHandleKindSnapshot, FileHandleSnapshot, FileHandleTableSnapshot,
};
use crate::hypervisor::fuse::inode_table::{InodeSnapshot, InodeTableSnapshot};
use crate::hypervisor::fuse::DirEntryData;

const STATE_MAGIC: &[u8; 8] = b"CPSVFS\0\0";
const STATE_VERSION: u32 = 1;
const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_INODES: usize = 1_048_576;
const MAX_HANDLES: usize = 4096;
const MAX_DIR_ENTRIES: usize = 1_048_576;
const MAX_PATH_BYTES: usize = 4096;
const MAX_NAME_BYTES: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VirtioFsBackendSnapshot {
    pub(super) tag: [u8; TAG_LEN],
    pub(super) read_only: bool,
    pub(super) inodes: InodeTableSnapshot,
    pub(super) file_handles: FileHandleTableSnapshot,
}

impl VirtioFsBackendSnapshot {
    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.inodes.entries.len() <= MAX_INODES,
            "too many VirtioFS inodes"
        );
        ensure!(
            self.file_handles.handles.len() <= MAX_HANDLES,
            "too many VirtioFS handles"
        );
        let mut out = Vec::new();
        out.extend_from_slice(STATE_MAGIC);
        push_u32(&mut out, STATE_VERSION);
        out.extend_from_slice(&self.tag);
        push_bool(&mut out, self.read_only);
        push_u32(&mut out, self.inodes.entries.len() as u32);
        push_u64(&mut out, self.inodes.next_ino);
        for inode in &self.inodes.entries {
            ensure!(
                inode.relative_path.len() <= MAX_PATH_BYTES,
                "VirtioFS inode path exceeds limit"
            );
            push_u64(&mut out, inode.ino);
            push_u64(&mut out, inode.refcount);
            push_u64(&mut out, inode.device);
            push_u64(&mut out, inode.host_inode);
            push_u32(&mut out, inode.file_type);
            push_bytes(&mut out, &inode.relative_path);
        }
        push_u32(&mut out, self.file_handles.handles.len() as u32);
        push_u64(&mut out, self.file_handles.next_fh);
        for handle in &self.file_handles.handles {
            push_u64(&mut out, handle.fh);
            push_u64(&mut out, handle.inode);
            match &handle.kind {
                FileHandleKindSnapshot::File {
                    readable,
                    writable,
                    append,
                    offset,
                    device,
                    inode,
                    file_type,
                } => {
                    out.push(1);
                    push_bool(&mut out, *readable);
                    push_bool(&mut out, *writable);
                    push_bool(&mut out, *append);
                    push_u64(&mut out, *offset);
                    push_u64(&mut out, *device);
                    push_u64(&mut out, *inode);
                    push_u32(&mut out, *file_type);
                }
                FileHandleKindSnapshot::Dir {
                    device,
                    host_inode,
                    entries,
                } => {
                    ensure!(
                        entries.len() <= MAX_DIR_ENTRIES,
                        "too many VirtioFS directory entries"
                    );
                    out.push(2);
                    push_u64(&mut out, *device);
                    push_u64(&mut out, *host_inode);
                    push_u32(&mut out, entries.len() as u32);
                    for entry in entries {
                        ensure!(
                            entry.name.len() <= MAX_NAME_BYTES,
                            "VirtioFS directory entry name exceeds limit"
                        );
                        push_u64(&mut out, entry.ino);
                        push_u32(&mut out, entry.type_);
                        push_bytes(&mut out, &entry.name);
                    }
                }
            }
        }
        ensure!(
            out.len() <= MAX_STATE_BYTES,
            "VirtioFS backend checkpoint exceeds size limit"
        );
        Ok(out)
    }

    pub(super) fn decode(encoded: &[u8]) -> Result<Self> {
        ensure!(
            encoded.len() <= MAX_STATE_BYTES,
            "VirtioFS backend checkpoint exceeds size limit"
        );
        let mut reader = Cursor::new(encoded);
        let mut magic = [0u8; 8];
        reader
            .read_exact(&mut magic)
            .context("read VirtioFS checkpoint magic")?;
        ensure!(&magic == STATE_MAGIC, "bad VirtioFS checkpoint magic");
        let version = read_u32(&mut reader)?;
        ensure!(
            version == STATE_VERSION,
            "unsupported VirtioFS checkpoint version: {version}"
        );
        let mut tag = [0u8; TAG_LEN];
        reader
            .read_exact(&mut tag)
            .context("read VirtioFS checkpoint tag")?;
        let read_only = read_bool(&mut reader, "read_only")?;
        let inode_count = read_bounded_count(&mut reader, MAX_INODES, "inode")?;
        let next_ino = read_u64(&mut reader)?;
        let mut inode_entries = Vec::with_capacity(inode_count);
        for _ in 0..inode_count {
            inode_entries.push(InodeSnapshot {
                ino: read_u64(&mut reader)?,
                refcount: read_u64(&mut reader)?,
                device: read_u64(&mut reader)?,
                host_inode: read_u64(&mut reader)?,
                file_type: read_u32(&mut reader)?,
                relative_path: read_bounded_bytes(&mut reader, MAX_PATH_BYTES, "inode path")?,
            });
        }
        let handle_count = read_bounded_count(&mut reader, MAX_HANDLES, "handle")?;
        let next_fh = read_u64(&mut reader)?;
        let mut handles = Vec::with_capacity(handle_count);
        for _ in 0..handle_count {
            let fh = read_u64(&mut reader)?;
            let inode = read_u64(&mut reader)?;
            let kind = match read_byte(&mut reader)? {
                1 => FileHandleKindSnapshot::File {
                    readable: read_bool(&mut reader, "file readable")?,
                    writable: read_bool(&mut reader, "file writable")?,
                    append: read_bool(&mut reader, "file append")?,
                    offset: read_u64(&mut reader)?,
                    device: read_u64(&mut reader)?,
                    inode: read_u64(&mut reader)?,
                    file_type: read_u32(&mut reader)?,
                },
                2 => {
                    let device = read_u64(&mut reader)?;
                    let host_inode = read_u64(&mut reader)?;
                    let count =
                        read_bounded_count(&mut reader, MAX_DIR_ENTRIES, "directory entry")?;
                    let mut entries = Vec::with_capacity(count);
                    for _ in 0..count {
                        entries.push(DirEntryData {
                            ino: read_u64(&mut reader)?,
                            type_: read_u32(&mut reader)?,
                            name: read_bounded_bytes(
                                &mut reader,
                                MAX_NAME_BYTES,
                                "directory entry name",
                            )?,
                        });
                    }
                    FileHandleKindSnapshot::Dir {
                        device,
                        host_inode,
                        entries,
                    }
                }
                kind => bail!("invalid VirtioFS checkpoint handle kind: {kind}"),
            };
            handles.push(FileHandleSnapshot { fh, inode, kind });
        }
        ensure!(
            reader.position() as usize == encoded.len(),
            "VirtioFS checkpoint has trailing bytes"
        );
        Ok(Self {
            tag,
            read_only,
            inodes: InodeTableSnapshot {
                entries: inode_entries,
                next_ino,
            },
            file_handles: FileHandleTableSnapshot { handles, next_fh },
        })
    }
}

fn push_bool(out: &mut Vec<u8>, value: bool) {
    out.push(u8::from(value));
}
fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}
fn read_byte(reader: &mut impl Read) -> Result<u8> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}
fn read_bool(reader: &mut impl Read, field: &str) -> Result<bool> {
    match read_byte(reader)? {
        0 => Ok(false),
        1 => Ok(true),
        value => bail!("invalid VirtioFS checkpoint boolean for {field}: {value}"),
    }
}
fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64(reader: &mut impl Read) -> Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn read_bounded_count(reader: &mut impl Read, max: usize, field: &str) -> Result<usize> {
    let count = read_u32(reader)? as usize;
    ensure!(
        count <= max,
        "VirtioFS checkpoint {field} count exceeds limit: {count}"
    );
    Ok(count)
}
fn read_bounded_bytes(reader: &mut impl Read, max: usize, field: &str) -> Result<Vec<u8>> {
    let len = read_u32(reader)? as usize;
    ensure!(
        len <= max,
        "VirtioFS checkpoint {field} length exceeds limit: {len}"
    );
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
