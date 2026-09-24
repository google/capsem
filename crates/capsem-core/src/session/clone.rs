//! Cloning a sandbox's state into a new session directory (fork, create-from).
//!
//! The source's `guest/` subtree is the VirtioFS share: the guest can write
//! every entry below it, including replacing `system/`, `workspace/` or
//! `rootfs.img` with a symlink to a host path. Nothing here resolves a path
//! below the share root: each step is a no-follow, descriptor-relative
//! operation through `capsem_foundation::unix::tree_clone`. In the current
//! layout the session root's `system`/`workspace` are compat links, so they
//! are never taken as the source.

use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use capsem_foundation::unix::contained::{ContainedDir, EntryKind};
use capsem_foundation::unix::tree_clone::{self, CloneStats};
use tracing::info;

/// The share subdirectories a sandbox's state lives in.
const SHARED_STATE_DIRS: [&str; 2] = ["system", "workspace"];
const SYSTEM_OVERLAY_IMAGE: &str = "rootfs.img";

/// Clone a sandbox's `guest/{system,workspace}` and its session ledger from
/// `src_session_dir` into the existing, empty `dst_session_dir`, and create
/// the `system`/`workspace` compat links. Returns the destination's disk usage.
pub fn clone_sandbox_state(src_session_dir: &Path, dst_session_dir: &Path) -> anyhow::Result<u64> {
    // Sessions from before the single-share layout keep `system/` and
    // `workspace/` directly in the session directory.
    let guest_share = crate::guest_share_dir(src_session_dir);
    let src_share_path = if guest_share.exists() {
        guest_share
    } else {
        src_session_dir.to_path_buf()
    };
    let dst_root = ContainedDir::open_root(dst_session_dir)
        .with_context(|| format!("open clone destination {}", dst_session_dir.display()))?;
    let mut stats = CloneStats::default();

    if src_share_path.exists() {
        let src_share = ContainedDir::open_root(&src_share_path)
            .with_context(|| format!("open guest share {}", src_share_path.display()))?;
        let dst_share = dst_root.descend_or_create(OsStr::new("guest"), src_share.mode()?)?;
        for name in SHARED_STATE_DIRS {
            let name = OsStr::new(name);
            // Absent is fine (nothing to clone); a symlink or any other type in
            // place of the directory is refused rather than followed.
            let Some(kind) = src_share.entry_kind(name)? else { continue };
            let from = src_share
                .descend(name)
                .with_context(|| format!("guest share entry {} ({kind:?})", name.to_string_lossy()))?;
            if name == "system" {
                flush_system_overlay(&from)?;
            }
            let to = dst_share.descend_or_create(name, from.mode()?)?;
            stats += tree_clone::clone_tree(&from, &to)
                .with_context(|| format!("clone guest {}", name.to_string_lossy()))?;
            if name == "system" {
                require_regular_overlay(&to)?;
            }
        }
    }

    for name in SHARED_STATE_DIRS {
        let link = dst_session_dir.join(name);
        if std::fs::symlink_metadata(&link).is_err() {
            std::os::unix::fs::symlink(Path::new("guest").join(name), &link)
                .with_context(|| format!("create compat symlink for {name}"))?;
        }
    }

    // The ledger lives at the session root, outside the share. session.db may
    // be in WAL mode while the VM runs, so copying the main file alone can
    // produce a stale or malformed fork; the logger writes a coherent image
    // and brings the body archive beside it, since the database only indexes
    // into that file.
    if src_session_dir.join("session.db").exists() {
        capsem_logger::snapshot_session_ledger(src_session_dir, dst_session_dir)
            .context("failed to snapshot the session ledger")?;
    }

    let size_bytes = super::disk_usage_bytes(dst_session_dir);
    info!(
        cloned_files = stats.cloned_files,
        copied_files = stats.copied_files,
        directories = stats.directories,
        symlinks = stats.symlinks,
        skipped = stats.skipped,
        size_bytes,
        "sandbox state cloned"
    );
    Ok(size_bytes)
}

/// Clone the host-owned file `src` to the new path `dst` (the preformatted
/// system overlay template into a session). `dst` must not exist.
pub fn clone_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let (src_dir, src_name) = split(src)?;
    let (dst_dir, dst_name) = split(dst)?;
    tree_clone::clone_file(
        &ContainedDir::open_root(src_dir)?,
        src_name,
        &ContainedDir::open_root(dst_dir)?,
        dst_name,
    )?;
    Ok(())
}

/// Guest writes to the overlay reach the host page cache through the block
/// device; a metadata-only clone (APFS clonefile) would capture stale data
/// without this flush. A missing image is fine; a link in its place is not.
fn flush_system_overlay(system: &ContainedDir) -> anyhow::Result<()> {
    let name = OsStr::new(SYSTEM_OVERLAY_IMAGE);
    if system.entry_kind(name)?.is_none() {
        return Ok(());
    }
    tree_clone::sync_file(system, name).context("flush system overlay image before clone")
}

/// The fork boots its overlay image by path. The walker recreates links
/// verbatim, so a guest that swapped `rootfs.img` for a link after the flush
/// would hand the fork a host file as its disk: refuse anything but a file.
fn require_regular_overlay(system: &ContainedDir) -> anyhow::Result<()> {
    match system.entry_kind(OsStr::new(SYSTEM_OVERLAY_IMAGE))? {
        None | Some(EntryKind::File) => Ok(()),
        Some(kind) => anyhow::bail!("cloned {SYSTEM_OVERLAY_IMAGE} is not a regular file ({kind:?})"),
    }
}

fn split(path: &Path) -> std::io::Result<(&Path, &OsStr)> {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => Ok((dir, name)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} has no parent directory or file name", path.display()),
        )),
    }
}

#[cfg(test)]
mod tests;
