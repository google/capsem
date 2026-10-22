use std::fs::File;
use std::path::{Path, PathBuf};

use capsem_foundation::unix::worker_sandbox::{Access, Policy, Role};

use super::*;

fn touch(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    File::create(path).unwrap();
    path.to_path_buf()
}

fn session_layout(root: &Path) {
    std::fs::create_dir_all(capsem_core::guest_share_dir(root).join("workspace")).unwrap();
    capsem_core::session::prepare_session_logs(root).unwrap();
    std::fs::create_dir_all(capsem_core::session::image_share_path(root)).unwrap();
    touch(capsem_core::session::system_overlay_image_path(root));
}

#[test]
fn owner_session_policy_accepts_only_materialized_canonical_paths() {
    let directory = tempfile::tempdir().unwrap();
    session_layout(directory.path());

    grant_owner_session_paths(Policy::new(Role::VmOwner), directory.path()).unwrap();

    let missing = directory.path().join("missing");
    let error = grant_owner_path(Policy::new(Role::VmOwner), &missing, Access::ReadOnly).unwrap_err();
    assert!(format!("{error:#}").contains("resolve sandbox grant"));
}

#[test]
fn owner_policy_grants_only_the_null_device_needed_for_router_stdio() {
    let policy = grant_owner_null(Policy::new(Role::VmOwner)).unwrap();
    let null = Path::new("/dev/null").canonicalize().unwrap();
    assert!(policy
        .paths()
        .iter()
        .any(|rule| rule.path() == null && rule.access() == Access::ReadWrite));
}

#[cfg(target_os = "macos")]
#[test]
fn owner_log_rotation_survives_real_confinement() {
    let (_directory, root) = run_log_sandbox_probe("rotation");
    assert_eq!(
        capsem_core::session::read_session_serial_tail(&root, 1024)
            .unwrap()
            .unwrap(),
        b"ccccddddeeee"
    );
    assert_eq!(
        capsem_core::session::read_session_transcript(&root).unwrap().unwrap(),
        [vec![b'c'; 80], vec![b'd'; 80], b"current marker".to_vec()].concat()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn owner_cannot_redirect_the_privileged_transcript_reader() {
    let (_directory, root) = run_log_sandbox_probe("redirect");
    assert!(capsem_core::session::read_session_transcript(&root).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn owner_can_read_its_immutable_code_without_granting_the_binary_directory() {
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let policy = grant_owner_binaries(Policy::new(Role::VmOwner)).unwrap();
    assert!(policy
        .paths()
        .iter()
        .any(|rule| rule.path() == executable && rule.access() == Access::ReadOnly));
    assert!(!policy
        .paths()
        .iter()
        .any(|rule| rule.path() == executable.parent().unwrap()));
    run_log_sandbox_probe("identity");
}

#[cfg(target_os = "macos")]
fn run_log_sandbox_probe(mode: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("session");
    session_layout(&root);
    touch(root.join("session.db"));
    touch(root.join("metadata.json"));
    let unrelated = directory.path().join("host-only");
    let fixture = capsem_core::pty_log::PtyLog::open(&unrelated).unwrap();
    fixture.record_output(b"unrelated host fixture");
    drop(fixture);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .args([
            "--exact",
            "owner_sandbox::tests::owner_log_sandbox_child",
            "--nocapture",
        ])
        .env("CAPSEM_OWNER_LOG_ROOT", &root)
        .env("CAPSEM_OWNER_LOG_MODE", mode)
        .env("CAPSEM_OWNER_LOG_UNRELATED", &unrelated)
        .env(
            "LLVM_PROFILE_FILE",
            root.join(capsem_core::session::OWNER_STATE_DIR)
                .join("profile-%p-%m.profraw"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (directory, root)
}

#[cfg(target_os = "macos")]
#[test]
fn owner_log_sandbox_child() {
    let Some(root) = std::env::var_os("CAPSEM_OWNER_LOG_ROOT") else {
        return;
    };
    let root = PathBuf::from(root).canonicalize().unwrap();
    let unrelated = PathBuf::from(std::env::var_os("CAPSEM_OWNER_LOG_UNRELATED").unwrap());
    let mode = std::env::var("CAPSEM_OWNER_LOG_MODE").unwrap();
    let log_dir = root.join(capsem_core::session::OWNER_STATE_DIR).join("logs");
    let log = capsem_core::pty_log::PtyLog::open_with_max(&log_dir.join("pty.log"), 100).unwrap();
    let mut serial = capsem_foundation::telemetry::CappedLogWriter::open(&log_dir.join("serial.log"), 8)
        .unwrap()
        .shared();
    let mut terminal = serial.clone();
    let executable = std::env::current_exe().unwrap();
    use std::io::Read as _;
    let mut expected = [0; 4];
    File::open(&executable).unwrap().read_exact(&mut expected).unwrap();
    let policy = grant_owner_session_paths(Policy::new(Role::VmOwner), &root).unwrap();
    let policy = grant_owner_binaries(policy).unwrap();
    capsem_foundation::unix::worker_sandbox::confine(&policy).unwrap();
    for denied in [&unrelated, &root.join("session.db"), &root.join("metadata.json")] {
        assert_eq!(
            std::fs::read(denied).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    assert_eq!(
        std::fs::write(root.join("ungranted"), b"denied").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    if mode == "identity" {
        let mut prefix = [0; 4];
        File::open(&executable).unwrap().read_exact(&mut prefix).unwrap();
        assert_eq!(prefix, expected);
    } else if mode == "redirect" {
        drop(log);
        std::fs::remove_file(log_dir.join("pty.log")).unwrap();
        std::os::unix::fs::symlink(unrelated, log_dir.join("pty.log")).unwrap();
    } else {
        // Start after confinement, as in the real VM owner. Rotation happens
        // on the inherited-policy worker, not on this already-confined thread.
        let recorder = capsem_core::pty_log::PtyRecorder::new(log).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        recorder.record_output(&[b'a'; 80]).unwrap();
        recorder.record_output(&[b'b'; 80]).unwrap();
        runtime.block_on(recorder.flush()).unwrap();
        assert!(
            log_dir.join("pty.log.1").exists(),
            "rotation was denied under owner confinement"
        );
        assert_eq!(std::fs::metadata(log_dir.join("pty.log")).unwrap().len(), 0);
        recorder.record_output(&[b'c'; 80]).unwrap();
        recorder.record_output(&[b'd'; 80]).unwrap();
        recorder.record_output(b"current marker").unwrap();
        runtime.block_on(recorder.shutdown()).unwrap();
        drop(recorder);
        drop(runtime);
        use std::io::Write;
        serial.write_all(b"aaaa").unwrap();
        terminal.write_all(b"bbbb").unwrap();
        serial.write_all(b"cccc").unwrap();
        terminal.write_all(b"dddd").unwrap();
        serial.write_all(b"eeee").unwrap();
        assert_eq!(std::fs::read(log_dir.join("serial.1.log")).unwrap(), b"ccccdddd");
        assert_eq!(std::fs::read(log_dir.join("serial.log")).unwrap(), b"eeee");
    }
}

#[test]
fn owner_attestation_prepares_private_direct_and_brokered_targets() {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    session_layout(directory.path());

    let attestation = prepare_owner_sandbox_attestation(directory.path()).unwrap();

    assert!(attestation
        .direct_path
        .starts_with(directory.path().join(capsem_core::session::OWNER_STATE_DIR)));
    assert_eq!(
        std::fs::metadata(&attestation.direct_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        attestation.guest_path,
        capsem_core::guest_share_dir(directory.path()).join(std::ffi::OsStr::from_bytes(&attestation.guest_relative))
    );
    assert!(attestation.guest_path.exists());
    assert_eq!(attestation.ledger_path, directory.path().join("session.db"));

    drop(attestation.direct_file);
    std::fs::remove_file(attestation.direct_path).unwrap();
    std::fs::remove_file(attestation.guest_path).unwrap();
}
