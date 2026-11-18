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
//! It holds one image's blobs and explicitly named host-owned launch controls.
//! Each blob is a regular file named by its
//! SHA-256 hex digest, linked from the service's verified pull or from the
//! clone source's share. Both are host-only and never guest-writable, which is
//! what makes a hard link (one inode, two names) safe here. Ordinary writable
//! cache entries are copied into the private pull first. Published filesystem
//! payloads use the assets owner's separate immutable generations: verified
//! read-only links retain one inode, and active links protect it from pruning.
//! Neither cache publication nor replacement can modify a held generation.

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
pub(crate) const LAUNCH_CONTROL_LIMIT: u64 = 16 * 1024 * 1024;

fn launch_control_name(name: &OsStr) -> bool {
    name == OsStr::new("options.json")
        || crate::container::LAUNCHER_FILES
            .iter()
            .any(|(allowed, _)| name == OsStr::new(allowed))
}

/// Publish service-owned launch material outside the guest-writable share.
/// Atomic replacement keeps previous readers and forked sessions independent.
pub fn publish_launch_control(session_dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(launch_control_name(OsStr::new(name)), "unknown launch control name");
    anyhow::ensure!(
        bytes.len() as u64 <= LAUNCH_CONTROL_LIMIT,
        "launch control exceeds byte limit"
    );
    let share = open_or_create(session_dir)?;
    anyhow::ensure!(
        matches!(share.entry_kind(OsStr::new(name))?, None | Some(EntryKind::File)),
        "launch control is not a regular file"
    );
    super::publish_contained_private(&share, name, bytes)
}

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
    publish_image_share(destination, &from, &blobs)?;
    for entry in from.entries()? {
        if launch_control_name(&entry.name) {
            use std::io::Read;
            let file = from.open_file(&entry.name, ContainedOpenOptions::read_only())?;
            if file.metadata()?.len() > LAUNCH_CONTROL_LIMIT {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized launch control"));
            }
            let mut bytes = Vec::new();
            file.take(LAUNCH_CONTROL_LIMIT + 1).read_to_end(&mut bytes)?;
            publish_launch_control(
                destination,
                entry.name.to_str().ok_or_else(|| foreign(&entry.name))?,
                &bytes,
            )
            .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

/// Empty the share: the session stages no image.
pub fn clear_image_share(session_dir: &Path) -> io::Result<()> {
    let share = open_or_create(session_dir)?;
    empty(&share)?;
    share.sync()
}

/// The blobs the share holds, by SHA-256 hex digest, sorted. Anything else in
/// it is an error, except for explicitly named regular launch controls.
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
        if launch_control_name(&entry.name) {
            continue;
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
