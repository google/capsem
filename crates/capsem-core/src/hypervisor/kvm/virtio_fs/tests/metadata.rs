use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_foundation::unix::contained::{ContainedDir, FileTimeSpec};
use capsem_proto::upstream_grant::GuestFileTime;

use super::tests::{build_request, lookup, make_header, response_error, temp_share, test_processor};
use super::*;

struct LocalMetadataAuthority(PathBuf);

impl crate::GuestMetadataAuthority for LocalMetadataAuthority {
    fn set_mode(&self, relative_path: &[u8], mode: u16) -> std::io::Result<()> {
        let root = ContainedDir::open_root(&self.0)?;
        root.set_relative_mode(Path::new(std::ffi::OsStr::from_bytes(relative_path)), u32::from(mode))
    }

    fn set_times(&self, relative_path: &[u8], atime: GuestFileTime, mtime: GuestFileTime) -> std::io::Result<()> {
        let convert = |value| match value {
            GuestFileTime::Omit => FileTimeSpec::Omit,
            GuestFileTime::Now => FileTimeSpec::Now,
            GuestFileTime::Exact { seconds, nanoseconds } => FileTimeSpec::Exact { seconds, nanoseconds },
        };
        let root = ContainedDir::open_root(&self.0)?;
        root.set_relative_times(
            Path::new(std::ffi::OsStr::from_bytes(relative_path)),
            convert(atime),
            convert(mtime),
        )
    }
}

pub(super) fn metadata_authority(dir: &Path) -> Arc<dyn crate::GuestMetadataAuthority> {
    Arc::new(LocalMetadataAuthority(dir.to_path_buf()))
}

#[test]
fn setattr_timestamps_use_the_metadata_authority() {
    let dir = temp_share("setattr-times");
    let path = dir.join("f.txt");
    std::fs::write(&path, b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let attr_in = FuseSetAttrIn {
        valid: FATTR_ATIME | FATTR_MTIME,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 1_234_567,
        mtime: 2_345_678,
        ctime: 0,
        atimensec: 123_456_789,
        mtimensec: 987_654_321,
        ctimensec: 0,
        mode: 0,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };

    let h = make_header(FUSE_SETATTR, ino, 4);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));

    assert_eq!(response_error(&resp), 0);
    let metadata = std::fs::metadata(path).unwrap();
    assert_eq!((metadata.atime(), metadata.atime_nsec()), (1_234_567, 123_456_789));
    assert_eq!((metadata.mtime(), metadata.mtime_nsec()), (2_345_678, 987_654_321));
}

#[test]
fn setattr_timestamps_never_fall_back_to_owner_authority() {
    let dir = temp_share("setattr-times-no-broker");
    let path = dir.join("f.txt");
    std::fs::write(&path, b"x").unwrap();
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut proc = test_processor(&dir);
    proc.metadata_authority = None;
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let attr_in = FuseSetAttrIn {
        valid: FATTR_MTIME_NOW,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };

    let h = make_header(FUSE_SETATTR, ino, 5);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));

    assert_eq!(response_error(&resp), -libc::EPERM);
    assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), before);
}
