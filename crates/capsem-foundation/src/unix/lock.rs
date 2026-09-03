//! Race-safe advisory file locks.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::ops::Deref;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};

use super::errno;

/// Compatibility class for an advisory lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockMode {
    Shared,
    Exclusive,
}

/// Result of one non-blocking acquisition attempt.
#[derive(Debug)]
pub enum LockAttempt {
    Acquired(FileLock),
    Contended,
}

/// A held advisory lock, released when dropped.
#[derive(Debug)]
pub struct FileLock {
    flock: Option<Flock<File>>,
    path: PathBuf,
    registry_key: PathBuf,
    mode: LockMode,
}

impl FileLock {
    /// The user-supplied backing path, for caller-owned diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AsRawFd for FileLock {
    fn as_raw_fd(&self) -> RawFd {
        self.flock.as_ref().expect("held lock missing descriptor").as_raw_fd()
    }
}

impl AsFd for FileLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.flock.as_ref().expect("held lock missing descriptor").as_fd()
    }
}

impl Deref for FileLock {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        self.flock.as_ref().expect("held lock missing descriptor")
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        drop(self.flock.take());
        release_reservation(&self.registry_key, self.mode);
    }
}

#[derive(Default)]
struct Reservation {
    readers: usize,
    writer: bool,
}

fn held_locks() -> &'static Mutex<HashMap<PathBuf, Reservation>> {
    static HELD: OnceLock<Mutex<HashMap<PathBuf, Reservation>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Try to acquire a shared or exclusive lock without waiting.
///
/// The lockfile is created mode 0600 with `O_CLOEXEC | O_NOFOLLOW`; an
/// existing file is verified as current-user-owned and tightened to 0600
/// through the opened descriptor. The acquired descriptor's device/inode is
/// compared with the path after `flock`, closing the open-to-lock replacement
/// race. Expected contention is a typed outcome rather than an IO error.
pub fn try_acquire(path: &Path, mode: LockMode) -> io::Result<LockAttempt> {
    try_acquire_after_open(path, mode, || {})
}

/// Acquire a shared or exclusive lock, waiting until contention clears.
///
/// This preserves the same path-replacement and ownership checks as
/// [`try_acquire`]. It is intended for synchronous read-modify-write sections.
pub fn acquire(path: &Path, mode: LockMode) -> io::Result<FileLock> {
    loop {
        match try_acquire(path, mode)? {
            LockAttempt::Acquired(lock) => return Ok(lock),
            LockAttempt::Contended => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}

fn try_acquire_inner(path: &Path, mode: LockMode, after_open: impl FnOnce()) -> io::Result<LockAttempt> {
    let registry_key = prepare_key(path)?;
    if !reserve(&registry_key, mode)? {
        return Ok(LockAttempt::Contended);
    }

    let result = acquire_reserved(path, mode, after_open);
    if !matches!(&result, Ok(LockAttempt::Acquired(_))) {
        release_reservation(&registry_key, mode);
    }
    result.map(|attempt| match attempt {
        LockAttempt::Acquired(mut lock) => {
            lock.registry_key = registry_key;
            LockAttempt::Acquired(lock)
        }
        LockAttempt::Contended => LockAttempt::Contended,
    })
}

fn acquire_reserved(path: &Path, mode: LockMode, after_open: impl FnOnce()) -> io::Result<LockAttempt> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("lock path {} is not a regular file", path.display()),
        ));
    }
    let uid = super::process::current_uid();
    if metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "lock path {} is owned by uid {}, not {uid}",
                path.display(),
                metadata.uid()
            ),
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    after_open();

    let argument = match mode {
        LockMode::Shared => FlockArg::LockSharedNonblock,
        LockMode::Exclusive => FlockArg::LockExclusiveNonblock,
    };
    let mut candidate = file;
    let flock = loop {
        match Flock::lock(candidate, argument) {
            Ok(flock) => break flock,
            Err((file, Errno::EINTR)) => candidate = file,
            Err((_file, Errno::EWOULDBLOCK)) => return Ok(LockAttempt::Contended),
            Err((_file, error)) => return Err(errno::io(error)),
        }
    };
    let current = std::fs::symlink_metadata(path)?;
    if current.file_type().is_symlink() || current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("lock path {} changed during acquisition", path.display()),
        ));
    }

    Ok(LockAttempt::Acquired(FileLock {
        flock: Some(flock),
        path: path.to_path_buf(),
        registry_key: PathBuf::new(),
        mode,
    }))
}

fn prepare_key(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let parent = absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("lock path {} has no parent", path.display()),
            )
        })?;
    std::fs::create_dir_all(parent)?;
    let parent = std::fs::canonicalize(parent)?;
    let name = absolute.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("lock path {} has no file name", path.display()),
        )
    })?;
    Ok(parent.join(name))
}

fn reserve(key: &Path, mode: LockMode) -> io::Result<bool> {
    let mut held = held_locks()
        .lock()
        .map_err(|_| io::Error::other("file-lock reservation registry is poisoned"))?;
    let reservation = held.entry(key.to_path_buf()).or_default();
    let available = match mode {
        LockMode::Shared => !reservation.writer,
        LockMode::Exclusive => !reservation.writer && reservation.readers == 0,
    };
    if available {
        match mode {
            LockMode::Shared => reservation.readers += 1,
            LockMode::Exclusive => reservation.writer = true,
        }
    } else if reservation.readers == 0 && !reservation.writer {
        held.remove(key);
    }
    drop(held);
    Ok(available)
}

fn release_reservation(key: &Path, mode: LockMode) {
    let Ok(mut held) = held_locks().lock() else {
        return;
    };
    let Some(reservation) = held.get_mut(key) else {
        return;
    };
    match mode {
        LockMode::Shared => reservation.readers = reservation.readers.saturating_sub(1),
        LockMode::Exclusive => reservation.writer = false,
    }
    if reservation.readers == 0 && !reservation.writer {
        held.remove(key);
    }
}

fn try_acquire_after_open(path: &Path, mode: LockMode, after_open: impl FnOnce()) -> io::Result<LockAttempt> {
    try_acquire_inner(path, mode, after_open)
}

#[cfg(test)]
mod tests;
