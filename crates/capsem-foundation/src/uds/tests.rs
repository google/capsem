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

/// The terminal socket is the one both the gateway and the VM process derive
/// *independently*, so its short form has to be deterministic.
///
/// It did not go through this module at all: the gateway built
/// `run_dir/instances/{uuid}-ws.sock` and the process built the same string by
/// hand. With a 36-character session id that is 54 bytes of fixed suffix,
/// leaving about fifty for the run directory against macOS's 104. Past that,
/// every connection failed with `path must be shorter than SUN_LEN` -- 12,024
/// times in one observed run -- while the TUI showed a session whose shell
/// simply never appeared.
#[test]
fn terminal_socket_prefers_the_readable_path() {
    let run_dir = PathBuf::from("/tmp/r");
    let path = terminal_socket_path(&run_dir, "vm-1").expect("socket path");

    assert_eq!(path, PathBuf::from("/tmp/r/instances/vm-1-ws.sock"));
}

#[test]
fn terminal_socket_fits_under_a_long_run_dir() {
    let run_dir = PathBuf::from("/Users/somebody/git/capsem/cache/target/tests/ironbank/co-work/home/.capsem/run");
    let path = terminal_socket_path(&run_dir, "322e7460-f1b2-4fdd-88f1-0c4b58c48e46").expect("socket path");

    assert!(
        path.as_os_str().len() < SUN_PATH_MAX,
        "{} is {} bytes",
        path.display(),
        path.as_os_str().len()
    );
}

#[test]
fn the_terminal_fallback_is_the_same_in_every_process() {
    // The gateway and capsem-process each compute this from the run dir and
    // the id, and never exchange it. A per-process hash would leave them
    // binding and dialling different paths -- which fails exactly like the
    // overflow it was meant to fix.
    let run_dir = PathBuf::from("/Users/somebody/git/capsem/cache/target/tests/ironbank/co-work/home/.capsem/run");
    let id = "322e7460-f1b2-4fdd-88f1-0c4b58c48e46";

    assert_eq!(
        terminal_socket_path(&run_dir, id).expect("socket path"),
        terminal_socket_path(&run_dir, id).expect("socket path")
    );
    assert_ne!(
        terminal_socket_path(&run_dir, id).expect("socket path"),
        terminal_socket_path(&run_dir, "0acea121-db0b-431e-91f3-c51291fa64fc").expect("socket path")
    );
}

/// The two sides have to agree about *which run directory*, not only about
/// how a run directory becomes a socket path.
///
/// `capsem-process` derived its run directory by walking two levels up from
/// its own IPC socket -- correct while that socket is `{run}/instances/x.sock`,
/// and wrong the moment the IPC path is itself shortened to
/// `/tmp/capsem/<hash>.sock`, which is exactly the case the shortening exists
/// for. Walking up gave `/tmp`, so the process bound `/tmp/instances/…` while
/// the gateway dialled `{run}/instances/…`, and `/tmp/instances` does not
/// exist: `async loop failed: No such file or directory (os error 2)`, and a
/// VM that never became exec-ready.
#[test]
fn walking_up_from_a_shortened_ipc_path_finds_the_wrong_run_dir() {
    let run_dir = PathBuf::from("/private/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/capsem-test-xw4j_rzq/run");
    let id = "bb61246d-1dec-489f-8a2f-48c263fe4d5c";

    let ipc = instance_socket_path(&run_dir, id).expect("socket path");
    assert!(
        ipc.starts_with(format!("/tmp/capsem-{}", current_uid())),
        "this run dir must shorten: {ipc:?}"
    );

    let walked = ipc.parent().and_then(|p| p.parent()).unwrap();
    assert_ne!(
        walked, run_dir,
        "walking up a shortened path cannot recover the run directory"
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
        let path = terminal_socket_path(temp.path(), id).expect("socket path");
        assert!(path.parent().unwrap().is_dir(), "{path:?} has no directory to bind in");
        std::os::unix::net::UnixListener::bind(&path).unwrap_or_else(|e| panic!("cannot bind {path:?}: {e}"));
        let _ = std::fs::remove_file(&path);
    }
}

/// The gateway dials what the process bound.
///
/// Both derive this path independently and never exchange it, so agreeing on
/// the *rule* is not enough -- they have to start from the same run directory.
/// The gateway takes it from the service socket (`{run}/service.sock`); the
/// process took it by walking two levels up from its own IPC socket, which is
/// `{run}` only while that socket was not shortened.
///
/// With the directory now created either way, the process binds successfully
/// at `/tmp/instances/...` and exec still works -- so a full-chain boot test
/// passes while the terminal is dialling somewhere else entirely. That is why
/// this asserts the paths, not that something bound.
#[test]
fn the_gateway_and_the_process_agree_on_a_long_run_dir() {
    let run_dir = PathBuf::from("/private/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/capsem-test-xw4j_rzq/run");
    let id = "bb61246d-1dec-489f-8a2f-48c263fe4d5c";

    // What the gateway does: the service socket's directory.
    let service_uds = run_dir.join("service.sock");
    let gateway = terminal_socket_path(service_uds.parent().unwrap(), id).expect("socket path");

    // What the process does: the run directory it was handed.
    let process = terminal_socket_path(&run_dir, id).expect("socket path");
    assert_eq!(gateway, process);

    // And what it did before, from its own -- shortened -- IPC socket.
    let ipc = instance_socket_path(&run_dir, id).expect("socket path");
    let walked_up = ipc.parent().and_then(|p| p.parent()).unwrap();
    assert_ne!(
        terminal_socket_path(walked_up, id).expect("socket path"),
        gateway,
        "walking up a shortened IPC path binds a socket the gateway never dials"
    );
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
