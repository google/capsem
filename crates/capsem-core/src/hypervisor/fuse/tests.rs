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

#[test]
fn extract_name_null_terminated() {
    assert_eq!(extract_name(b"hello\0world"), Some(b"hello".as_slice()));
}
#[test]
fn extract_name_no_null() {
    assert_eq!(extract_name(b"hello"), Some(b"hello".as_slice()));
}
#[test]
fn extract_name_empty_returns_none() {
    assert!(extract_name(b"").is_none());
    assert!(extract_name(b"\0").is_none());
}
#[test]
fn two_names_works() {
    let (a, b) = extract_two_names(b"old\0new\0").unwrap();
    assert_eq!(a, b"old");
    assert_eq!(b, b"new");
}
#[test]
fn two_names_no_second_null() {
    let (a, b) = extract_two_names(b"old\0new").unwrap();
    assert_eq!(a, b"old");
    assert_eq!(b, b"new");
}

// Dirent alignment

#[test]
fn dirent_align_already() {
    assert_eq!(dirent_align(24), 24);
}
#[test]
fn dirent_align_rounds() {
    assert_eq!(dirent_align(25), 32);
}
#[test]
fn dirent_align_zero() {
    assert_eq!(dirent_align(0), 0);
}

// Mode to dtype

#[test]
fn dtype_regular() {
    assert_eq!(mode_to_dtype(S_IFREG | 0o644), DT_REG);
}
#[test]
fn dtype_directory() {
    assert_eq!(mode_to_dtype(S_IFDIR | 0o755), DT_DIR);
}
#[test]
fn dtype_symlink() {
    assert_eq!(mode_to_dtype(S_IFLNK | 0o777), DT_LNK);
}
#[test]
fn dtype_unknown() {
    assert_eq!(mode_to_dtype(0), DT_UNKNOWN);
}

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
    assert_eq!(attr.uid, 0);
    assert_eq!(attr.gid, 0);
}

#[test]
fn attr_directory() {
    let dir = temp_share("meta-dir");
    let meta = std::fs::metadata(&dir).unwrap();
    let attr = metadata_to_fuse_attr(1, &meta);
    assert_ne!(attr.mode & S_IFDIR, 0);
}
