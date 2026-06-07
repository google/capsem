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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_path_uses_run_dir() {
        let run_dir = PathBuf::from("/tmp/r");
        let p = instance_socket_path(&run_dir, "vm-1");
        assert_eq!(p, PathBuf::from("/tmp/r/instances/vm-1.sock"));
    }

    #[test]
    fn long_path_falls_back_to_tmp_capsem() {
        let run_dir = PathBuf::from(
            "/var/folders/lv/deeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeep/T/capsem-test-xxxx",
        );
        let p = instance_socket_path(&run_dir, "tmp-long-name-that-blows-past-sun-len");
        assert!(
            p.starts_with("/tmp/capsem/"),
            "expected fallback under /tmp/capsem/, got {}",
            p.display()
        );
        assert!(p.as_os_str().len() < SUN_PATH_MAX);
    }
}
