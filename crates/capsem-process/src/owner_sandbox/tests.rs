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
    touch(aggregator_log_path(root));
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
