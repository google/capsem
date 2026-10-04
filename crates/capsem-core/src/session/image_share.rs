//! The image share: one image's verified blobs, which the guest reads through
//! a second, read-only VirtioFS device.
//!
//! An image used to reach the guest by being copied, layer by layer, into the
//! workspace stage: guest-writable, and holding every layer byte. It now
//! reaches it from here. The directory lives in the host-only part of the
//! session, beside `system/`, never below the read-write `guest/` share, and
//! the VM owner attaches it at boot as a read-only device ([`IMAGE_SHARE_TAG`]).
//! Read-only is the device's property -- Apple VZ's `VZSharedDirectory`
//! `readOnly`, the KVM FUSE server's `read_only` -- so a guest root that
//! remounts it read-write still cannot change a byte.
//!
//! It holds nothing but one image's blobs, each a regular file named by its
//! SHA-256 hex digest, linked from the service's verified pull or from the
//! clone source's share. Both are host-only and never guest-writable, which is
//! what makes a hard link (one inode, two names) safe here. The global blob
//! cache is never linked: its entries are pruned and rewritten by their own
//! owner, and a session keeps its image whatever the cache later drops.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions, EntryKind};
use capsem_foundation::unix::{fs, tree_clone};

/// The share's directory in the session, outside the guest share.
pub const IMAGE_SHARE_DIR: &str = "image";
/// The VirtioFS tag the guest mounts the share by.
pub const IMAGE_SHARE_TAG: &str = "capsem-image";
/// Every blob is read-only on the host too: hard links share the mode, and
/// nothing may write a staged image after it is verified.
const BLOB_MODE: u32 = 0o444;

/// The share's host path in `session_dir`.
pub fn image_share_path(session_dir: &Path) -> PathBuf {
    session_dir.join(IMAGE_SHARE_DIR)
}

/// Create the share when absent and return its path, for the VM owner to
/// attach before boot. A link or any other entry in its place is refused,
/// never followed.
pub fn prepare_image_share(session_dir: &Path) -> io::Result<PathBuf> {
    open_or_create(session_dir)?;
    Ok(image_share_path(session_dir))
}

/// Make the share hold exactly `blobs`: each a regular file of `from`, named
/// by its SHA-256 hex digest. What the share held before is removed first, so
/// no blob of another image survives in it.
pub fn publish_image_share(session_dir: &Path, from: &ContainedDir, blobs: &[String]) -> io::Result<()> {
    // A layer an image lists twice is one blob.
    let blobs: std::collections::BTreeSet<&OsStr> = blobs.iter().map(OsStr::new).collect();
    for name in &blobs {
        check_blob_name(name)?;
    }
    let share = open_or_create(session_dir)?;
    empty(&share)?;
    for name in blobs {
        tree_clone::link_file(from, name, &share, name)?;
        let blob = share.open_file(name, ContainedOpenOptions::read_only())?;
        fs::set_mode(blob.as_fd(), BLOB_MODE)?;
    }
    share.sync()
}

/// Give `destination`'s share the blobs of `source`'s: a fork or a `--from`
/// clone starts with the image its source staged, linked rather than copied.
/// A source without a share gives an empty one.
pub fn carry_image_share(source: &Path, destination: &Path) -> io::Result<()> {
    let from = match ContainedDir::open_root(source)?.descend(OsStr::new(IMAGE_SHARE_DIR)) {
        Ok(from) => from,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return clear_image_share(destination),
        Err(error) => return Err(error),
    };
    let blobs = blob_names(&from)?;
    publish_image_share(destination, &from, &blobs)
}

/// Empty the share: the session stages no image.
pub fn clear_image_share(session_dir: &Path) -> io::Result<()> {
    let share = open_or_create(session_dir)?;
    empty(&share)?;
    share.sync()
}

/// The blobs the share holds, by SHA-256 hex digest, sorted. Anything else in
/// it is an error: the share holds nothing but blobs.
pub fn image_share_blobs(session_dir: &Path) -> io::Result<Vec<String>> {
    match ContainedDir::open_root(session_dir)?.descend(OsStr::new(IMAGE_SHARE_DIR)) {
        Ok(share) => blob_names(&share),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn open_or_create(session_dir: &Path) -> io::Result<ContainedDir> {
    ContainedDir::open_root(session_dir)?.descend_or_create(OsStr::new(IMAGE_SHARE_DIR), 0o700)
}

fn blob_names(share: &ContainedDir) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in share.entries()? {
        if entry.kind != EntryKind::File {
            return Err(foreign(&entry.name));
        }
        check_blob_name(&entry.name)?;
        names.push(entry.name.to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Remove every entry without following one. The share is host-only, so a
/// directory in it is not one of ours and is refused rather than recursed.
fn empty(share: &ContainedDir) -> io::Result<()> {
    for entry in share.entries()? {
        if entry.kind == EntryKind::Directory {
            return Err(foreign(&entry.name));
        }
        share.remove_non_directory(&entry.name)?;
    }
    Ok(())
}

fn check_blob_name(name: &OsStr) -> io::Result<()> {
    let bytes = name.as_encoded_bytes();
    if bytes.len() == 64 && bytes.iter().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a SHA-256 blob name"),
        ))
    }
}

fn foreign(name: &OsString) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("the image share holds {name:?}, which is not a blob"),
    )
}

#[cfg(test)]
mod tests;
