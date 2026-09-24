//! Reaching a sandbox's workspace from the host.
//!
//! The workspace is inside the guest's read-write VirtioFS share, so the guest
//! can replace `guest/`'s `workspace` entry with a link to any host directory.
//! Opening its path follows that link; this opens the host-owned session
//! directory and descends without following one.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use capsem_foundation::unix::contained::ContainedDir;

use crate::GUEST_SHARE_DIR;

/// The workspace directory's name, inside the guest share.
pub const WORKSPACE_DIR: &str = "workspace";

/// Open the workspace of `session_dir`, refusing a link at either level.
/// Sessions from before the single-share layout keep it at the session root.
pub fn open_workspace(session_dir: &Path) -> io::Result<ContainedDir> {
    let root = ContainedDir::open_root(session_dir)?;
    let share = OsStr::new(GUEST_SHARE_DIR);
    let parent = match root.entry_kind(share)? {
        Some(_) => root.descend(share)?,
        None => root,
    };
    parent.descend(OsStr::new(WORKSPACE_DIR))
}

#[cfg(test)]
mod tests;
