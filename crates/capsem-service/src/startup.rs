//! Service startup coordination: make `capsem-service` self-idempotent.
//!
//! Four parallel `capsem-service --uds-path X` invocations must converge on
//! exactly one running service. This module provides the primitives:
//!
//!   - `probe_running_version` -- ask whoever is listening at a UDS path for
//!     its `/version`, so the caller can decide to reuse it or refuse.
//!   - `StartupLock` -- a filesystem lock next to the socket that serialises
//!     startup races. Released when dropped (including on crash).

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// Probe the `/version` endpoint on a UDS. Returns:
///   - `Ok(Some(version))` if a service answered with a version string
///   - `Ok(None)` if nothing is listening (stale socket file or no file)
///   - `Err(e)` only for unexpected IO errors (not ECONNREFUSED / ENOENT)
///
/// Keeps the HTTP exchange deliberately small so we don't pull hyper here.
pub async fn probe_running_version(sock: &Path, timeout: Duration) -> io::Result<Option<String>> {
    let connect = async {
        match UnixStream::connect(sock).await {
            Ok(s) => Ok(Some(s)),
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => Ok(None),
            Err(e) => Err(e),
        }
    };

    let mut stream = match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(Some(s))) => s,
        Ok(Ok(None)) => return Ok(None),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(None),
    };

    let request = b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    let exchange = async {
        stream.write_all(request).await?;
        let mut buf = Vec::with_capacity(256);
        // /version responds with a tiny JSON body, so read the whole thing.
        stream.read_to_end(&mut buf).await?;
        Ok::<_, io::Error>(buf)
    };

    let buf = match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(buf)) => buf,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(None),
    };

    Ok(parse_version_body(&buf))
}

/// Split HTTP response headers from body and extract the `"version"` field.
fn parse_version_body(response: &[u8]) -> Option<String> {
    let sep = b"\r\n\r\n";
    let idx = response.windows(sep.len()).position(|w| w == sep)?;
    let body = &response[idx + sep.len()..];
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    json.get("version").and_then(|v| v.as_str()).map(str::to_string)
}

/// Host-wide flock guarding Apple VZ save_state / restore_state so the
/// serialization reaches across sibling `capsem-service` processes (e.g.
/// pytest-xdist `-n 4` workers).
///
/// Cold starts and teardown take a shared lock; save/restore take an exclusive
/// lock. Apple's VZ framework does not tolerate crossing checkpoint lifecycle
/// edges, but it does tolerate sibling cold starts. See
/// web/docs/src/content/docs/gotchas/concurrent-suspend-resume.mdx.
///
/// Lock file lives at `/tmp/capsem-<uid>/vz-save-restore.lock` -- outside
/// any `CAPSEM_HOME`/`CAPSEM_RUN_DIR` override so every sibling service
/// on the same host agrees on one path, and inside the verified private
/// per-user directory so another user cannot plant a symlink or squat the
/// name. `/tmp` is always writable and survives a suspend; the flock
/// releases automatically on crash (fd close).
pub struct VzHostLock {
    _lock: capsem_foundation::unix::lock::FileLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VzHostLockMode {
    Shared,
    Exclusive,
}

impl VzHostLock {
    fn lock_path() -> Result<std::path::PathBuf> {
        Ok(capsem_foundation::uds::private_fallback_dir()?.join("vz-save-restore.lock"))
    }

    /// Acquire the host-wide lock, waiting up to `timeout` for a compatible
    /// sibling lifecycle operation to release it. Returns `Ok(Some(lock))`
    /// on success, `Ok(None)` on timeout (caller decides whether to fail).
    pub fn acquire(mode: VzHostLockMode, timeout: Duration) -> Result<Option<Self>> {
        let path = Self::lock_path()?;
        Self::acquire_path(&path, mode, timeout)
    }

    fn acquire_path(path: &Path, mode: VzHostLockMode, timeout: Duration) -> Result<Option<Self>> {
        let deadline = Instant::now() + timeout;
        let mode = match mode {
            VzHostLockMode::Shared => capsem_foundation::unix::lock::LockMode::Shared,
            VzHostLockMode::Exclusive => capsem_foundation::unix::lock::LockMode::Exclusive,
        };
        loop {
            match capsem_foundation::unix::lock::try_acquire(path, mode)
                .with_context(|| format!("failed to acquire vz host lock {}", path.display()))?
            {
                capsem_foundation::unix::lock::LockAttempt::Acquired(lock) => return Ok(Some(Self { _lock: lock })),
                capsem_foundation::unix::lock::LockAttempt::Contended => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    #[cfg(test)]
    fn acquire_test_path(path: &Path, mode: VzHostLockMode, timeout: Duration) -> Result<Option<Self>> {
        Self::acquire_path(path, mode, timeout)
    }
}

/// A filesystem-held advisory lock (flock) guarding service startup. Dropping
/// this handle releases the lock (fd close or explicit LOCK_UN) -- so a crash
/// during startup does NOT leave the lock held.
pub struct StartupLock {
    _lock: capsem_foundation::unix::lock::FileLock,
}

impl StartupLock {
    /// Try to acquire the lock, waiting up to `timeout` for the holder to
    /// release it. Returns `Ok(Some(lock))` on success or `Ok(None)` if the
    /// holder never released within the deadline.
    pub fn acquire(lock_path: &Path, timeout: Duration) -> Result<Option<Self>> {
        let deadline = Instant::now() + timeout;
        loop {
            match capsem_foundation::unix::lock::try_acquire(
                lock_path,
                capsem_foundation::unix::lock::LockMode::Exclusive,
            )
            .with_context(|| format!("failed to acquire startup lock {}", lock_path.display()))?
            {
                capsem_foundation::unix::lock::LockAttempt::Acquired(lock) => return Ok(Some(Self { _lock: lock })),
                capsem_foundation::unix::lock::LockAttempt::Contended => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
