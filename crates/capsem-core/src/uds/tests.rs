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
        "/Users/somebody/git/capsem/target/ironbank-assets/co-work/home/.capsem/run",
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
        "/Users/somebody/git/capsem/target/ironbank-assets/co-work/home/.capsem/run",
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
