use super::*;

#[test]
fn short_path_uses_run_dir() {
    let run_dir = PathBuf::from("/tmp/r");
    let p = instance_socket_path(&run_dir, "vm-1").expect("socket path");
    assert_eq!(p, PathBuf::from("/tmp/r/instances/vm-1.sock"));
}

#[test]
fn long_path_falls_back_to_tmp_capsem() {
    let run_dir = PathBuf::from("/var/folders/lv/deeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeep/T/capsem-test-xxxx");
    let p = instance_socket_path(&run_dir, "tmp-long-name-that-blows-past-sun-len").expect("socket path");
    assert!(
        p.starts_with(format!("/tmp/capsem-{}/", current_uid())),
        "expected fallback under the private per-user dir, got {}",
        p.display()
    );
    assert!(p.as_os_str().len() < SUN_PATH_MAX);
}

#[test]
fn owner_sockets_prefer_the_readable_path() {
    let run_dir = PathBuf::from("/tmp/r");
    let path = private_handoff_socket_path(&run_dir, "vm-1").expect("socket path");

    assert_eq!(path, PathBuf::from("/tmp/r/instances/vm-1-handoff.sock"));
}

/// A 36-character session id under a deep run dir used to overflow
/// `sun_path` (12,024 failed terminal connections in one observed run); an
/// owner socket shortens instead.
#[test]
fn owner_sockets_fit_under_a_long_run_dir() {
    let run_dir = PathBuf::from("/Users/somebody/git/capsem/cache/target/tests/ironbank/co-work/home/.capsem/run");
    let path = private_handoff_socket_path(&run_dir, "322e7460-f1b2-4fdd-88f1-0c4b58c48e46").expect("socket path");

    assert!(
        path.as_os_str().len() < SUN_PATH_MAX,
        "{} is {} bytes",
        path.display(),
        path.as_os_str().len()
    );
}

#[test]
fn the_owner_socket_fallback_is_the_same_in_every_process() {
    let run_dir = PathBuf::from("/Users/somebody/git/capsem/cache/target/tests/ironbank/co-work/home/.capsem/run");
    let id = "322e7460-f1b2-4fdd-88f1-0c4b58c48e46";

    assert_eq!(
        private_handoff_socket_path(&run_dir, id).expect("socket path"),
        private_handoff_socket_path(&run_dir, id).expect("socket path")
    );
    assert_ne!(
        private_handoff_socket_path(&run_dir, id).expect("socket path"),
        private_handoff_socket_path(&run_dir, "0acea121-db0b-431e-91f3-c51291fa64fc").expect("socket path")
    );
}

/// Whatever it returns, the directory is there to bind in.
///
/// Only the fallback branch created its directory. The preferred branch
/// returned `{run_dir}/instances/…` and trusted somebody else to have made it,
/// which held for the service's own run tree and for nothing else.
#[test]
fn the_returned_path_has_a_directory_to_bind_in() {
    let temp = tempfile::tempdir().unwrap();

    for id in ["short-id", "322e7460-f1b2-4fdd-88f1-0c4b58c48e46"] {
        let path = private_handoff_socket_path(temp.path(), id).expect("socket path");
        assert!(path.parent().unwrap().is_dir(), "{path:?} has no directory to bind in");
        std::os::unix::net::UnixListener::bind(&path).unwrap_or_else(|e| panic!("cannot bind {path:?}: {e}"));
        let _ = std::fs::remove_file(&path);
    }
}

// The fallback directory is under the world-writable /tmp, so it is shared
// with every other user of the machine unless it is private and verified.
// `/tmp/capsem` was created 0755 by whichever user came first; another user
// could pre-create it, or a socket path inside it, and either delete a
// service's socket or bind their own there before the service did.

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_fallback_dir_is_created_private_to_this_user() {
    let temp = tempfile::tempdir().unwrap();
    let dir = private_fallback_dir_under(temp.path()).expect("fresh private dir");
    assert_eq!(dir, temp.path().join(format!("capsem-{}", current_uid())));
    assert!(dir.is_dir());
    assert_eq!(mode_of(&dir), 0o700);
    // Idempotent: a second call finds and accepts the same directory.
    assert_eq!(private_fallback_dir_under(temp.path()).unwrap(), dir);
}

#[test]
fn a_shared_or_planted_fallback_dir_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(format!("capsem-{}", current_uid()));

    // Group/other-writable: somebody else can unlink our sockets.
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let err = private_fallback_dir_under(temp.path()).expect_err("0777 dir must be refused");
    assert!(err.to_string().contains("mode"), "{err}");
    // Readable by others is still refused: socket names leak session ids.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    private_fallback_dir_under(temp.path()).expect_err("0755 dir must be refused");
    std::fs::remove_dir(&dir).unwrap();

    // A symlink planted at the path: the target is somebody else's choice.
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    let err = private_fallback_dir_under(temp.path()).expect_err("symlink must be refused");
    assert!(err.to_string().contains("symlink"), "{err}");
    std::fs::remove_file(&dir).unwrap();

    // A regular file squatting on the name.
    std::fs::write(&dir, b"").unwrap();
    private_fallback_dir_under(temp.path()).expect_err("file must be refused");
}
