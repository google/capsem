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

#[cfg(test)]
mod tests;
