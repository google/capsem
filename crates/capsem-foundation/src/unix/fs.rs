//! Filesystem operations with owner-only creation and atomic publication.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} has no parent directory", path.display()),
            )
        })?;
    let (mut file, temporary) = create_private_sibling(path)?;
    let write_result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result.map_err(|error| context(error, "atomically write private file", path))
}

fn create_private_sibling(path: &Path) -> io::Result<(File, PathBuf)> {
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
                if let Err(error) = file.set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE)) {
                    drop(file);
                    let _ = std::fs::remove_file(&temporary);
                    return Err(error);
                }
                return Ok((file, temporary));
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
