//! Descriptor-relative traversal beneath an untrusted directory.
//!
//! Every descent and file open is relative to an already-open directory and
//! refuses symlinks in the same syscall that would otherwise follow them.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{openat, AtFlags, OFlag};
use nix::sys::stat::{fstatat, mkdirat, Mode, SFlag};
use nix::unistd::{unlinkat, UnlinkatFlags};

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

fn permission_mode(mode: u32) -> Mode {
    let bits = mode & 0o7777;
    Mode::from_bits_truncate(permission_bits(bits))
}

fn permission_bits<T>(bits: u32) -> T
where
    T: TryFrom<u32>,
    T::Error: std::fmt::Debug,
{
    bits.try_into().expect("Unix permission bits fit mode_t")
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

    /// Verify that this opened descriptor names a current-user-owned mode
    /// 0700 directory. The check is descriptor-based, so replacing the path
    /// after open cannot redirect later `openat` operations.
    pub fn validate_private(&self) -> io::Result<()> {
        let metadata = File::from(self.fd.try_clone()?).metadata()?;
        let uid = super::process::current_uid();
        let mode = metadata.permissions().mode() & 0o777;
        if !metadata.is_dir() || metadata.uid() != uid || mode != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "directory {} is not private mode 0700 owned by uid {uid}",
                    self.path.display()
                ),
            ));
        }
        Ok(())
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
        match mkdirat(Some(self.fd.as_raw_fd()), name, permission_mode(mode)) {
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
        let fd = openat(Some(self.fd.as_raw_fd()), name, flags, permission_mode(options.mode))?;
        let file = File::from(owned(fd));
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", Path::new(name).display()),
            ));
        }
        Ok(file)
    }

    /// Exclusively create an owner-only regular child for read/append access.
    ///
    /// The directory descriptor anchors the create even if an attacker swaps
    /// a pathname above it. `O_EXCL | O_NOFOLLOW` prevents reuse or link
    /// traversal, and the returned descriptor is independently owned.
    pub fn create_new_private_file(&self, name: &OsStr) -> io::Result<File> {
        check_component(name)?;
        let flags = OFlag::O_RDWR
            | OFlag::O_APPEND
            | OFlag::O_CREAT
            | OFlag::O_EXCL
            | OFlag::O_NOFOLLOW
            | OFlag::O_CLOEXEC
            | OFlag::O_NONBLOCK;
        let fd = openat(Some(self.fd.as_raw_fd()), name, flags, permission_mode(0o600))?;
        let file = File::from(owned(fd));
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let metadata = file.metadata()?;
        let uid = super::process::current_uid();
        if !metadata.is_file() || metadata.uid() != uid || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refusing newly created private file {}", Path::new(name).display()),
            ));
        }
        Ok(file)
    }

    /// Open an existing owner-only regular child for read/append access.
    pub fn open_existing_private_append(&self, name: &OsStr) -> io::Result<File> {
        check_component(name)?;
        let flags = OFlag::O_RDWR | OFlag::O_APPEND | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK;
        let fd = openat(Some(self.fd.as_raw_fd()), name, flags, Mode::empty())?;
        let file = File::from(owned(fd));
        let metadata = file.metadata()?;
        let uid = super::process::current_uid();
        let mode = metadata.permissions().mode() & 0o777;
        if !metadata.is_file() || metadata.uid() != uid || metadata.nlink() != 1 || mode != 0o600 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "existing private file {} is not a mode 0600 current-user regular file with one link",
                    Path::new(name).display()
                ),
            ));
        }
        Ok(file)
    }

    /// Remove one current-user owner-only regular child without following or
    /// accepting extra links.
    pub fn remove_private_file(&self, name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        let stat = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        let mode = stat.st_mode & 0o777;
        let uid = super::process::current_uid();
        if kind_of(stat.st_mode) != EntryKind::File || stat.st_uid != uid || stat.st_nlink != 1 || mode != 0o600 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to remove non-private file {} (uid {}, mode {mode:o}, links {})",
                    Path::new(name).display(),
                    stat.st_uid,
                    stat.st_nlink
                ),
            ));
        }
        unlinkat(Some(self.fd.as_raw_fd()), name, UnlinkatFlags::NoRemoveDir).map_err(Into::into)
    }

    /// Persist namespace changes made through this directory.
    pub fn sync(&self) -> io::Result<()> {
        File::from(self.fd.try_clone()?).sync_all()
    }

    /// List children with metadata read without following links.
    pub fn entries(&self) -> io::Result<Vec<ContainedEntry>> {
        let mut entries = Vec::new();
        self.visit_entries(|entry| {
            entries.push(entry);
            Ok(true)
        })?;
        Ok(entries)
    }

    /// Visit children one at a time without following links.
    ///
    /// Returning `false` stops the walk. Callers that inspect or clean a
    /// potentially large managed directory can therefore keep constant
    /// memory rather than materializing every name first.
    pub fn visit_entries(&self, mut visit: impl FnMut(ContainedEntry) -> io::Result<bool>) -> io::Result<()> {
        let mut directory = nix::dir::Dir::from_fd(self.fd.try_clone()?.into_raw_fd())?;
        for entry in directory.iter() {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let Ok(stat) = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW) else {
                continue;
            };
            if !visit(ContainedEntry {
                name: name.to_owned(),
                kind: kind_of(stat.st_mode),
                size: u64::try_from(stat.st_size).unwrap_or(0),
                mtime_secs: u64::try_from(stat.st_mtime).unwrap_or(0),
            })? {
                break;
            }
        }
        Ok(())
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
