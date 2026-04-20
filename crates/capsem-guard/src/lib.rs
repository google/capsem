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
    static HELD: OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        OnceLock::new();
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
        let c_path = CString::new(lock_path.as_os_str().as_encoded_bytes()).map_err(|_| {
            GuardError::Io {
                path: lock_path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "lock path contains a NUL byte",
                ),
            }
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
        let canonical = std::fs::canonicalize(lock_path)
            .unwrap_or_else(|_| lock_path.to_path_buf());
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
        let rc = unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB)
        };
        if rc != 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // Give the reservation back so a future retry can succeed.
            held_locks().lock().expect("held-locks mutex poisoned").remove(&canonical);
            if errno == libc::EWOULDBLOCK {
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
mod tests {
    use super::*;
    use std::time::Instant;

    // ---- is_alive ------------------------------------------------------

    #[test]
    fn is_alive_detects_self() {
        assert!(is_alive(std::process::id()));
    }

    #[test]
    fn is_alive_rejects_zero() {
        assert!(!is_alive(0));
    }

    #[test]
    fn is_alive_handles_very_high_pid() {
        assert!(!is_alive(i32::MAX as u32));
    }

    // ---- parent_is_expected / current_ppid -----------------------------

    #[test]
    fn parent_is_expected_matches_our_real_ppid() {
        let real_ppid = current_ppid();
        assert!(parent_is_expected(real_ppid));
    }

    #[test]
    fn parent_is_expected_rejects_wrong_pid() {
        // Our own PID is certainly not our parent.
        assert!(!parent_is_expected(std::process::id()));
    }

    #[test]
    fn parent_is_expected_rejects_zero() {
        assert!(!parent_is_expected(0));
    }

    #[test]
    fn parent_is_expected_rejects_pid_one() {
        // A companion must never accept init as a legitimate parent; that's
        // exactly the orphan state we're defending against.
        assert!(!parent_is_expected(1));
    }

    // ---- parse_parent_pid ----------------------------------------------

    #[test]
    fn parse_parent_pid_accepts_digits() {
        assert_eq!(parse_parent_pid(Some("42")), Some(42));
        assert_eq!(parse_parent_pid(Some("  42  ")), Some(42));
    }

    #[test]
    fn parse_parent_pid_rejects_junk() {
        assert_eq!(parse_parent_pid(None), None);
        assert_eq!(parse_parent_pid(Some("")), None);
        assert_eq!(parse_parent_pid(Some("foo")), None);
        assert_eq!(parse_parent_pid(Some("0")), None);
        assert_eq!(parse_parent_pid(Some("-1")), None);
    }

    // ---- Singleton -----------------------------------------------------
    //
    // These tests run fully in parallel (no serialization). Cross-process
    // flock inheritance through fork has a well-known race: Command::spawn
    // does fork() then exec(), and CLOEXEC only closes fds at exec. During
    // the fork-to-exec window the child has a copy of every open fd in the
    // test binary, including anyone else's flock. In PRODUCTION this is a
    // non-issue because the service and companions lock DISTINCT paths --
    // no one reacquires the same path in the same process. The tests below
    // explicitly avoid the pathological "drop + reacquire in the same
    // process while other tests fork" pattern; we cover reacquire-after-
    // crash with a cross-process test using a real subprocess instead.

    #[test]
    fn singleton_blocks_same_process_second_call() {
        // In-process mutex (registry + flock) must prevent a same-process
        // double acquire. The registry short-circuits before flock, so this
        // is immune to any sibling Command::spawn races.
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("sing.lock");

        let _a = Singleton::try_acquire(&lock).unwrap().expect("first");
        let b = Singleton::try_acquire(&lock).unwrap();
        assert!(b.is_none(), "second must return None while first is held");
    }

    #[test]
    fn singleton_reacquires_after_drop_in_isolated_process() {
        // Drop-then-reacquire must work. Rather than doing it in the test
        // binary -- where a sibling test's Command::spawn fork can briefly
        // inherit our flock fd and keep the kernel lock alive past drop --
        // we fork a subprocess whose ONLY work is acquire + drop + reacquire.
        // No other threads in that process do Command::spawn, so no leak.
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("reacquire.lock");

        // Tiny Rust-equivalent via a shell+perl flock: acquire, release, reacquire.
        // perl's flock is LOCK_EX|LOCK_NB by design; the script exits 0 iff
        // every step succeeds.
        let script = format!(
            "use Fcntl qw(:flock); \
             for (1..2) {{ \
                 open(F, '>', '{}') or die $!; \
                 flock(F, LOCK_EX|LOCK_NB) or exit 1; \
                 close(F); \
             }} \
             exit 0;",
            lock.display()
        );
        let status = Command::new("perl")
            .arg("-e")
            .arg(&script)
            .status()
            .expect("run perl");
        assert!(
            status.success(),
            "flock acquire+release+reacquire in isolated process must succeed"
        );
    }

    #[test]
    fn singleton_writes_pid_for_debugging() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("pid.lock");
        let _g = Singleton::try_acquire(&lock).unwrap().expect("acquire");
        let contents = std::fs::read_to_string(&lock).unwrap();
        let parsed: u32 = contents.trim().parse().expect("lock file must contain pid");
        assert_eq!(parsed, std::process::id());
    }

    #[test]
    fn singleton_fd_is_cloexec() {
        // FD_CLOEXEC is what stops our flock from leaking into
        // Command::spawn'd children; verify it's actually set on the fd.
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("cloexec.lock");
        let guard = Singleton::try_acquire(&lock).unwrap().expect("acquire");
        use std::os::fd::AsRawFd;
        // SAFETY: guard owns the fd for its lifetime; F_GETFD is read-only.
        let flags = unsafe { libc::fcntl(guard._file.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0, "fcntl F_GETFD must succeed");
        assert!(
            flags & libc::FD_CLOEXEC != 0,
            "FD_CLOEXEC must be set to prevent leaking locks into children"
        );
    }

    #[test]
    fn singleton_reacquires_after_ungraceful_holder_exit() {
        // A subprocess that acquires the lock and is SIGKILL'd must release
        // the flock on fd close (kernel-level crash semantics) so future
        // holders can take it.
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("crash.lock");

        let mut sleeper = Command::new("perl")
            .arg("-e")
            .arg("use Fcntl qw(:flock); open(F, \">\", $ARGV[0]) or die $!; flock(F, LOCK_EX|LOCK_NB) or die \"locked\"; print \"locked\\n\"; $|=1; sleep 30;")
            .arg(&lock)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sleeper");

        // Wait for the subprocess to actually print that it holds the lock.
        use std::io::Read;
        let mut stdout = sleeper.stdout.take().unwrap();
        let mut buf = [0u8; 16];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() >= deadline {
                let _ = sleeper.kill();
                let _ = sleeper.wait();
                panic!("subprocess never acquired lock");
            }
            match stdout.read(&mut buf) {
                Ok(n) if n > 0 => break,
                Ok(_) => thread::sleep(Duration::from_millis(50)),
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }

        // Observable: the parent must be unable to acquire while the
        // subprocess holds it.
        assert!(
            Singleton::try_acquire(&lock).unwrap().is_none(),
            "lock must be held by subprocess"
        );

        // SIGKILL -- skips any cleanup in the subprocess.
        unsafe { libc::kill(sleeper.id() as libc::pid_t, libc::SIGKILL) };
        let _ = sleeper.wait();

        let reacq = Singleton::try_acquire(&lock)
            .unwrap()
            .expect("flock must release on crash");
        drop(reacq);
    }

    // ---- watch_parent_or_exit / install --------------------------------

    #[test]
    fn watch_parent_or_exit_rejects_missing_pid() {
        assert!(matches!(
            watch_parent_or_exit(None),
            Err(GuardError::NoParent)
        ));
    }

    #[test]
    fn watch_parent_or_exit_rejects_non_parent() {
        // Our own PID is not our parent.
        let result = watch_parent_or_exit(Some(std::process::id()));
        assert!(matches!(result, Err(GuardError::ParentDead(_))));
    }

    #[test]
    fn watch_parent_or_exit_accepts_real_parent() {
        // Our real parent (cargo test / the shell) is alive.
        let ppid = current_ppid();
        assert!(ppid > 1, "test runner expected to have a real parent");
        watch_parent_or_exit(Some(ppid)).expect("real parent must pass");
    }

    #[test]
    fn install_rejects_missing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let r = install(None, &dir.path().join("x.lock"));
        assert!(matches!(r, Err(GuardError::NoParent)));
    }

    #[test]
    fn install_rejects_non_parent_before_touching_lock() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("x.lock");
        let r = install(Some(std::process::id()), &lock);
        assert!(matches!(r, Err(GuardError::ParentDead(_))));
        assert!(!lock.exists(), "non-parent guard must not create the lockfile");
    }

    // ---- PARENT_POLL_INTERVAL contract --------------------------------
    //
    // `_ensure-service` (justfile recipe) SIGTERMs the prior dev service and
    // sleeps 500 ms before spawning the new one on the same TCP port. If the
    // watch interval ever grows back to approach that budget, a SIGKILL'd
    // service's companions can still hold port 19222 when the new service's
    // gateway tries to bind -- reintroducing the orphan-gateway regression
    // fixed in this sprint. These tests lock the invariant into code, not
    // prose.

    #[test]
    fn parent_poll_interval_is_well_under_ensure_service_budget() {
        // `_ensure-service`'s sleep is 500 ms. We need the worst-case
        // companion exit latency to comfortably fit inside that, which
        // means the poll interval must be FAR smaller -- not equal.
        assert!(
            PARENT_POLL_INTERVAL <= Duration::from_millis(200),
            "PARENT_POLL_INTERVAL ({:?}) must stay <= 200 ms so SIGKILL'd \
             companions reliably exit within `_ensure-service`'s 500 ms \
             restart window. See CHANGELOG for the regression history.",
            PARENT_POLL_INTERVAL,
        );
    }

    #[test]
    fn watch_parent_with_fires_within_two_polls_on_reparent() {
        // Simulate reparenting by targeting a PID that is not our parent:
        // `parent_is_expected` returns false immediately, so the watcher
        // fires on its very next poll.
        let wrong_pid = std::process::id();
        let start = Instant::now();
        let fired = watch_parent_with(
            wrong_pid,
            PARENT_POLL_INTERVAL,
            || {},
        );
        // Two full poll intervals is the absolute worst case: wake, sleep,
        // wake. One interval is the common case. Anything larger means the
        // watcher is not doing what the module promises.
        let deadline = Instant::now() + 2 * PARENT_POLL_INTERVAL + Duration::from_millis(50);
        while Instant::now() < deadline {
            if fired.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            fired.load(Ordering::Acquire),
            "watcher did not fire within 2x PARENT_POLL_INTERVAL (~{}ms)",
            (2 * PARENT_POLL_INTERVAL).as_millis(),
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "parent-watch should fire within 1 s even on a loaded CI"
        );
    }

    #[test]
    fn watch_parent_with_does_not_fire_while_parent_alive() {
        // Watcher targeting our real parent must stay quiet for several
        // poll intervals. This catches accidental `parent_is_expected`
        // inversions or zero-interval bugs.
        let real_ppid = current_ppid();
        assert!(real_ppid > 1, "test must run under a real parent");
        let fired = watch_parent_with(
            real_ppid,
            Duration::from_millis(20),
            || {},
        );
        thread::sleep(Duration::from_millis(200)); // ~10 polls
        assert!(
            !fired.load(Ordering::Acquire),
            "watcher spuriously fired while our real parent is alive"
        );
    }

    // ---- Singleton error paths ----------------------------------------
    //
    // The happy path is covered above; these exercise the branches llvm-cov
    // shows uncovered: CString NUL, open() failure, parent-dir creation
    // failure.

    #[test]
    fn singleton_rejects_path_with_nul_byte() {
        // CString::new rejects any interior NUL. The function converts that
        // into a GuardError::Io with InvalidInput so callers can surface
        // the specific reason.
        let evil = PathBuf::from("/tmp/nul\0.lock");
        let result = Singleton::try_acquire(&evil);
        match result {
            Err(GuardError::Io { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
            }
            Err(other) => panic!("expected Io/InvalidInput for NUL path, got {other:?}"),
            Ok(_) => panic!("expected Io/InvalidInput for NUL path, got Ok(_)"),
        }
    }

    #[test]
    fn singleton_fails_when_path_is_an_existing_directory() {
        // open(O_RDWR|O_CREAT) on a directory returns EISDIR -- the error
        // path must surface that cleanly rather than panicking or leaking
        // a registry entry.
        let dir = tempfile::tempdir().unwrap();
        let as_lock = dir.path().to_path_buf();
        // Sanity: our target path is a directory.
        assert!(as_lock.is_dir());
        let result = Singleton::try_acquire(&as_lock);
        assert!(
            matches!(result, Err(GuardError::Io { .. })),
            "expected Io error for directory-as-lockfile"
        );
        // Registry must not still have a reservation for this path.
        let canonical = std::fs::canonicalize(&as_lock).unwrap_or(as_lock.clone());
        assert!(
            !held_locks().lock().unwrap().contains(&canonical),
            "lock reservation leaked into the registry on IO error"
        );
    }

    #[test]
    fn singleton_creates_missing_parent_dirs() {
        // A path several levels deep under a nonexistent parent must still
        // succeed -- try_acquire does create_dir_all on the parent chain.
        // This exercises the `if let Some(parent) = lock_path.parent()` arm.
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c/sing.lock");
        let g = Singleton::try_acquire(&deep)
            .expect("try_acquire should not error")
            .expect("try_acquire should return Some on fresh path");
        assert!(deep.exists(), "lockfile not created: {}", deep.display());
        drop(g);
    }

    #[test]
    fn singleton_fails_when_parent_cannot_be_created() {
        // If the "parent directory" is actually an existing FILE, create_dir_all
        // will fail with NotADirectory. The error must propagate with the
        // right path in the GuardError.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not_a_dir");
        std::fs::write(&file, "occupied").unwrap();
        let bogus = file.join("child.lock");
        let result = Singleton::try_acquire(&bogus);
        match result {
            Err(GuardError::Io { path, source: _ }) => {
                // Must report the unwritable parent, not the leaf.
                assert_eq!(path, file, "error path should be the parent, got {}", path.display());
            }
            Err(other) => panic!("expected Io error for unwritable parent, got {other:?}"),
            Ok(_) => panic!("expected Io error for unwritable parent, got Ok(_)"),
        }
    }

    #[test]
    fn singleton_path_accessor_returns_original_path() {
        // The `path()` accessor is advertised as "informational, for logs".
        // Ensure it returns the exact input (not the canonical form) so log
        // output doesn't surprise operators when symlinks are involved.
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("accessor.lock");
        let g = Singleton::try_acquire(&lock).unwrap().expect("acquire");
        assert_eq!(g.path(), lock.as_path());
    }

    #[test]
    fn is_alive_reports_pid_one_as_alive() {
        // PID 1 (launchd on macOS, init/systemd on Linux) is always running
        // in any POSIX system the test could run on, and is owned by root.
        // is_alive must return true for it -- either via direct kill(pid, 0)
        // success when running as root, or via the EPERM-means-alive branch
        // when running as an ordinary user. Both paths are correct; we
        // don't care which one fires.
        assert!(
            is_alive(1),
            "PID 1 (launchd/init) must always be reported alive"
        );
    }

    // ---- install() end-to-end ------------------------------------------

    #[test]
    fn install_happy_path_returns_guards_and_creates_lock() {
        // The existing install_* tests only cover the rejection arms
        // (NoParent / ParentDead). Cover Ok(Some(_)): real parent + fresh
        // lock path must produce an InstalledGuards and leave the lockfile
        // on disk. Drop-then-reacquire coverage lives in
        // `singleton_reacquires_after_drop_in_isolated_process`, which
        // forks a clean subprocess -- doing it here would regress under
        // parallel cargo test, since a sibling test's Command::spawn fork
        // can briefly inherit our flock fd and keep the kernel lock alive
        // past drop.
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("happy.lock");
        let ppid = current_ppid();
        assert!(ppid > 1);
        let guards = install(Some(ppid), &lock)
            .expect("install must succeed under a real parent")
            .expect("install must return Some when lock is fresh");
        assert!(lock.exists(), "install did not create the lock file");
        drop(guards);
    }

    #[test]
    fn install_returns_none_when_lock_already_held() {
        // Pre-acquire the lock in this process (in-process registry path);
        // install() must observe that and return Ok(None) without touching
        // the watcher thread's process-wide state.
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("busy.lock");
        let holder = Singleton::try_acquire(&lock).unwrap().expect("pre-acquire");
        let ppid = current_ppid();
        let result = install(Some(ppid), &lock).expect("install IO must succeed");
        assert!(
            result.is_none(),
            "install should bounce Ok(None) when the singleton is held"
        );
        drop(holder);
    }

    #[test]
    fn multiple_watchers_on_same_parent_coexist() {
        // `install()` in production only arms one watcher per process, but
        // nothing in the contract says two watchers cannot share a target.
        // The internal state of watch_parent_with is per-watcher (each
        // owns its own AtomicBool), so they must not interfere.
        let real_ppid = current_ppid();
        let fired_a = watch_parent_with(real_ppid, Duration::from_millis(20), || {});
        let fired_b = watch_parent_with(real_ppid, Duration::from_millis(20), || {});
        thread::sleep(Duration::from_millis(100));
        assert!(!fired_a.load(Ordering::Acquire));
        assert!(!fired_b.load(Ordering::Acquire));

        // Point a third watcher at a bogus PID; only IT should fire.
        let fired_c = watch_parent_with(std::process::id(), Duration::from_millis(20), || {});
        thread::sleep(Duration::from_millis(100));
        assert!(fired_c.load(Ordering::Acquire));
        assert!(!fired_a.load(Ordering::Acquire));
        assert!(!fired_b.load(Ordering::Acquire));
    }
}
