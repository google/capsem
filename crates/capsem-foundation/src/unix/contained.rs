//! Descriptor-relative traversal beneath an untrusted directory.
//!
//! Every descent and file open is relative to an already-open directory and
//! refuses symlinks in the same syscall that would otherwise follow them.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{openat, readlinkat, renameat, AtFlags, OFlag};
use nix::sys::stat::{fchmod, fchmodat, fstatat, mkdirat, FchmodatFlags, Mode, SFlag};
use nix::unistd::{linkat, symlinkat};
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
    exclusive: bool,
    mode: u32,
}

impl ContainedOpenOptions {
    pub const fn read_only() -> Self {
        Self {
            write: false,
            truncate: false,
            exclusive: false,
            mode: 0,
        }
    }

    pub const fn write_create(mode: u32) -> Self {
        Self {
            write: true,
            truncate: false,
            exclusive: false,
            mode,
        }
    }

    /// Create a file that must not already exist under any name or type,
    /// including a symlink or a dangling one.
    pub const fn write_create_new(mode: u32) -> Self {
        Self {
            write: true,
            truncate: false,
            exclusive: true,
            mode,
        }
    }

    pub const fn write_create_truncate(mode: u32) -> Self {
        Self {
            write: true,
            truncate: true,
            exclusive: false,
            mode,
        }
    }

    fn flags(self) -> OFlag {
        if self.write {
            let mut flags = OFlag::O_WRONLY | OFlag::O_CREAT;
            if self.truncate {
                flags |= OFlag::O_TRUNC;
            }
            if self.exclusive {
                flags |= OFlag::O_EXCL;
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
    /// Whether this `Other` entry is a symlink rather than a FIFO, socket or
    /// device. Reported, never followed.
    pub is_symlink: bool,
    pub size: u64,
    /// Bytes allocated on disk (`st_blocks * 512`): a sparse file's true cost.
    pub allocated: u64,
    pub mtime_secs: u64,
    pub identity: EntryIdentity,
}

/// What changes whenever an entry's content or metadata does. A writer can set
/// mtime back after an edit; it cannot set ctime or choose the inode, so two
/// equal identities mean the entry was not touched in between -- provided its
/// ctime is strictly older than the moment the first identity was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryIdentity {
    /// Inode numbers are unique only within this device.
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    /// (seconds, nanoseconds) since the epoch.
    pub mtime: (i64, i64),
    pub ctime: (i64, i64),
}

/// `O_NOFOLLOW` on a symlink fails with `ELOOP` on Linux and macOS alike.
pub fn is_symlink_refusal(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::ELOOP as i32)
}

/// Whether [`ContainedDir::hard_link`] failed because a hard link cannot be
/// made there at all -- another filesystem, one without hard links, or a link
/// count at its limit -- rather than because of the names involved.
pub fn is_link_unsupported(error: &io::Error) -> bool {
    [
        Errno::EXDEV,
        Errno::EPERM,
        Errno::ENOTSUP,
        Errno::EOPNOTSUPP,
        Errno::EMLINK,
    ]
    .iter()
    .any(|errno| error.raw_os_error() == Some(*errno as i32))
}

/// Whether a path component expected to be a directory was another file type.
pub fn is_not_directory(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::ENOTDIR as i32)
}

fn dir_flags() -> OFlag {
    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
}

pub(super) fn permission_mode(mode: u32) -> Mode {
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

impl AsFd for ContainedDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
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

    /// Metadata of the open directory, even if its path has been replaced.
    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        File::from(self.fd.try_clone()?).metadata()
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

    /// Permission bits of this directory, read from the open descriptor.
    pub fn mode(&self) -> io::Result<u32> {
        use std::os::unix::fs::PermissionsExt;
        let metadata = File::from(self.fd.try_clone()?).metadata()?;
        Ok(metadata.permissions().mode() & 0o7777)
    }

    /// Change one entry's permission bits below this open directory.
    ///
    /// Every parent component is opened without following links and the final
    /// operation uses `AT_SYMLINK_NOFOLLOW`, so a guest cannot redirect a
    /// metadata request outside the share between validation and mutation.
    /// An empty relative path names this directory itself.
    pub fn set_relative_mode(&self, rel: &Path, mode: u32) -> io::Result<()> {
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
        let Some((name, parents)) = names.split_last() else {
            return fchmod(self.fd.as_raw_fd(), permission_mode(mode)).map_err(Into::into);
        };
        let mut parent = self.try_clone()?;
        for component in parents {
            parent = parent.descend(component)?;
        }
        if parent.entry_kind(name)? == Some(EntryKind::Other) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file or directory", Path::new(name).display()),
            ));
        }
        fchmodat(
            Some(parent.fd.as_raw_fd()),
            *name,
            permission_mode(mode),
            FchmodatFlags::NoFollowSymlink,
        )
        .map_err(Into::into)
    }

    /// Create a symlink child. An existing entry of any type is an error; the
    /// target is stored verbatim and never resolved.
    pub fn symlink(&self, name: &OsStr, target: &OsStr) -> io::Result<()> {
        check_component(name)?;
        symlinkat(target, Some(self.fd.as_raw_fd()), name)?;
        Ok(())
    }

    /// Move the child `name` to `to_name` below `to`, replacing an existing
    /// entry there as rename(2) does. Neither name is resolved: a symlink
    /// moves as the link itself and its target is never touched.
    pub fn rename_to(&self, name: &OsStr, to: &ContainedDir, to_name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        check_component(to_name)?;
        renameat(Some(self.fd.as_raw_fd()), name, Some(to.fd.as_raw_fd()), to_name)?;
        Ok(())
    }

    /// Make `to_name` below `to` a second name for the regular file `name`.
    ///
    /// Neither name is resolved. A source that is not a regular file -- a
    /// link, FIFO or device -- is refused rather than linked, and an existing
    /// destination of any type is an error. `linkat` without
    /// `AT_SYMLINK_FOLLOW` links a symlink as itself, so a source swapped for
    /// one after the check is caught by the destination check and removed.
    ///
    /// Both names then share one inode: a writer of either writes the other.
    /// Link only files no untrusted writer can reach under either name.
    pub fn hard_link(&self, name: &OsStr, to: &ContainedDir, to_name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        check_component(to_name)?;
        let not_regular = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", Path::new(name).display()),
            )
        };
        if self.entry_kind(name)? != Some(EntryKind::File) {
            return Err(not_regular());
        }
        linkat(
            Some(self.fd.as_raw_fd()),
            name,
            Some(to.fd.as_raw_fd()),
            to_name,
            AtFlags::empty(),
        )?;
        if to.entry_kind(to_name)? != Some(EntryKind::File) {
            to.remove_non_directory(to_name)?;
            return Err(not_regular());
        }
        Ok(())
    }

    /// Remove the symlink child `name` without following it. Any other entry
    /// type is refused, so a caller retiring a link it made cannot delete
    /// whatever a writer put in its place.
    pub fn remove_symlink(&self, name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        if !self.is_symlink(name)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a symlink", Path::new(name).display()),
            ));
        }
        unlinkat(Some(self.fd.as_raw_fd()), name, UnlinkatFlags::NoRemoveDir).map_err(Into::into)
    }

    /// Remove the child `name` -- a file, a symlink or any other non-directory
    /// entry -- without following it. A directory is refused. For entries a
    /// guest writes, where a symlink in place of a file is removed as a link
    /// and its target is never touched.
    pub fn remove_non_directory(&self, name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        unlinkat(Some(self.fd.as_raw_fd()), name, UnlinkatFlags::NoRemoveDir).map_err(Into::into)
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

    /// Read a symlink's target relative to this directory; never follow it.
    /// Non-symlink entries return None, including FIFOs and devices.
    pub fn read_link(&self, name: &OsStr) -> io::Result<Option<OsString>> {
        check_component(name)?;
        match readlinkat(Some(self.fd.as_raw_fd()), name) {
            Ok(target) => Ok(Some(target)),
            Err(Errno::EINVAL) => Ok(None),
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
        let mut metadata = file.metadata()?;
        // Normal umasks leave an explicitly requested 0600 mode unchanged.
        // Avoid a redundant fchmod so sandboxed workers can create private
        // files while permission changes remain outside their authority.
        if metadata.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            metadata = file.metadata()?;
        }
        let uid = super::process::current_uid();
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o777 != 0o600
        {
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

    /// Remove the child `name` and, when it is a directory, everything below
    /// it, without following a link at any depth. A symlink is unlinked as
    /// the link itself, so its target is never touched. Directories are
    /// entered with `O_NOFOLLOW`: an entry swapped for a link between the
    /// inspection and the descent is refused, not followed.
    pub fn remove_tree(&self, name: &OsStr) -> io::Result<()> {
        check_component(name)?;
        let stat = fstatat(Some(self.fd.as_raw_fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        if kind_of(stat.st_mode) != EntryKind::Directory {
            return unlinkat(Some(self.fd.as_raw_fd()), name, UnlinkatFlags::NoRemoveDir).map_err(Into::into);
        }
        let child = self.descend(name)?;
        for entry in child.entries()? {
            child.remove_tree(&entry.name)?;
        }
        unlinkat(Some(self.fd.as_raw_fd()), name, UnlinkatFlags::RemoveDir).map_err(Into::into)
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
                is_symlink: SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFLNK,
                size: u64::try_from(stat.st_size).unwrap_or(0),
                allocated: u64::try_from(stat.st_blocks).unwrap_or(0).saturating_mul(512),
                mtime_secs: u64::try_from(stat.st_mtime).unwrap_or(0),
                identity: identity_of(&stat),
            })? {
                break;
            }
        }
        Ok(())
    }
}

fn identity_of(stat: &nix::sys::stat::FileStat) -> EntryIdentity {
    #[cfg(target_os = "macos")]
    let dev = u64::from(stat.st_dev.cast_unsigned());
    #[cfg(not(target_os = "macos"))]
    let dev = stat.st_dev;
    EntryIdentity {
        dev,
        ino: stat.st_ino,
        size: u64::try_from(stat.st_size).unwrap_or(0),
        mtime: (stat.st_mtime, stat.st_mtime_nsec),
        ctime: (stat.st_ctime, stat.st_ctime_nsec),
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
