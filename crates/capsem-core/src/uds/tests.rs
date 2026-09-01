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
    let path = terminal_socket_path(&run_dir, "vm-1");

    assert_eq!(path, PathBuf::from("/tmp/r/instances/vm-1-ws.sock"));
}

#[test]
fn terminal_socket_fits_under_a_long_run_dir() {
    let run_dir = PathBuf::from(
        "/Users/somebody/git/capsem/cache/target/ironbank-assets/co-work/home/.capsem/run",
    );
    let path = terminal_socket_path(&run_dir, "322e7460-f1b2-4fdd-88f1-0c4b58c48e46");

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
    let run_dir = PathBuf::from(
        "/Users/somebody/git/capsem/cache/target/ironbank-assets/co-work/home/.capsem/run",
    );
    let id = "322e7460-f1b2-4fdd-88f1-0c4b58c48e46";

    assert_eq!(
        terminal_socket_path(&run_dir, id),
        terminal_socket_path(&run_dir, id)
    );
    assert_ne!(
        terminal_socket_path(&run_dir, id),
        terminal_socket_path(&run_dir, "0acea121-db0b-431e-91f3-c51291fa64fc")
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
    let run_dir = PathBuf::from(
        "/private/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/capsem-test-xw4j_rzq/run",
    );
    let id = "bb61246d-1dec-489f-8a2f-48c263fe4d5c";

    let ipc = instance_socket_path(&run_dir, id);
    assert!(ipc.starts_with("/tmp/capsem"), "this run dir must shorten: {ipc:?}");

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
        let path = terminal_socket_path(temp.path(), id);
        assert!(
            path.parent().unwrap().is_dir(),
            "{path:?} has no directory to bind in"
        );
        std::os::unix::net::UnixListener::bind(&path)
            .unwrap_or_else(|e| panic!("cannot bind {path:?}: {e}"));
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
    let run_dir = PathBuf::from(
        "/private/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/capsem-test-xw4j_rzq/run",
    );
    let id = "bb61246d-1dec-489f-8a2f-48c263fe4d5c";

    // What the gateway does: the service socket's directory.
    let service_uds = run_dir.join("service.sock");
    let gateway = terminal_socket_path(service_uds.parent().unwrap(), id);

    // What the process does: the run directory it was handed.
    let process = terminal_socket_path(&run_dir, id);
    assert_eq!(gateway, process);

    // And what it did before, from its own -- shortened -- IPC socket.
    let ipc = instance_socket_path(&run_dir, id);
    let walked_up = ipc.parent().and_then(|p| p.parent()).unwrap();
    assert_ne!(
        terminal_socket_path(walked_up, id),
        gateway,
        "walking up a shortened IPC path binds a socket the gateway never dials"
    );
}
