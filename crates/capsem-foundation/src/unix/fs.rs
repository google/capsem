//! Filesystem operations with owner-only creation and atomic publication.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod durability;
mod extents;

pub use durability::{set_mode, sync, sync_before_barrier, sync_filesystem};
pub use extents::{clone_file_into, copy_sparse, CloneMethod};

const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Capacity figures for the filesystem containing a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesystemSpace {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
}

/// Read filesystem capacity without exposing platform `statvfs` types.
pub fn filesystem_space(path: &Path) -> io::Result<FilesystemSpace> {
    let stat = nix::sys::statvfs::statvfs(path).map_err(super::errno::io)?;
    let block_size = stat.block_size();
    Ok(FilesystemSpace {
        total_bytes: bytes_for_blocks(stat.blocks(), block_size),
        free_bytes: bytes_for_blocks(stat.blocks_free(), block_size),
        available_bytes: bytes_for_blocks(stat.blocks_available(), block_size),
    })
}

fn bytes_for_blocks<T: Into<u64>>(blocks: T, block_size: u64) -> u64 {
    blocks.into().saturating_mul(block_size)
}

/// Open a regular file for reading without following a link or blocking on a
/// special file.
///
/// The handle is for callers that seek and read spans rather than slurping the
/// file: reading it whole is `read_regular_file_no_follow`, which is this plus
/// a `read_to_end`.
pub fn open_regular_file_no_follow(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| context(error, "open regular file without following links", path))?;
    require_regular_file(&file, path)?;
    Ok(file)
}

/// Open an owner-only regular file for appending, creating it if absent,
/// without following a link.
///
/// The handle reads and seeks as well as appends: append-mode writes go to the
/// end whatever the read cursor is doing, which is what an append-only log
/// wants. A file created here is 0o600, and an existing one is opened only if
/// it is a regular file -- a symlink or a device planted at the path is
/// refused rather than written through.
pub fn open_private_append_no_follow(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| context(error, "open private append-only file", path))?;
    require_regular_file(&file, path)?;
    Ok(file)
}

fn require_regular_file(file: &File, path: &Path) -> io::Result<()> {
    if file.metadata()?.is_file() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} is not a regular file", path.display()),
    ))
}

/// Read a regular file without following a link or blocking on a special file.
pub fn read_regular_file_no_follow(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_regular_file_no_follow(path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|error| context(error, "read regular file", path))?;
    Ok(contents)
}

/// Write and sync a newly-created regular file without following a link.
pub fn write_new_regular_file_no_follow(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| context(error, "create regular file without following links", path))?;
    file.write_all(data)
        .map_err(|error| context(error, "write regular file", path))?;
    file.sync_all()
        .map_err(|error| context(error, "sync regular file", path))
}

/// Persist a regular file's bytes and length using the platform's strongest
/// supported local-filesystem barrier.
pub fn durable_sync_file(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        loop {
            // SAFETY: `file` owns a live descriptor and F_FULLFSYNC does not
            // retain the integer after this synchronous call.
            let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };
            if result == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        file.sync_all()
    }
}

/// Persist directory-entry creation, replacement, or deletion.
pub fn durable_sync_directory(path: &Path) -> io::Result<()> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
        .map_err(|error| context(error, "open directory for durable sync", path))?;
    directory
        .sync_all()
        .map_err(|error| context(error, "durably sync directory", path))
}

/// Create `path` as an owner-only directory, or verify an existing one.
///
/// Symlinks, non-directories, foreign owners, and group/other permissions are
/// refused. The parent is deliberately not created implicitly: callers must
/// choose and establish the trust boundary above this directory.
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(PRIVATE_DIR_MODE).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(context(error, "create private directory", path)),
    }

    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| context(error, "inspect private directory", path))?;
    if metadata.file_type().is_symlink() {
        return Err(refused(path, "it is a symlink"));
    }
    if !metadata.is_dir() {
        return Err(refused(path, "it is not a directory"));
    }
    let uid = super::process::current_uid();
    if metadata.uid() != uid {
        return Err(refused(
            path,
            &format!("it is owned by uid {}, not {uid}", metadata.uid()),
        ));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != PRIVATE_DIR_MODE {
        return Err(refused(path, &format!("its mode is {mode:o}, not 700")));
    }
    Ok(())
}

/// Atomically replace `path` with owner-only `data`.
///
/// Bytes are written and synced through a unique sibling opened with
/// `O_CLOEXEC | O_NOFOLLOW | O_EXCL`, then renamed over the destination. A
/// reader therefore observes either the previous complete file or the new
/// complete file, never a partial write or a permissive chmod window.
pub fn atomic_write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut sibling = create_private_sibling(path)?;
    let write_result = (|| {
        sibling.file().write_all(data)?;
        sibling.file().sync_all()?;
        rename_private_sibling(sibling, path)
    })();
    write_result.map_err(|error| context(error, "atomically write private file", path))
}

/// Rename a private sibling over `destination` and make the rename durable.
///
/// The directory entry is fsynced, not only the file's contents. Without it
/// the rename can still be in the page cache when a caller that committed
/// something *else* durably -- a SQLite transaction naming the new file's
/// contents -- has already returned: after a power loss the commit is there
/// and the rename is not, and nothing afterwards would ever notice.
///
/// Consumes the sibling, so a caller cannot both rename it and have it
/// removed by the guard; a failed rename removes it, leaving the destination
/// exactly as it was.
pub fn rename_private_sibling(sibling: PrivateSibling, destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} has no parent directory", destination.display()),
            )
        })?
        .to_path_buf();
    let temporary = sibling.keep();
    let renamed = std::fs::rename(&temporary, destination).and_then(|()| File::open(&parent)?.sync_all());
    if renamed.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    renamed
}

/// A unique, owner-only sibling file, removed when dropped.
///
/// The guard is the point: a temporary is created to become some other file,
/// and every path that does not reach the rename -- an error, an early
/// return, a panic unwinding through the caller -- must not leave it behind.
/// Cleanup written as an `if result.is_err()` at the end of a function is
/// cleanup that a panic walks straight past.
#[derive(Debug)]
pub struct PrivateSibling {
    file: File,
    path: PathBuf,
    /// Set by `keep`, which hands the path to a caller that is about to
    /// rename it into place.
    kept: bool,
}

impl PrivateSibling {
    /// The open handle, for writing and syncing the contents.
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }

    /// Where it is, while it is still a temporary.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Give up ownership: the file is no longer removed on drop, and the
    /// caller owns the path. Used by whoever is about to rename it.
    #[must_use]
    pub fn keep(mut self) -> PathBuf {
        self.kept = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for PrivateSibling {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Create a unique, owner-only, write-only sibling of `path`, ready to be
/// renamed over it.
///
/// Public so that callers who rewrite a large file cannot be forced to hold
/// its whole contents in memory to get `atomic_write_private`'s guarantees:
/// they stream into this handle and rename it with `rename_private_sibling`.
/// The name is dot-prefixed and carries the pid and a process-unique
/// sequence, and the open is `O_EXCL | O_NOFOLLOW`, so two concurrent writers
/// never share one. The temporary removes itself unless it is renamed.
pub fn create_private_sibling(path: &Path) -> io::Result<PrivateSibling> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} has no parent directory", path.display()),
            )
        })?;
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no file name", path.display()),
        )
    })?;

    loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".tmp.{}.{sequence}", std::process::id()));
        let temporary = parent.join(temporary_name);
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary);
        match opened {
            Ok(file) => {
                let sibling = PrivateSibling {
                    file,
                    path: temporary,
                    kept: false,
                };
                // Through the handle, and before the guard could hand it out:
                // a mode the filesystem refuses fails the create, and the
                // guard removes the file on the way out.
                sibling
                    .file
                    .set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
                return Ok(sibling);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

fn refused(path: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("refusing private path {}: {reason}", path.display()),
    )
}

fn context(error: io::Error, operation: &str, path: &Path) -> io::Error {
    io::Error::new(error.kind(), format!("{operation} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests;
