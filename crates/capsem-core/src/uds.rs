//! Unix domain socket helpers.
//!
//! macOS caps `sockaddr_un.sun_path` at 104 bytes; Linux at 108. Temp dirs on
//! macOS (`/var/folders/lv/…`) easily blow past this, so per-VM socket paths
//! must fall back to a short hashed path under `/tmp/capsem/`.
//!
//! This module is the single source of truth for that rule. Clients MUST NOT
//! recompute the fallback path -- the fallback hash uses `DefaultHasher` which
//! is not stable across processes. Callers get the chosen path from the
//! service via the provision response.

use std::path::{Path, PathBuf};

/// Maximum length of a UDS path we'll accept before falling back to
/// `/tmp/capsem/<hash>.sock`. Chosen well under macOS's 104 and Linux's 108
/// so there's headroom for any suffix.
pub const SUN_PATH_MAX: usize = 90;

/// Compute the UDS socket path for a VM instance.
///
/// Returns `{run_dir}/instances/{id}.sock` when that fits under
/// `SUN_PATH_MAX`; otherwise a short hashed path under `/tmp/capsem/`.
///
/// The hashed path uses `DefaultHasher` which is randomised per-process --
/// so this function's output is ONLY valid in the process that originally
/// computed it. Other processes must receive the chosen path via IPC.
pub fn instance_socket_path(run_dir: &Path, id: &str) -> PathBuf {
    let preferred = run_dir.join("instances").join(format!("{id}.sock"));
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return preferred;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    run_dir.hash(&mut h);
    let dir = PathBuf::from("/tmp/capsem");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{:x}.sock", h.finish()))
}

/// Compute the terminal WebSocket UDS path for a VM instance.
///
/// Unlike [`instance_socket_path`], both the gateway and `capsem-process`
/// derive this *independently* and never exchange it -- so the short form has
/// to be deterministic across processes. That rules out `DefaultHasher`, whose
/// seed is randomised per process: a fallback computed with it would leave one
/// side binding a path the other never dials.
///
/// Neither side used this module at all. Each built
/// `{run_dir}/instances/{id}-ws.sock` by hand, which is 54 bytes of fixed
/// suffix for a 36-character session id, leaving roughly fifty for the run
/// directory. Past that every connection failed with `path must be shorter
/// than SUN_LEN`, logged at ERROR on each retry and surfaced to the user as a
/// session whose shell never appeared.
pub fn terminal_socket_path(run_dir: &Path, id: &str) -> PathBuf {
    let preferred = run_dir.join("instances").join(format!("{id}-ws.sock"));
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return preferred;
    }
    let mut digest = blake3::Hasher::new();
    digest.update(run_dir.as_os_str().as_encoded_bytes());
    digest.update(id.as_bytes());
    let short = &digest.finalize().to_hex()[..16];
    let dir = PathBuf::from("/tmp/capsem");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{short}-ws.sock"))
}

#[cfg(test)]
mod tests;
