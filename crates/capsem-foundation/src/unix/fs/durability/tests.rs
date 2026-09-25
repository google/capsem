use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;

use super::*;

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn set_mode_applies_permission_bits_to_files_and_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("file");
    let file = std::fs::File::create(&file_path).unwrap();
    set_mode(file.as_fd(), 0o640).unwrap();
    assert_eq!(mode_of(&file_path), 0o640);

    let dir = std::fs::File::open(tmp.path()).unwrap();
    set_mode(dir.as_fd(), 0o750).unwrap();
    assert_eq!(mode_of(tmp.path()), 0o750);
    set_mode(dir.as_fd(), 0o700).unwrap();
}

#[test]
fn set_mode_ignores_bits_outside_the_permission_mask() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("file");
    let file = std::fs::File::create(&path).unwrap();
    set_mode(file.as_fd(), 0o170_644).unwrap();
    assert_eq!(mode_of(&path), 0o644);
}

#[test]
fn sync_calls_accept_read_only_descriptors() {
    // Flushing never needs write access: the clone path syncs a guest file
    // through a read-only, no-follow descriptor.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("file");
    std::fs::write(&path, b"data").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    sync(file.as_fd()).unwrap();
    sync_before_barrier(file.as_fd()).unwrap();
    sync_filesystem(file.as_fd()).unwrap();
    let dir = std::fs::File::open(tmp.path()).unwrap();
    sync_filesystem(dir.as_fd()).unwrap();
}

#[test]
fn errors_keep_the_os_errno() {
    // A socket can never be fsynced: the failure must surface with its errno.
    let (reader, _writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let error = sync(reader.as_fd()).unwrap_err();
    assert!(error.raw_os_error().is_some(), "{error}");
}
