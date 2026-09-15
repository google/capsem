//! Unix domain socket helpers.
//!
//! macOS caps `sockaddr_un.sun_path` at 104 bytes; Linux at 108. Temp dirs on
//! macOS (`/var/folders/lv/…`) easily blow past this, so per-VM socket paths
//! must fall back to a short hashed path under a private per-user directory
//! in `/tmp`.
//!
//! This module is the single source of truth for that rule. Clients MUST NOT
//! recompute the fallback path -- the fallback hash uses `DefaultHasher` which
//! is not stable across processes. Callers get the chosen path from the
//! service via the provision response.

use std::io;
use std::path::{Path, PathBuf};

pub use crate::unix::process::current_uid;

/// Maximum length of a UDS path we'll accept before falling back to
/// `/tmp/capsem-<uid>/<hash>.sock`. Chosen well under macOS's 104 and Linux's 108
/// so there's headroom for any suffix.
pub const SUN_PATH_MAX: usize = 90;

/// Compute the UDS socket path for a VM instance.
///
/// Returns `{run_dir}/instances/{id}.sock` when that fits under
/// `SUN_PATH_MAX`; otherwise a short hashed path under the private per-user
/// fallback directory, which is an error when that directory cannot be
/// created or is not ours alone.
///
/// The hashed path uses `DefaultHasher` which is randomised per-process --
/// so this function's output is ONLY valid in the process that originally
/// computed it. Other processes must receive the chosen path via IPC.
pub fn instance_socket_path(run_dir: &Path, id: &str) -> io::Result<PathBuf> {
    let preferred = run_dir.join("instances").join(format!("{id}.sock"));
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return Ok(preferred);
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    run_dir.hash(&mut h);
    Ok(private_fallback_dir()?.join(format!("{:x}.sock", h.finish())))
}

/// `/tmp/capsem-<uid>`, created 0700 and verified to be exactly that.
///
/// The fallback used to live in a shared `/tmp/capsem`, created 0755 by
/// whichever user came first. Under a world-writable `/tmp` that directory
/// belonged to somebody else for everyone after them: they could unlink a
/// service's socket or bind their own at a path the service was about to
/// use, and every client holding that path from the provision response would
/// dial it. A directory that is a symlink, not ours, or readable by anyone
/// else is refused rather than used.
pub fn private_fallback_dir() -> io::Result<PathBuf> {
    private_fallback_dir_under(Path::new("/tmp"))
}

fn private_fallback_dir_under(base: &Path) -> io::Result<PathBuf> {
    let uid = current_uid();
    let dir = base.join(format!("capsem-{uid}"));
    crate::unix::fs::ensure_private_dir(&dir)?;
    Ok(dir)
}

/// Where the service asks a VM owner for the guest's end of a cable it is
/// plugging, one path per VM.
///
/// Short forms come from blake3 over the run dir, id and role, so the
/// fallback is the same in every process that derives it; `DefaultHasher` is
/// seeded per process.
pub fn private_handoff_socket_path(run_dir: &Path, id: &str) -> io::Result<PathBuf> {
    owner_socket_path(run_dir, id, "handoff")
}

fn owner_socket_path(run_dir: &Path, id: &str, role: &str) -> io::Result<PathBuf> {
    let preferred = run_dir.join("instances").join(format!("{id}-{role}.sock"));
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return Ok(ensured(preferred));
    }
    let mut digest = blake3::Hasher::new();
    digest.update(run_dir.as_os_str().as_encoded_bytes());
    digest.update(id.as_bytes());
    digest.update(role.as_bytes());
    let short = &digest.finalize().to_hex()[..16];
    Ok(private_fallback_dir()?.join(format!("{short}-{role}.sock")))
}

/// A path with a directory to bind in.
///
/// Only the fallback branch created its directory; the preferred branch
/// returned `{run_dir}/instances/…` and trusted somebody else to have made it.
/// That held for the service's own run tree and nowhere else, and the failure
/// it produced was `bind: No such file or directory` from inside an async
/// loop -- a VM that simply never became exec-ready.
///
/// A creation failure is left to `bind`, which reports the same condition
/// with the path in it: a caller that cannot bind here is going to say so a
/// line later, and better.
fn ensured(path: PathBuf) -> PathBuf {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    path
}

#[cfg(test)]
mod tests;
