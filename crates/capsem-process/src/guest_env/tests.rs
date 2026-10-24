use super::*;

#[test]
fn consumes_private_env_with_equals_and_removes_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    capsem_foundation::unix::fs::atomic_write_private(&path, br#"[["TOKEN","secret=tail"]]"#).unwrap();
    assert_eq!(load(dir.path()).unwrap(), vec![("TOKEN".into(), "secret=tail".into())]);
    assert!(!path.exists());
}

#[test]
fn refuses_symlink_and_world_readable_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let target = dir.path().join("outside");
    std::fs::write(&target, br#"[["TOKEN","secret"]]"#).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(load(dir.path()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::copy(&target, &path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(path.exists());
}
