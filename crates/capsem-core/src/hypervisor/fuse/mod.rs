//! FUSE protocol types and portable filesystem helpers.
//!
//! Compiled on all Unix platforms (macOS + Linux) so tests run everywhere.
//! The KVM-specific VirtioFS device (`kvm/virtio_fs/`) imports from here.

pub mod file_handles;
pub mod inode_table;
pub mod protocol;

#[allow(unused_imports)] // Used by KVM backend (not compiled on macOS)
pub use file_handles::{DirEntryData, FileHandleTable, OpenHandle};
#[allow(unused_imports)] // Used by KVM backend (not compiled on macOS)
pub use inode_table::{InodeEntry, InodeTable};
pub use protocol::*;

use std::os::unix::fs::MetadataExt;

// ---------------------------------------------------------------------------
// Struct serialization helpers
// ---------------------------------------------------------------------------

/// Deserialize a `Copy` struct from the front of a byte buffer.
///
/// Returns `None` if `buf` is shorter than `size_of::<T>()`.
pub fn read_struct<T: Copy>(buf: &[u8]) -> Option<T> {
    if buf.len() < std::mem::size_of::<T>() {
        return None;
    }
    // Safety: bounds check above guarantees sufficient bytes.
    // read_unaligned handles any alignment.
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) })
}

pub fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(val as *const T as *const u8, std::mem::size_of::<T>()) }
}

// ---------------------------------------------------------------------------
// Response builders
// ---------------------------------------------------------------------------

pub fn error_response(unique: u64, errno: i32) -> Vec<u8> {
    let header = FuseOutHeader {
        len: std::mem::size_of::<FuseOutHeader>() as u32,
        error: errno,
        unique,
    };
    as_bytes(&header).to_vec()
}

pub fn success_response(unique: u64, body: &[u8]) -> Vec<u8> {
    let header = FuseOutHeader {
        len: (std::mem::size_of::<FuseOutHeader>() + body.len()) as u32,
        error: 0,
        unique,
    };
    let mut buf = as_bytes(&header).to_vec();
    buf.extend_from_slice(body);
    buf
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

pub fn metadata_to_fuse_attr(ino: u64, meta: &std::fs::Metadata) -> FuseAttr {
    FuseAttr {
        ino,
        size: meta.size(),
        blocks: meta.blocks(),
        atime: meta.atime() as u64,
        mtime: meta.mtime() as u64,
        ctime: meta.ctime() as u64,
        atimensec: meta.atime_nsec() as u32,
        mtimensec: meta.mtime_nsec() as u32,
        ctimensec: meta.ctime_nsec() as u32,
        mode: meta.mode(),
        nlink: meta.nlink() as u32,
        // The backing workspace is owned by the host user, but the guest runs
        // the exported tree as root. Reporting host ids into the guest makes
        // standard tools reject their own files, e.g. git's safe.directory
        // ownership check.
        uid: 0,
        gid: 0,
        rdev: meta.rdev() as u32,
        blksize: meta.blksize() as u32,
        flags: 0,
    }
}

pub fn mode_to_dtype(mode: u32) -> u32 {
    match mode & S_IFMT {
        S_IFREG => DT_REG,
        S_IFDIR => DT_DIR,
        S_IFLNK => DT_LNK,
        S_IFBLK => DT_BLK,
        S_IFCHR => DT_CHR,
        _ => DT_UNKNOWN,
    }
}

pub fn extract_name(body: &[u8]) -> Option<&[u8]> {
    let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
    if end == 0 {
        return None;
    }
    Some(&body[..end])
}

pub fn extract_two_names(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let first_end = body.iter().position(|&b| b == 0)?;
    if first_end == 0 {
        return None;
    }
    let rest = &body[first_end + 1..];
    let second_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    if second_end == 0 {
        return None;
    }
    Some((&body[..first_end], &rest[..second_end]))
}

pub fn dirent_align(size: usize) -> usize {
    (size + 7) & !7
}

pub fn io_error_to_errno(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(libc::EIO)
}

pub fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
