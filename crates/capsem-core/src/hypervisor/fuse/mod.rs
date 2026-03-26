//! FUSE protocol types and portable filesystem helpers.
//!
//! Compiled on all Unix platforms (macOS + Linux) so tests run everywhere.
//! The KVM-specific VirtioFS device (`kvm/virtio_fs/`) imports from here.

pub mod protocol;
pub mod inode_table;
pub mod file_handles;

pub use protocol::*;
pub use inode_table::{InodeTable, InodeEntry};
pub use file_handles::{FileHandleTable, OpenHandle, DirEntryData};

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
    unsafe {
        std::slice::from_raw_parts(val as *const T as *const u8, std::mem::size_of::<T>())
    }
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
        uid: meta.uid(),
        gid: meta.gid(),
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
    if end == 0 { return None; }
    Some(&body[..end])
}

pub fn extract_two_names(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let first_end = body.iter().position(|&b| b == 0)?;
    if first_end == 0 { return None; }
    let rest = &body[first_end + 1..];
    let second_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    if second_end == 0 { return None; }
    Some((&body[..first_end], &rest[..second_end]))
}

pub fn dirent_align(size: usize) -> usize {
    (size + 7) & !7
}

pub fn io_error_to_errno(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(libc::EIO)
}

pub fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // read_struct safety

    #[test]
    fn read_struct_short_buffer_returns_none() {
        let bytes = [0u8; 4]; // FuseOutHeader is 16 bytes
        assert!(read_struct::<FuseOutHeader>(&bytes).is_none());
    }

    #[test]
    fn read_struct_empty_buffer_returns_none() {
        assert!(read_struct::<FuseOutHeader>(&[]).is_none());
    }

    #[test]
    fn read_struct_exact_size_succeeds() {
        let bytes = [0u8; 16]; // exactly size_of::<FuseOutHeader>()
        assert!(read_struct::<FuseOutHeader>(&bytes).is_some());
    }

    // Response construction

    #[test]
    fn error_response_format() {
        let resp = error_response(42, -libc::ENOENT);
        assert_eq!(resp.len(), 16);
        let header: FuseOutHeader = read_struct(&resp).unwrap();
        assert_eq!(header.len, 16);
        assert_eq!(header.error, -libc::ENOENT);
        assert_eq!(header.unique, 42);
    }

    #[test]
    fn success_response_with_data() {
        let resp = success_response(99, &[1, 2, 3, 4]);
        assert_eq!(resp.len(), 20);
        let header: FuseOutHeader = read_struct(&resp).unwrap();
        assert_eq!(header.len, 20);
        assert_eq!(header.error, 0);
        assert_eq!(&resp[16..], &[1, 2, 3, 4]);
    }

    #[test]
    fn success_response_empty_body() {
        let resp = success_response(1, &[]);
        assert_eq!(resp.len(), 16);
    }

    // Name extraction

    #[test] fn extract_name_null_terminated() {
        assert_eq!(extract_name(b"hello\0world"), Some(b"hello".as_slice()));
    }
    #[test] fn extract_name_no_null() {
        assert_eq!(extract_name(b"hello"), Some(b"hello".as_slice()));
    }
    #[test] fn extract_name_empty_returns_none() {
        assert!(extract_name(b"").is_none());
        assert!(extract_name(b"\0").is_none());
    }
    #[test] fn two_names_works() {
        let (a, b) = extract_two_names(b"old\0new\0").unwrap();
        assert_eq!(a, b"old"); assert_eq!(b, b"new");
    }
    #[test] fn two_names_no_second_null() {
        let (a, b) = extract_two_names(b"old\0new").unwrap();
        assert_eq!(a, b"old"); assert_eq!(b, b"new");
    }

    // Dirent alignment

    #[test] fn dirent_align_already() { assert_eq!(dirent_align(24), 24); }
    #[test] fn dirent_align_rounds() { assert_eq!(dirent_align(25), 32); }
    #[test] fn dirent_align_zero() { assert_eq!(dirent_align(0), 0); }

    // Mode to dtype

    #[test] fn dtype_regular() { assert_eq!(mode_to_dtype(S_IFREG | 0o644), DT_REG); }
    #[test] fn dtype_directory() { assert_eq!(mode_to_dtype(S_IFDIR | 0o755), DT_DIR); }
    #[test] fn dtype_symlink() { assert_eq!(mode_to_dtype(S_IFLNK | 0o777), DT_LNK); }
    #[test] fn dtype_unknown() { assert_eq!(mode_to_dtype(0), DT_UNKNOWN); }

    // metadata_to_fuse_attr

    #[test]
    fn attr_regular_file() {
        let dir = temp_share("meta-reg");
        std::fs::write(dir.join("test.txt"), b"hello world").unwrap();
        let meta = std::fs::metadata(dir.join("test.txt")).unwrap();
        let attr = metadata_to_fuse_attr(42, &meta);
        assert_eq!(attr.ino, 42);
        assert_eq!(attr.size, 11);
        assert_ne!(attr.mode & S_IFREG, 0);
    }

    #[test]
    fn attr_directory() {
        let dir = temp_share("meta-dir");
        let meta = std::fs::metadata(&dir).unwrap();
        let attr = metadata_to_fuse_attr(1, &meta);
        assert_ne!(attr.mode & S_IFDIR, 0);
    }
}
