//! Copy-on-write cloning of a directory tree a guest can write to.
//!
//! The source is guest-controlled and may be live: the guest can swap any
//! entry for a symlink between the listing and the copy. Every step therefore
//! goes through [`ContainedDir`] descriptors. Directories and files open
//! relative to an already-open parent with `O_NOFOLLOW`, file contents clone
//! from the open descriptor, and destination entries are created exclusively.
//! A symlink is recreated as a link and never followed; FIFOs, sockets and
//! devices are skipped, as is an entry that vanishes or changes type mid-copy.
//!
//! Setuid, setgid and sticky bits are dropped: a guest file must not become a
//! privileged host file by being cloned.
//!
//! This module only composes [`super::contained`] and [`super::fs`]; it makes
//! no system call of its own.

use std::ffi::OsStr;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::contained::{is_not_directory, is_symlink_refusal, ContainedDir, ContainedOpenOptions, EntryKind};
use super::fs::{self, CloneMethod};

/// Permission bits carried from a source entry to its clone.
const CLONED_MODE_BITS: u32 = 0o777;

/// What one [`clone_tree`] did, for a single log line at the caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CloneStats {
    /// Files whose extents were shared with the source (clonefile / FICLONE).
    pub cloned_files: u64,
    /// Files copied because the filesystem cannot share extents.
    pub copied_files: u64,
    pub directories: u64,
    pub symlinks: u64,
    /// Special files, and entries that vanished or changed type mid-copy.
    pub skipped: u64,
}

impl std::ops::AddAssign for CloneStats {
    fn add_assign(&mut self, other: Self) {
        self.cloned_files += other.cloned_files;
        self.copied_files += other.copied_files;
        self.directories += other.directories;
        self.symlinks += other.symlinks;
        self.skipped += other.skipped;
    }
}

/// Clone every entry below `src` into the empty directory `dst`, then make
/// the result durable. `dst` keeps its own mode; each child gets its
/// source's permission bits.
pub fn clone_tree(src: &ContainedDir, dst: &ContainedDir) -> io::Result<CloneStats> {
    let mut stats = CloneStats::default();
    // Relative paths, re-walked from the roots: one descriptor per level is
    // held only while walking, so a deep guest tree cannot exhaust descriptors.
    let mut pending = vec![PathBuf::new()];
    // Directory modes are applied after the tree is filled, deepest first, so
    // a read-only source directory does not stop its own children being copied.
    let mut directory_modes = Vec::new();
    while let Some(rel) = pending.pop() {
        let from = match src.walk(&rel) {
            Ok(dir) => dir,
            Err(error) if is_race(&error) => {
                stats.skipped += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let to = dst.walk(&rel)?;
        for entry in from.entries()? {
            let name = entry.name.as_os_str();
            match clone_entry(&from, &to, name, entry.kind, &mut stats) {
                Ok(Some(mode)) => {
                    let child = rel.join(name);
                    directory_modes.push((child.clone(), mode));
                    pending.push(child);
                }
                Ok(None) => {}
                Err(error) if is_race(&error) => stats.skipped += 1,
                Err(error) => return Err(error),
            }
        }
    }
    for (rel, mode) in directory_modes.iter().rev() {
        let dir = dst.walk(rel)?;
        fs::set_mode(dir.as_fd(), *mode)?;
        fs::sync_before_barrier(dir.as_fd())?;
    }
    stats.directories = directory_modes.len() as u64;
    fs::sync_before_barrier(dst.as_fd())?;
    fs::sync_filesystem(dst.as_fd())?;
    Ok(stats)
}

/// Clone one regular file `name` of `src_dir` to the new entry `dst_name` of
/// `dst_dir`, ready for the caller's [`fs::sync_filesystem`] barrier.
pub fn clone_file(
    src_dir: &ContainedDir,
    name: &OsStr,
    dst_dir: &ContainedDir,
    dst_name: &OsStr,
) -> io::Result<CloneMethod> {
    let source = src_dir.open_file(name, ContainedOpenOptions::read_only())?;
    let mode = source.metadata()?.permissions().mode() & CLONED_MODE_BITS;
    let (clone, method) = fs::clone_file_into(&source, dst_dir, dst_name, mode)?;
    fs::sync_before_barrier(clone.as_fd())?;
    Ok(method)
}

/// Flush a regular file of `dir` without following a link. The descriptor is
/// read-only: flushing never needs write access.
pub fn sync_file(dir: &ContainedDir, name: &OsStr) -> io::Result<()> {
    fs::sync(dir.open_file(name, ContainedOpenOptions::read_only())?.as_fd())
}

/// Returns the directory's source mode when `name` is a directory to descend.
fn clone_entry(
    from: &ContainedDir,
    to: &ContainedDir,
    name: &OsStr,
    kind: EntryKind,
    stats: &mut CloneStats,
) -> io::Result<Option<u32>> {
    match kind {
        EntryKind::Directory => {
            let mode = from.descend(name)?.mode()? & CLONED_MODE_BITS;
            to.descend_or_create(name, 0o700)?;
            Ok(Some(mode))
        }
        EntryKind::File => {
            match clone_file(from, name, to, name)? {
                CloneMethod::SharedExtents => stats.cloned_files += 1,
                CloneMethod::SparseCopy => stats.copied_files += 1,
            }
            Ok(None)
        }
        EntryKind::Other => {
            match from.read_link(name)? {
                Some(target) => {
                    to.symlink(name, &target)?;
                    stats.symlinks += 1;
                }
                None => stats.skipped += 1,
            }
            Ok(None)
        }
    }
}

/// An entry the live guest removed, replaced by a link, or turned into
/// another type between the listing and the open.
fn is_race(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || is_symlink_refusal(error)
        || is_not_directory(error)
        // `open_file` refuses a non-regular file with InvalidInput.
        || error.kind() == io::ErrorKind::InvalidInput
}

#[cfg(test)]
mod tests;
