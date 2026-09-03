//! Directory handles that never follow a symlink.
//!
//! The files API operates on the VirtioFS-shared workspace, which the guest
//! writes at will. Any path-based check on that tree -- `canonicalize`,
//! `exists`, `metadata` -- is answered for one file and acted on for another
//! when the guest swaps a component for a symlink between the two syscalls.
//! `ContainedDir` walks by file descriptor instead: every descent is an
//! `openat(O_NOFOLLOW | O_DIRECTORY)` relative to the parent handle, every
//! file open carries `O_NOFOLLOW`, and listings read from the handle. A
//! symlink anywhere below the root is refused in the same syscall that would
//! have followed it, so there is no window to race.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{openat, AtFlags, OFlag};
use nix::sys::stat::{fstatat, mkdirat, Mode, SFlag};

/// A handle on one directory below the containment root.
#[derive(Debug)]
pub struct ContainedDir {
    fd: OwnedFd,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    /// Symlink, FIFO, socket, or device: never opened, never listed.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainedEntry {
    pub name: OsString,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_secs: u64,
}

/// `O_NOFOLLOW` on a symlink fails with `ELOOP` on Linux and macOS alike.
pub fn is_symlink_refusal(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::ELOOP as i32)
}

fn dir_flags() -> OFlag {
    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
}

/// One path component the guest may have named: no separators, no `.`/`..`.
fn check_component(name: &OsStr) -> io::Result<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid path component {:?}", name),
        ));
    }
    Ok(())
}

fn owned(fd: i32) -> OwnedFd {
    // SAFETY: `fd` was just returned by a successful openat and is owned by
    // nobody else.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

impl ContainedDir {
    /// Open the containment root. The root path is host-owned, so it may be
    /// canonicalized (macOS `/var` -> `/private/var`); everything below it is
    /// guest-influenced and is never resolved by path again.
    pub fn open_root(root: &Path) -> io::Result<Self> {
        let path = root.canonicalize()?;
        let fd = openat(None, &path, dir_flags(), Mode::empty())?;
        Ok(Self { fd: owned(fd), path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
            path: self.path.clone(),
        })
    }

    /// Open the child directory `name`. A symlink fails with `ELOOP`, a
    /// non-directory with `ENOTDIR`. Linux reports a symlink under
    /// `O_DIRECTORY | O_NOFOLLOW` as `ENOTDIR`; that is re-checked without
    /// following so callers see one refusal code on every platform.
    pub fn descend(&self, name: &OsStr) -> io::Result<Self> {
        check_component(name)?;
        let fd = match openat(Some(self.fd.as_raw_fd()), name, dir_flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::ENOTDIR) if self.is_symlink(name)? => return Err(Errno::ELOOP.into()),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            fd: owned(fd),
            path: self.path.join(name),
        })
    }

    /// `descend`, creating the directory first if nothing is there. A symlink
    /// already at `name` survives `mkdirat` as `EEXIST` and is then refused.
    pub fn descend_or_create(&self, name: &OsStr, mode: Mode) -> io::Result<Self> {
        check_component(name)?;
        match mkdirat(Some(self.fd.as_raw_fd()), name, mode) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => return Err(e.into()),
        }
        self.descend(name)
    }

    /// Walk `rel` one component at a time. An empty `rel` is this directory.
    pub fn walk(&self, rel: &Path) -> io::Result<Self> {
        self.walk_with(rel, |dir, name| dir.descend(name))
    }

    /// `walk`, creating each missing directory with `mode`.
    pub fn walk_creating(&self, rel: &Path, mode: Mode) -> io::Result<Self> {
        self.walk_with(rel, |dir, name| dir.descend_or_create(name, mode))
    }

    fn walk_with(&self, rel: &Path, step: impl Fn(&Self, &OsStr) -> io::Result<Self>) -> io::Result<Self> {
        // Every component is checked before the first syscall, so a `..`
        // late in the path cannot create or touch anything on the way to it.
        let names = rel
            .components()
            .map(|component| match component {
                Component::Normal(name) => Ok(name),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("relative path may only contain plain names: {}", rel.display()),
                )),
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut current = self.try_clone()?;
        for name in names {
            current = step(&current, name)?;
        }
        Ok(current)
    }

    fn is_symlink(&self, name: &OsStr) -> io::Result<bool> {
        let st = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(SFlag::from_bits_truncate(st.st_mode) & SFlag::S_IFMT == SFlag::S_IFLNK)
    }

    /// What sits at `name`, without following it. `None` when nothing does.
    pub fn entry_kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
        check_component(name)?;
        match fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(st) => Ok(Some(kind_of(st.st_mode))),
            Err(Errno::ENOENT) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Open the regular file `name` with `flags` plus `O_NOFOLLOW`. Anything
    /// that is not a regular file once open -- a FIFO, a device -- is refused;
    /// `O_NONBLOCK` keeps a FIFO from parking the caller first.
    pub fn open_file(&self, name: &OsStr, flags: OFlag, mode: Mode) -> io::Result<File> {
        check_component(name)?;
        let flags = flags | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK;
        let fd = openat(Some(self.fd.as_raw_fd()), name, flags, mode)?;
        let file = File::from(owned(fd));
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", Path::new(name).display()),
            ));
        }
        Ok(file)
    }

    /// The directory's entries with metadata read without following symlinks.
    pub fn entries(&self) -> io::Result<Vec<ContainedEntry>> {
        // fdopendir takes ownership of its fd and moves its offset, so read
        // through a duplicate.
        let mut dir = nix::dir::Dir::from_fd(self.fd.try_clone()?.into_raw_fd())?;
        let mut out = Vec::new();
        for entry in dir.iter() {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let Ok(st) = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW) else {
                continue;
            };
            out.push(ContainedEntry {
                name: name.to_owned(),
                kind: kind_of(st.st_mode),
                size: u64::try_from(st.st_size).unwrap_or(0),
                mtime_secs: u64::try_from(st.st_mtime).unwrap_or(0),
            });
        }
        Ok(out)
    }
}

fn kind_of(mode: nix::sys::stat::mode_t) -> EntryKind {
    match SFlag::from_bits_truncate(mode) & SFlag::S_IFMT {
        SFlag::S_IFDIR => EntryKind::Directory,
        SFlag::S_IFREG => EntryKind::File,
        _ => EntryKind::Other,
    }
}

#[cfg(test)]
mod tests;
