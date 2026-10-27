use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt as _;

use super::*;

#[test]
fn writable_boot_share_applies_real_metadata_updates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"runtime proof").unwrap();
    let share = runtime_share(dir.path().to_path_buf()).unwrap();
    assert!(!share.read_only);
    let authority = share.metadata_authority.unwrap();
    authority.set_mode(b"file", 0o640).unwrap();
    authority
        .set_times(
            b"file",
            GuestFileTime::Omit,
            GuestFileTime::Exact {
                seconds: 1234567,
                nanoseconds: 123456789,
            },
        )
        .unwrap();
    let metadata = std::fs::metadata(path).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o640);
    assert_eq!((metadata.mtime(), metadata.mtime_nsec()), (1234567, 123456789));
}

#[test]
fn boot_metadata_authority_refuses_escape_and_retains_its_open_root() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("file");
    std::fs::write(&path, b"outside").unwrap();
    let before = std::fs::metadata(&path).unwrap();
    let share = runtime_share(dir.path().to_path_buf()).unwrap();
    let authority = share.metadata_authority.unwrap();
    ContainedDir::open_root(dir.path())
        .unwrap()
        .symlink(OsStr::new("escape"), outside.path().as_os_str())
        .unwrap();
    for relative in [b"escape/file".as_slice(), path.as_os_str().as_bytes(), b"../file"] {
        assert!(authority.set_mode(relative, 0o777).is_err());
        assert!(authority
            .set_times(relative, GuestFileTime::Now, GuestFileTime::Now)
            .is_err());
    }
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(
        (after.mode(), after.mtime(), after.mtime_nsec()),
        (before.mode(), before.mtime(), before.mtime_nsec())
    );

    let moved = dir.path().with_extension("moved");
    std::fs::rename(dir.path(), &moved).unwrap();
    std::fs::create_dir(dir.path()).unwrap();
    std::fs::write(moved.join("file"), b"original share").unwrap();
    authority.set_mode(b"file", 0o600).unwrap();
    assert_eq!(std::fs::metadata(moved.join("file")).unwrap().mode() & 0o777, 0o600);
    assert!(!dir.path().join("file").exists());
    std::fs::remove_dir_all(moved).unwrap();
}
