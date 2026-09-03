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
    #[error("failed to spawn parent-watch thread: {source}")]
    WatcherSpawn {
        #[source]
        source: std::io::Error,
    },
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
    use capsem_foundation::unix::process::{probe, ProcessId, ProcessState};
    let Ok(pid) = ProcessId::try_from(pid) else {
        return false;
    };
    match probe(pid) {
        Ok(ProcessState::Alive) => true,
        Ok(ProcessState::Gone) => false,
        Err(error) => {
            warn!(
                operation = "guard-parent-liveness-probe",
                pid = pid.get(),
                errno = error.raw_os_error(),
                error = %error,
                "parent liveness probe failed"
            );
            false
        }
    }
}

/// Returns the current process's parent PID.
fn current_ppid() -> u32 {
    capsem_foundation::unix::process::parent_process_id()
        .map(capsem_foundation::unix::process::ProcessId::get)
        .unwrap_or(0)
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
    spawn_watcher(ppid, PARENT_POLL_INTERVAL, || std::process::exit(0))?;
    info!(parent_pid = ppid, "parent watch armed");
    Ok(())
}

/// Internal helper used by the real `watch_parent_or_exit` and by tests.
/// Tests inject a custom terminator so they can observe the effect without
/// exiting the test runner.
type WatcherTask = Box<dyn FnOnce() + Send + 'static>;

fn spawn_watcher<F>(parent_pid: u32, interval: Duration, terminator: F) -> Result<(), GuardError>
where
    F: Fn() + Send + 'static,
{
    spawn_watcher_with(parent_pid, interval, terminator, |builder, task| builder.spawn(task))
}

fn spawn_watcher_with<F, S>(parent_pid: u32, interval: Duration, terminator: F, spawn: S) -> Result<(), GuardError>
where
    F: Fn() + Send + 'static,
    S: FnOnce(thread::Builder, WatcherTask) -> std::io::Result<thread::JoinHandle<()>>,
{
    let task: WatcherTask = Box::new(move || loop {
        if !parent_is_expected(parent_pid) {
            let observed_parent_pid = current_ppid();
            // Parent death often closes the companion's stdout/stderr pipes.
            // Termination is the contract; a broken diagnostic sink must not
            // panic this watcher before it performs that action.
            terminator();
            warn!(
                parent_pid,
                current_ppid = observed_parent_pid,
                "parent gone or reparented; companion terminator returned"
            );
            return;
        }
        thread::sleep(interval);
    });
    let handle = spawn(
        thread::Builder::new().name(format!("capsem-guard-watch-{parent_pid}")),
        task,
    )
    .map_err(|source| GuardError::WatcherSpawn { source })?;
    // The watcher intentionally owns its process-lifetime execution. Dropping
    // JoinHandle detaches it; it does not stop or cancel the created thread.
    drop(handle);
    Ok(())
}

/// System-wide singleton guard backed by `flock(2)` plus an in-process
/// registry. Holds the lock for the lifetime of the struct; dropping it (or
/// process exit) releases it.
pub struct Singleton {
    _lock: capsem_foundation::unix::lock::FileLock,
    path: PathBuf,
}

impl Singleton {
    /// Attempt to acquire a non-blocking exclusive flock on `lock_path`.
    ///
    /// * `Ok(Some(guard))` -- we won; we are the sole instance.
    /// * `Ok(None)` -- another process already holds the lock; caller exits 0.
    /// * `Err(_)` -- a real IO error (permissions, missing parent dir we could
    ///   not create, etc.). The caller should fail loudly.
    pub fn try_acquire(lock_path: &Path) -> Result<Option<Self>, GuardError> {
        if let Some(parent) = lock_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| GuardError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }
        let attempt =
            capsem_foundation::unix::lock::try_acquire(lock_path, capsem_foundation::unix::lock::LockMode::Exclusive)
                .map_err(|source| GuardError::Io {
                path: lock_path.to_path_buf(),
                source,
            })?;
        let capsem_foundation::unix::lock::LockAttempt::Acquired(lock) = attempt else {
            return Ok(None);
        };

        // Best-effort pid stamp for debuggability. The lock, not the file
        // contents, is the source of truth.
        use std::io::{Seek, SeekFrom, Write};
        let _ = (&*lock).seek(SeekFrom::Start(0));
        let payload = format!("{}\n", std::process::id());
        let _ = (&*lock).write_all(payload.as_bytes());
        let _ = lock.set_len(payload.len() as u64);

        Ok(Some(Self {
            _lock: lock,
            path: lock_path.to_path_buf(),
        }))
    }

    /// Path of the backing lockfile (informational, for logs).
    pub fn path(&self) -> &Path {
        &self.path
    }
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
pub fn install(parent_pid: Option<u32>, lock_path: &Path) -> Result<Option<InstalledGuards>, GuardError> {
    watch_parent_or_exit(parent_pid)?;
    match Singleton::try_acquire(lock_path)? {
        Some(s) => Ok(Some(InstalledGuards { _singleton: s })),
        None => Ok(None),
    }
}

/// Helper to parse `--parent-pid` style args. Accepts `None` and strings.
pub fn parse_parent_pid(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok()).filter(|&p| p > 0)
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
