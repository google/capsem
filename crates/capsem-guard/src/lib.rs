//! Companion process lifecycle guards for capsem.
//!
//! Two primitives, applied together, make companion processes (capsem-gateway,
//! capsem-tray) non-standalone and self-bounded to their parent service:
//!
//! 1. [`is_alive`] / [`watch_parent_or_exit`] -- check/monitor a parent PID.
//!    Companions accept `--parent-pid` at startup. If the PID is missing or
//!    already dead, the companion refuses to start (caller exits 0). While
//!    running, a background thread polls the parent and terminates the
//!    companion the moment the parent disappears -- even on SIGKILL, OOM, or
//!    test-harness interruption, where graceful shutdown never fires.
//!
//! 2. [`Singleton`] -- an `flock(2)`-based global lock. At most one companion
//!    of a given kind exists system-wide. A second instance acquires nothing
//!    and exits 0. The kernel releases the lock when the holder's fd closes
//!    (including on crash), so stuck lockfiles never wedge future startups.
//!
//! Together these turn tray + gateway into bind-to-parent children: the only
//! legitimate spawn path is via the service, and they cannot outlive it.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;
use tracing::{info, warn};

/// How often the parent-watch loop polls for parent death. Must stay well
/// under `_ensure-service`'s 500 ms restart budget so that a SIGKILL'd
/// service's companions exit before the next service tries to bind the
/// same TCP port. `getppid()` is a cheap vDSO call -- 100 ms of polling
/// overhead is negligible.
const PARENT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("parent pid not provided")]
    NoParent,
    #[error("parent pid {0} is not alive at startup")]
    ParentDead(u32),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Returns true iff `pid` belongs to an existing (possibly zombie) process
/// that we have permission to probe. Used for pre-flight checks.
pub fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: kill with sig=0 performs error-checking only, never delivers a
    // signal. Safe regardless of pid value.
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return true;
    }
    // errno == EPERM means the pid exists but is owned by another uid; still
    // "alive" for our purposes. Only ESRCH means truly gone.
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    errno == libc::EPERM
}

/// Returns the current process's parent PID.
fn current_ppid() -> u32 {
    // SAFETY: getppid is always safe and cannot fail.
    unsafe { libc::getppid() as u32 }
}

/// True while we are still an active child of `expected_parent_pid`.
///
/// This uses `getppid()` and is immune to zombie state and to pid reuse of
/// the original parent. The kernel re-parents orphaned children to init
/// (PID 1) the moment the real parent's exit is reported, whether the real
/// parent is reaped yet or not.
pub fn parent_is_expected(expected_parent_pid: u32) -> bool {
    if expected_parent_pid == 0 {
        return false;
    }
    let ppid = current_ppid();
    ppid == expected_parent_pid && ppid != 1
}

/// Verify the given parent PID is our actual parent and spawn a background
/// thread that terminates the current process the moment we are re-parented
/// away from it (i.e. the parent dies or we were never its child).
///
/// Returns immediately on success. On failure (no PID, parent dead, or we're
/// not actually a child of that PID) returns `Err` -- the caller is expected
/// to exit 0 so that test harnesses and dev launches don't leave companions
/// running without a service.
///
/// The watcher calls `std::process::exit(0)`, not a graceful shutdown: there
/// is no legitimate work left once the service is gone.
pub fn watch_parent_or_exit(parent_pid: Option<u32>) -> Result<(), GuardError> {
    let Some(ppid) = parent_pid else {
        return Err(GuardError::NoParent);
    };
    if !parent_is_expected(ppid) {
        return Err(GuardError::ParentDead(ppid));
    }
    spawn_watcher(ppid, PARENT_POLL_INTERVAL, || std::process::exit(0));
    info!(parent_pid = ppid, "parent watch armed");
    Ok(())
}

/// Internal helper used by the real `watch_parent_or_exit` and by tests.
/// Tests inject a custom terminator so they can observe the effect without
/// exiting the test runner.
fn spawn_watcher<F>(parent_pid: u32, interval: Duration, terminator: F)
where
    F: Fn() + Send + 'static,
{
    thread::Builder::new()
        .name(format!("capsem-guard-watch-{parent_pid}"))
        .spawn(move || loop {
            if !parent_is_expected(parent_pid) {
                warn!(
                    parent_pid,
                    current_ppid = current_ppid(),
                    "parent gone or reparented; terminating companion"
                );
                terminator();
                return;
            }
            thread::sleep(interval);
        })
        .expect("failed to spawn parent-watch thread");
}

/// Process-wide registry of in-flight Singleton paths. Covers the window
/// between lock request and flock release where other kernel-level state
/// (fork-inherited fds) could otherwise keep the lock alive.
fn held_locks() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    use std::sync::OnceLock;
    static HELD: OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> = OnceLock::new();
    HELD.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// System-wide singleton guard backed by `flock(2)` plus an in-process
/// registry. Holds the lock for the lifetime of the struct; dropping it (or
/// process exit) releases it.
pub struct Singleton {
    // Kept alive for its Drop: closing the fd releases the flock.
    _file: File,
    path: PathBuf,
    canonical: PathBuf,
}

impl Drop for Singleton {
    fn drop(&mut self) {
        if let Ok(mut held) = held_locks().lock() {
            held.remove(&self.canonical);
        }
    }
}

impl Singleton {
    /// Attempt to acquire a non-blocking exclusive flock on `lock_path`.
    ///
    /// * `Ok(Some(guard))` -- we won; we are the sole instance.
    /// * `Ok(None)` -- another process already holds the lock; caller exits 0.
    /// * `Err(_)` -- a real IO error (permissions, missing parent dir we could
    ///   not create, etc.). The caller should fail loudly.
    pub fn try_acquire(lock_path: &Path) -> Result<Option<Self>, GuardError> {
        Self::try_acquire_inner(lock_path, true)
    }

    fn try_acquire_inner(
        lock_path: &Path,
        break_stale_pid_lock: bool,
    ) -> Result<Option<Self>, GuardError> {
        if let Some(parent) = lock_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| GuardError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }

        // Open with O_CLOEXEC set ATOMICALLY at open() time. Setting CLOEXEC
        // post-hoc via fcntl has a race window where a concurrent fork/exec
        // in another thread leaks the fd into the child; a child that
        // inherits this fd keeps the flock alive in the kernel even after we
        // close our own copy (flock(2) locks are file-scoped on BSD/macOS
        // and shared across dup'd fds from fork).
        // SAFETY: libc::open with a valid CString path and standard flags.
        use std::ffi::CString;
        use std::os::fd::FromRawFd;
        let c_path =
            CString::new(lock_path.as_os_str().as_encoded_bytes()).map_err(|_| GuardError::Io {
                path: lock_path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "lock path contains a NUL byte",
                ),
            })?;
        let raw_fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC,
                0o644,
            )
        };
        if raw_fd < 0 {
            return Err(GuardError::Io {
                path: lock_path.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }
        // SAFETY: we just opened this fd successfully.
        let file: File = unsafe { File::from_raw_fd(raw_fd) };

        // In-process exclusion: if another thread in this process already
        // holds a Singleton on the canonical path, refuse without touching
        // the file lock. flock alone is not sufficient for same-process
        // mutual exclusion: a subprocess spawn in another thread can cause
        // our fd to briefly leak through the fork-to-exec window and keep
        // the kernel lock alive after we close our copy, causing spurious
        // reacquire failures.
        let canonical =
            std::fs::canonicalize(lock_path).unwrap_or_else(|_| lock_path.to_path_buf());
        {
            let mut held = held_locks().lock().expect("held-locks mutex poisoned");
            if held.contains(&canonical) {
                return Ok(None);
            }
            // Reserve the slot before the syscall so racing threads in this
            // process see "taken" even before flock returns.
            held.insert(canonical.clone());
        }

        // Kernel-level cross-process exclusion via flock(2). CLOEXEC above
        // keeps the fd from leaking into exec'd children; any brief fork-to-
        // exec window is covered by the in-process registry we just updated.
        // SAFETY: flock signature; LOCK_EX|LOCK_NB are valid flag bits.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // Give the reservation back so a future retry can succeed.
            held_locks()
                .lock()
                .expect("held-locks mutex poisoned")
                .remove(&canonical);
            if errno == libc::EWOULDBLOCK {
                if break_stale_pid_lock && lockfile_stamped_pid_is_dead(lock_path) {
                    drop(file);
                    let _ = std::fs::remove_file(lock_path);
                    return Self::try_acquire_inner(lock_path, false);
                }
                return Ok(None);
            }
            return Err(GuardError::Io {
                path: lock_path.to_path_buf(),
                source: std::io::Error::from_raw_os_error(errno),
            });
        }

        // Best-effort pid stamp for debuggability. The lock, not the file
        // contents, is the source of truth.
        use std::io::{Seek, SeekFrom, Write};
        let _ = (&file).seek(SeekFrom::Start(0));
        let payload = format!("{}\n", std::process::id());
        let _ = (&file).write_all(payload.as_bytes());
        let _ = file.set_len(payload.len() as u64);

        Ok(Some(Self {
            _file: file,
            path: lock_path.to_path_buf(),
            canonical,
        }))
    }

    /// Path of the backing lockfile (informational, for logs).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn lockfile_stamped_pid_is_dead(lock_path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(lock_path) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return false;
    };
    !is_alive(pid)
}

/// Convenience: install both guards in one call. Returns `None` if either
/// bounce condition is hit (no parent, parent dead, singleton already held)
/// so the caller can `match` and exit 0.
pub struct InstalledGuards {
    _singleton: Singleton,
}

/// Arm parent-watch + acquire singleton lock. Intended startup call for
/// every companion process.
///
/// Returns:
/// * `Ok(Some(_))` -- guards active; caller should proceed with normal startup.
/// * `Ok(None)` -- another instance already owns the singleton lock; caller
///   should exit 0 (this is the "fast-probe passthrough" path for tests and
///   concurrent spawns).
/// * `Err(_)` -- parent missing/dead, or real IO error. Caller should exit 0
///   for the parent cases (they're expected when someone runs the binary
///   standalone) and fail loudly for IO.
pub fn install(
    parent_pid: Option<u32>,
    lock_path: &Path,
) -> Result<Option<InstalledGuards>, GuardError> {
    watch_parent_or_exit(parent_pid)?;
    match Singleton::try_acquire(lock_path)? {
        Some(s) => Ok(Some(InstalledGuards { _singleton: s })),
        None => Ok(None),
    }
}

/// Helper to parse `--parent-pid` style args. Accepts `None` and strings.
pub fn parse_parent_pid(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&p| p > 0)
}

/// Spawn a watcher that calls `terminator` when we are re-parented away from
/// `parent_pid`. Exposed for tests and for callers that need a non-exiting
/// reaction (e.g. to trigger a graceful flush before exit).
pub fn watch_parent_with<F>(parent_pid: u32, interval: Duration, terminator: F) -> Arc<AtomicBool>
where
    F: Fn() + Send + 'static,
{
    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = Arc::clone(&fired);
    thread::Builder::new()
        .name(format!("capsem-guard-watch-{parent_pid}"))
        .spawn(move || loop {
            if !parent_is_expected(parent_pid) {
                fired_clone.store(true, Ordering::Release);
                terminator();
                return;
            }
            thread::sleep(interval);
        })
        .expect("failed to spawn parent-watch thread");
    fired
}

#[cfg(test)]
mod tests;
