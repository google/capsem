//! Cloning a sandbox's state into a new session directory (fork, create-from).
//!
//! The source's `guest/` subtree is the VirtioFS share: the guest can write
//! every entry below it, including replacing `workspace/` with a symlink to a
//! host path. Nothing here resolves a path below the session root: each step
//! is a no-follow, descriptor-relative operation through
//! `capsem_foundation::unix::tree_clone`. The system overlay lives in the
//! host-only `system/` directory (see `super::overlay`).

use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use capsem_foundation::unix::contained::{ContainedDir, EntryKind};
use capsem_foundation::unix::tree_clone::{self, CloneStats};
use tracing::info;

use super::overlay::{adopt_system_overlay, SYSTEM_OVERLAY_DIR, SYSTEM_OVERLAY_IMAGE};
use crate::GUEST_SHARE_DIR;

const WORKSPACE_DIR: &str = "workspace";

/// Clone a sandbox's `system/` overlay, `guest/workspace` and session ledger
/// from `src_session_dir` into the existing, empty `dst_session_dir`, and
/// create the `workspace` compat link. Returns the destination's disk usage.
pub fn clone_sandbox_state(src_session_dir: &Path, dst_session_dir: &Path) -> anyhow::Result<u64> {
    adopt_system_overlay(src_session_dir).context("system overlay of the clone source")?;
    let src_root = ContainedDir::open_root(src_session_dir)
        .with_context(|| format!("open clone source {}", src_session_dir.display()))?;
    let dst_root = ContainedDir::open_root(dst_session_dir)
        .with_context(|| format!("open clone destination {}", dst_session_dir.display()))?;
    let mut stats = CloneStats::default();

    let system = OsStr::new(SYSTEM_OVERLAY_DIR);
    let from = src_root.descend(system).context("clone source system directory")?;
    flush_system_overlay(&from)?;
    let to = dst_root.descend_or_create(system, 0o700)?;
    stats += tree_clone::clone_tree(&from, &to).context("clone system overlay")?;
    require_regular_overlay(&to)?;

    // Sessions from before the single-share layout keep `workspace/`
    // directly in the session directory.
    let share = OsStr::new(GUEST_SHARE_DIR);
    let src_share = match src_root.entry_kind(share)? {
        Some(_) => src_root.descend(share).context("clone source guest share")?,
        None => src_root.try_clone()?,
    };
    let workspace = OsStr::new(WORKSPACE_DIR);
    let dst_share = dst_root.descend_or_create(share, src_share.mode()?)?;
    // Absent is fine (nothing to clone); a symlink or any other type in place
    // of the directory is refused rather than followed.
    if let Some(kind) = src_share.entry_kind(workspace)? {
        let from = src_share
            .descend(workspace)
            .with_context(|| format!("guest share entry {WORKSPACE_DIR} ({kind:?})"))?;
        let to = dst_share.descend_or_create(workspace, from.mode()?)?;
        stats += tree_clone::clone_tree(&from, &to).context("clone guest workspace")?;
    }
    if dst_root.entry_kind(workspace)?.is_none() {
        dst_root
            .symlink(workspace, Path::new(GUEST_SHARE_DIR).join(WORKSPACE_DIR).as_os_str())
            .context("create compat symlink for workspace")?;
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

/// The fork boots its overlay image by path, and the walker recreates links
/// verbatim: whatever the source held, the copy must be a regular file.
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
