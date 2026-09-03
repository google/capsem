//! Descriptor-relative traversal beneath an untrusted directory.
//!
//! Every descent and file open is relative to an already-open directory and
//! refuses symlinks in the same syscall that would otherwise follow them.

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

/// The safe file-open shapes supported below a containment root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainedOpenOptions {
    write: bool,
    truncate: bool,
    mode: u32,
}

impl ContainedOpenOptions {
    pub const fn read_only() -> Self {
        Self {
            write: false,
            truncate: false,
            mode: 0,
        }
    }

    pub const fn write_create(mode: u32) -> Self {
        Self {
            write: true,
            truncate: false,
            mode,
        }
    }

    pub const fn write_create_truncate(mode: u32) -> Self {
        Self {
            write: true,
            truncate: true,
            mode,
        }
    }

    fn flags(self) -> OFlag {
        if self.write {
            let mut flags = OFlag::O_WRONLY | OFlag::O_CREAT;
            if self.truncate {
                flags |= OFlag::O_TRUNC;
            }
            flags
        } else {
            OFlag::O_RDONLY
        }
    }
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

/// Whether a path component expected to be a directory was another file type.
pub fn is_not_directory(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::ENOTDIR as i32)
}

fn dir_flags() -> OFlag {
    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
}

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
    // SAFETY: `fd` was just returned by a successful openat and is not owned
    // anywhere else.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

impl ContainedDir {
    /// Open the host-owned containment root.
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

    /// Open a child directory without following a link.
    pub fn descend(&self, name: &OsStr) -> io::Result<Self> {
        check_component(name)?;
        let fd = match openat(Some(self.fd.as_raw_fd()), name, dir_flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::ENOTDIR) if self.is_symlink(name)? => return Err(Errno::ELOOP.into()),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            fd: owned(fd),
            path: self.path.join(name),
        })
    }

    /// Descend into a child, creating it first when absent.
    pub fn descend_or_create(&self, name: &OsStr, mode: u32) -> io::Result<Self> {
        check_component(name)?;
        match mkdirat(Some(self.fd.as_raw_fd()), name, Mode::from_bits_truncate(mode)) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(error) => return Err(error.into()),
        }
        self.descend(name)
    }

    /// Walk `rel` one component at a time. An empty path clones this handle.
    pub fn walk(&self, rel: &Path) -> io::Result<Self> {
        self.walk_with(rel, |dir, name| dir.descend(name))
    }

    /// Walk `rel`, creating each absent directory with `mode`.
    pub fn walk_creating(&self, rel: &Path, mode: u32) -> io::Result<Self> {
        self.walk_with(rel, |dir, name| dir.descend_or_create(name, mode))
    }

    fn walk_with(&self, rel: &Path, step: impl Fn(&Self, &OsStr) -> io::Result<Self>) -> io::Result<Self> {
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
        let stat = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFLNK)
    }

    /// Inspect a child without following it.
    pub fn entry_kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
        check_component(name)?;
        match fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(kind_of(stat.st_mode))),
            Err(Errno::ENOENT) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Open a regular child without following links or blocking on a FIFO.
    pub fn open_file(&self, name: &OsStr, options: ContainedOpenOptions) -> io::Result<File> {
        check_component(name)?;
        let flags = options.flags() | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK;
        let fd = openat(
            Some(self.fd.as_raw_fd()),
            name,
            flags,
            Mode::from_bits_truncate(options.mode),
        )?;
        let file = File::from(owned(fd));
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", Path::new(name).display()),
            ));
        }
        Ok(file)
    }

    /// List children with metadata read without following links.
    pub fn entries(&self) -> io::Result<Vec<ContainedEntry>> {
        let mut directory = nix::dir::Dir::from_fd(self.fd.try_clone()?.into_raw_fd())?;
        let mut entries = Vec::new();
        for entry in directory.iter() {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let Ok(stat) = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW) else {
                continue;
            };
            entries.push(ContainedEntry {
                name: name.to_owned(),
                kind: kind_of(stat.st_mode),
                size: u64::try_from(stat.st_size).unwrap_or(0),
                mtime_secs: u64::try_from(stat.st_mtime).unwrap_or(0),
            });
        }
        Ok(entries)
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
