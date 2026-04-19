//! Service startup coordination: make `capsem-service` self-idempotent.
//!
//! Four parallel `capsem-service --uds-path X` invocations must converge on
//! exactly one running service. This module provides the primitives:
//!
//!   - `probe_running_version` -- ask whoever is listening at a UDS path for
//!     its `/version`, so the caller can decide to reuse it or refuse.
//!   - `StartupLock` -- a filesystem lock next to the socket that serialises
//!     startup races. Released when dropped (including on crash).

use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nix::fcntl::{Flock, FlockArg};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// Probe the `/version` endpoint on a UDS. Returns:
///   - `Ok(Some(version))` if a service answered with a version string
///   - `Ok(None)` if nothing is listening (stale socket file or no file)
///   - `Err(e)` only for unexpected IO errors (not ECONNREFUSED / ENOENT)
///
/// Keeps the HTTP exchange deliberately small so we don't pull hyper here.
pub async fn probe_running_version(
    sock: &Path,
    timeout: Duration,
) -> io::Result<Option<String>> {
    let connect = async {
        match UnixStream::connect(sock).await {
            Ok(s) => Ok(Some(s)),
            Err(e) if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => Ok(None),
            Err(e) => Err(e),
        }
    };

    let mut stream = match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(Some(s))) => s,
        Ok(Ok(None)) => return Ok(None),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(None),
    };

    let request =
        b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

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
    json.get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// A filesystem-held advisory lock (flock) guarding service startup. Dropping
/// this handle releases the lock (fd close or explicit LOCK_UN) -- so a crash
/// during startup does NOT leave the lock held.
pub struct StartupLock {
    _flock: Flock<std::fs::File>,
}

impl StartupLock {
    /// Try to acquire the lock, waiting up to `timeout` for the holder to
    /// release it. Returns `Ok(Some(lock))` on success or `Ok(None)` if the
    /// holder never released within the deadline.
    pub fn acquire(lock_path: &Path, timeout: Duration) -> Result<Option<Self>> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent of lock file {}", lock_path.display())
            })?;
        }

        let deadline = Instant::now() + timeout;
        loop {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(lock_path)
                .with_context(|| {
                    format!("failed to open lock file {}", lock_path.display())
                })?;

            match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
                Ok(flock) => return Ok(Some(Self { _flock: flock })),
                Err((_file, nix::errno::Errno::EWOULDBLOCK)) => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err((_file, e)) => {
                    return Err(anyhow::anyhow!(
                        "flock failed on {}: {}",
                        lock_path.display(),
                        e
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_body_extracts_version() {
        let resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"version\":\"1.2.3\"}";
        assert_eq!(parse_version_body(resp).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn parse_version_body_missing_field_returns_none() {
        let resp = b"HTTP/1.1 200 OK\r\n\r\n{\"other\":\"x\"}";
        assert_eq!(parse_version_body(resp), None);
    }

    #[test]
    fn parse_version_body_no_body_returns_none() {
        let resp = b"HTTP/1.1 500 OK\r\n\r\n";
        assert_eq!(parse_version_body(resp), None);
    }

    #[test]
    fn startup_lock_is_mutually_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("service.lock");

        let a = StartupLock::acquire(&lock_path, Duration::from_millis(50))
            .unwrap()
            .expect("first acquisition");
        let b = StartupLock::acquire(&lock_path, Duration::from_millis(50)).unwrap();
        assert!(b.is_none(), "second acquisition must fail while first is held");

        drop(a);

        let c = StartupLock::acquire(&lock_path, Duration::from_millis(500))
            .unwrap()
            .expect("reacquire after drop");
        drop(c);
    }
}
