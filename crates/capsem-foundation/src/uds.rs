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

/// The calling user's numeric id.
pub fn current_uid() -> u32 {
    // SAFETY: getuid(2) takes no arguments and cannot fail.
    unsafe { libc::getuid() }
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
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let uid = current_uid();
    let dir = base.join(format!("capsem-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("create private socket dir {}: {error}", dir.display()),
            ))
        }
    }
    let refuse = |what: String| io::Error::other(format!("refusing socket fallback dir {}: {what}", dir.display()));
    let metadata = std::fs::symlink_metadata(&dir)?;
    if metadata.file_type().is_symlink() {
        return Err(refuse("it is a symlink".to_string()));
    }
    if !metadata.is_dir() {
        return Err(refuse("it is not a directory".to_string()));
    }
    if metadata.uid() != uid {
        return Err(refuse(format!("it is owned by uid {}, not {uid}", metadata.uid())));
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(refuse(format!("its mode is {mode:o}, not 700")));
    }
    Ok(dir)
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
pub fn terminal_socket_path(run_dir: &Path, id: &str) -> io::Result<PathBuf> {
    let preferred = run_dir.join("instances").join(format!("{id}-ws.sock"));
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return Ok(ensured(preferred));
    }
    let mut digest = blake3::Hasher::new();
    digest.update(run_dir.as_os_str().as_encoded_bytes());
    digest.update(id.as_bytes());
    let short = &digest.finalize().to_hex()[..16];
    Ok(private_fallback_dir()?.join(format!("{short}-ws.sock")))
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
