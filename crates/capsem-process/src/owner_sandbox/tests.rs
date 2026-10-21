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
    std::fs::create_dir_all(root.join(capsem_core::session::OWNER_STATE_DIR)).unwrap();
    std::fs::create_dir_all(capsem_core::session::image_share_path(root)).unwrap();
    touch(capsem_core::session::system_overlay_image_path(root));
    touch(root.join("serial.log"));
    touch(root.join("pty.log"));
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
fn owner_cannot_redirect_the_privileged_transcript_reader() {
    let (_directory, root) = run_log_sandbox_probe("redirect");
    assert!(capsem_core::pty_log::read_output_bytes(&root.join("pty.log")).is_err());
}

#[cfg(target_os = "macos")]
fn run_log_sandbox_probe(mode: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("session");
    session_layout(&root);
    touch(root.join("session.db"));
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
    let log = capsem_core::pty_log::PtyLog::open_with_max(&root.join("pty.log"), 100).unwrap();
    let policy = grant_owner_session_paths(Policy::new(Role::VmOwner), &root).unwrap();
    capsem_foundation::unix::worker_sandbox::confine(&policy).unwrap();
    for denied in [&unrelated, &root.join("session.db")] {
        assert_eq!(
            std::fs::read(denied).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    if mode == "redirect" {
        drop(log);
        std::fs::remove_file(root.join("pty.log")).unwrap();
        std::os::unix::fs::symlink(unrelated, root.join("pty.log")).unwrap();
    } else {
        log.record_output(&[b'a'; 80]);
        log.record_output(&[b'b'; 80]);
        assert!(
            root.join("pty.log.1").exists(),
            "rotation was denied under owner confinement"
        );
        assert_eq!(std::fs::metadata(root.join("pty.log")).unwrap().len(), 0);
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
