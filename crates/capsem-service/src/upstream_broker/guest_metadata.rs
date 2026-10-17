//! Descriptor-relative guest metadata mutations owned by the coordinator.
use capsem_foundation::unix::contained::FileTimeSpec;
use capsem_proto::upstream_grant::GuestFileTime;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

pub(super) fn set_guest_mode(session_dir: &Path, relative_path: &[u8], mode: u16) -> io::Result<()> {
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(session_dir)?
        .descend(std::ffi::OsStr::new(capsem_core::GUEST_SHARE_DIR))?;
    let path = Path::new(std::ffi::OsStr::from_bytes(relative_path));
    root.set_relative_mode(path, u32::from(mode))
}

pub(super) fn set_guest_times(
    session_dir: &Path,
    relative_path: &[u8],
    atime: GuestFileTime,
    mtime: GuestFileTime,
) -> io::Result<()> {
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(session_dir)?
        .descend(std::ffi::OsStr::new(capsem_core::GUEST_SHARE_DIR))?;
    let path = Path::new(std::ffi::OsStr::from_bytes(relative_path));
    root.set_relative_times(path, file_time_spec(atime), file_time_spec(mtime))
}

const fn file_time_spec(time: GuestFileTime) -> FileTimeSpec {
    match time {
        GuestFileTime::Omit => FileTimeSpec::Omit,
        GuestFileTime::Now => FileTimeSpec::Now,
        GuestFileTime::Exact { seconds, nanoseconds } => FileTimeSpec::Exact { seconds, nanoseconds },
    }
}
