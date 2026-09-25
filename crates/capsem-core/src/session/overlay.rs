//! Where a sandbox's system overlay image lives, and moving it there.
//!
//! The image is the guest's writable root overlay, attached as a virtio-blk
//! disk by path. It used to sit at `guest/system/rootfs.img`, inside the
//! read-write VirtioFS share, where a root guest could mount the share and
//! replace it with a symlink to any host file: the next boot attached that
//! file as the guest's disk, and the service and support bundle read through
//! the same link. It now lives in the session's host-only `system/`
//! directory, which the guest cannot name.

use std::ffi::OsStr;
use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions, EntryKind};

/// The host-only directory holding the overlay image.
pub const SYSTEM_OVERLAY_DIR: &str = "system";
/// The overlay image's file name.
pub const SYSTEM_OVERLAY_IMAGE: &str = "rootfs.img";

/// The overlay image's path, for a session `adopt_system_overlay` has run on.
pub fn system_overlay_image_path(session_dir: &Path) -> PathBuf {
    session_dir.join(SYSTEM_OVERLAY_DIR).join(SYSTEM_OVERLAY_IMAGE)
}

/// Open the overlay image read-only, refusing a link anywhere below the session.
pub fn open_system_overlay(session_dir: &Path) -> io::Result<File> {
    ContainedDir::open_root(session_dir)?
        .descend(OsStr::new(SYSTEM_OVERLAY_DIR))?
        .open_file(OsStr::new(SYSTEM_OVERLAY_IMAGE), ContainedOpenOptions::read_only())
}

/// The overlay image's metadata, read the way `open_system_overlay` opens it.
pub fn system_overlay_metadata(session_dir: &Path) -> io::Result<Metadata> {
    open_system_overlay(session_dir)?.metadata()
}

/// Make the host-only `system/` directory the overlay's home, moving the
/// image out of the guest share of a session laid out before it existed.
///
/// Idempotent, and a no-op for a current session. The move never follows a
/// link, and whatever the guest left in the image's place is refused rather
/// than attached: the session then fails to boot instead of handing the guest
/// a host file as its disk.
pub fn adopt_system_overlay(session_dir: &Path) -> io::Result<()> {
    let root = ContainedDir::open_root(session_dir)?;
    let dir = OsStr::new(SYSTEM_OVERLAY_DIR);
    let image = OsStr::new(SYSTEM_OVERLAY_IMAGE);
    let host = match root.entry_kind(dir)? {
        Some(EntryKind::Directory) => root.descend(dir)?,
        // The legacy layout's compat link into the share. Removing a link
        // never touches its target.
        Some(EntryKind::Other) => {
            root.remove_symlink(dir)?;
            root.descend_or_create(dir, 0o700)?
        }
        None => root.descend_or_create(dir, 0o700)?,
        Some(EntryKind::File) => return Err(refused(session_dir, "its system entry is a file")),
    };
    if host.entry_kind(image)?.is_none() {
        let legacy = match root
            .descend(OsStr::new(crate::GUEST_SHARE_DIR))
            .and_then(|share| share.descend(dir))
        {
            Ok(legacy) => legacy,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if legacy.entry_kind(image)?.is_none() {
            return Ok(());
        }
        legacy.rename_to(image, &host, image)?;
    }
    match host.entry_kind(image)? {
        Some(EntryKind::File) => Ok(()),
        kind => {
            if kind == Some(EntryKind::Other) {
                let _ = host.remove_symlink(image);
            }
            Err(refused(session_dir, "the image is not a regular file"))
        }
    }
}

fn refused(session_dir: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "refusing the system overlay of {}: {why}; it may have been replaced from inside the guest",
            session_dir.display()
        ),
    )
}

#[cfg(test)]
mod tests;
