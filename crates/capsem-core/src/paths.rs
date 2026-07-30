//! Centralized path resolution for the `~/.capsem/` hierarchy.
//!
//! Every crate that needs a `~/.capsem/...` path goes through here. This is
//! what lets `just test` run against an isolated `CAPSEM_HOME` without
//! killing or corrupting the user's locally installed capsem state.
//!
//! Override order (most specific wins):
//!   1. CAPSEM_RUN_DIR      -> capsem_run_dir()
//!      CAPSEM_ASSETS_DIR   -> capsem_assets_dir()
//!   2. CAPSEM_HOME         -> capsem_home() and all derived paths
//!   3. $HOME/.capsem       -> default
//!
//! Do NOT hand-roll `$HOME.join(".capsem")` anywhere in the tree. If you need
//! a new subdir, add a helper here.

use std::path::PathBuf;

/// Return the capsem base dir.
///
/// Priority: `CAPSEM_HOME` env var (if set + non-empty) -> `$HOME/.capsem`.
/// Panics only if neither `CAPSEM_HOME` nor `HOME` is set, which cannot happen
/// on a sane Unix environment; callers that need to handle that can use
/// [`capsem_home_opt`] instead.
pub fn capsem_home() -> PathBuf {
    capsem_home_opt().unwrap_or_else(|| PathBuf::from(".capsem"))
}

/// Fallible form of [`capsem_home`] for code that wants to surface a real error
/// when `HOME` is unset (rare: CI without `HOME`, bare container entrypoints).
pub fn capsem_home_opt() -> Option<PathBuf> {
    if let Some(h) = env_nonempty("CAPSEM_HOME") {
        return Some(PathBuf::from(h));
    }
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".capsem"))
}

/// Return `$CAPSEM_RUN_DIR` or `<capsem_home>/run`.
pub fn capsem_run_dir() -> PathBuf {
    if let Some(d) = env_nonempty("CAPSEM_RUN_DIR") {
        return PathBuf::from(d);
    }
    capsem_home().join("run")
}

/// Return `$CAPSEM_ASSETS_DIR` or `<capsem_home>/assets`.
pub fn capsem_assets_dir() -> PathBuf {
    if let Some(d) = env_nonempty("CAPSEM_ASSETS_DIR") {
        return PathBuf::from(d);
    }
    capsem_home().join("assets")
}

/// Return `<capsem_home>/sessions` (main.db + historical session rollups).
pub fn capsem_sessions_dir() -> PathBuf {
    capsem_home().join("sessions")
}

/// Return `<capsem_home>/bin` (installed binaries directory).
pub fn capsem_bin_dir() -> PathBuf {
    capsem_home().join("bin")
}

/// Return `<capsem_home>/logs` (app/frontend log files).
pub fn capsem_logs_dir() -> PathBuf {
    capsem_home().join("logs")
}

/// Return the service UDS socket path inside `capsem_run_dir()`.
pub fn service_socket_path() -> PathBuf {
    capsem_run_dir().join("service.sock")
}

/// Return the service pidfile path inside `capsem_run_dir()`.
pub fn service_pidfile_path() -> PathBuf {
    capsem_run_dir().join("service.pid")
}

fn env_nonempty(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Redirect every Capsem path variable at a temporary root, restoring on drop.
///
/// Test support, deliberately not `#[cfg(test)]`: fixtures in other crates
/// need it too, and the whole point is that there is one way to do this.
///
/// Set them **together or not at all**. `CAPSEM_RUN_DIR` and
/// `CAPSEM_ASSETS_DIR` each take precedence over the value derived from
/// `CAPSEM_HOME`, so a fixture that redirects only the home leaves production
/// code reading whichever run or assets directory the caller exported --
/// `just test` exports both. That fixture passes in a bare shell and fails
/// only inside the gate, which is exactly how the support-bundle tests came to
/// collect no host logs during a release run.
#[must_use = "the guard restores the previous environment when dropped"]
pub struct CapsemPathsGuard {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl CapsemPathsGuard {
    /// Point `CAPSEM_HOME`, `CAPSEM_RUN_DIR`, and `CAPSEM_ASSETS_DIR` at `root`.
    pub fn redirect(root: &std::path::Path) -> Self {
        let values = [
            ("CAPSEM_HOME", root.to_path_buf()),
            ("CAPSEM_RUN_DIR", root.join("run")),
            ("CAPSEM_ASSETS_DIR", root.join("assets")),
        ];
        let mut previous = Vec::with_capacity(values.len());
        for (key, value) in values {
            previous.push((key, std::env::var_os(key)));
            // SAFETY: tests that redirect paths serialize on a shared lock.
            unsafe {
                std::env::set_var(key, value);
            }
        }
        Self { previous }
    }
}

impl Drop for CapsemPathsGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            // SAFETY: as above.
            unsafe {
                match value {
                    Some(previous) => std::env::set_var(key, previous),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
