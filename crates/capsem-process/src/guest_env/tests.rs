use super::*;

#[test]
fn consumes_private_env_with_equals_and_removes_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let values = capsem_proto::guest_env::GuestEnvironment {
        values: vec![("TOKEN".into(), "secret=tail".into())],
    };
    let bytes = capsem_foundation::ipc_channel::encode_record(&values, capsem_proto::guest_env::MAX_BYTES).unwrap();
    capsem_foundation::unix::fs::atomic_write_private(&path, &bytes).unwrap();
    assert_eq!(load(dir.path()).unwrap(), vec![("TOKEN".into(), "secret=tail".into())]);
    assert!(!path.exists());
}

#[test]
fn json_is_not_accepted_as_a_guest_environment_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    capsem_foundation::unix::fs::atomic_write_private(&path, br#"[["TOKEN","secret"]]"#).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(path.exists(), "a refused handoff is never reported as consumed");
}

#[test]
fn oversized_handoff_is_refused_without_reading_or_consuming_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    capsem_foundation::unix::fs::atomic_write_private(&path, &vec![0; capsem_proto::guest_env::MAX_BYTES + 1]).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(path.exists());
}

#[test]
fn refuses_symlink_and_world_readable_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let target = dir.path().join("outside");
    let values = capsem_proto::guest_env::GuestEnvironment {
        values: vec![("TOKEN".into(), "secret".into())],
    };
    std::fs::write(
        &target,
        capsem_foundation::ipc_channel::encode_record(&values, capsem_proto::guest_env::MAX_BYTES).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(load(dir.path()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::copy(&target, &path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(path.exists());
}
