use super::*;
use crate::managed_sessions::VmBinding;
use capsem_foundation::unix::contained::ContainedDir;

#[test]
fn private_spawn_identity_reopens_and_replaces_without_touching_old_inode() {
    let root = tempfile::tempdir().unwrap();
    let session = ContainedDir::open_root(root.path()).unwrap();
    assert!(read_spawn_identity(&session).unwrap().is_none());
    let original = VmBinding::new("vm-original".into(), uuid::Uuid::new_v4()).unwrap();
    write_spawn_identity(&session, &original).unwrap();
    let reopened = ContainedDir::open_root(root.path()).unwrap();
    assert_eq!(read_spawn_identity(&reopened).unwrap(), Some(original));
    let held = std::fs::read(root.path().join(SPAWN_IDENTITY_FILE)).unwrap();
    std::fs::hard_link(root.path().join(SPAWN_IDENTITY_FILE), root.path().join("held-original")).unwrap();
    let replacement = VmBinding::new("vm-replacement".into(), uuid::Uuid::new_v4()).unwrap();
    write_spawn_identity(&session, &replacement).unwrap();
    assert_eq!(read_spawn_identity(&reopened).unwrap(), Some(replacement));
    assert_eq!(std::fs::read(root.path().join("held-original")).unwrap(), held);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(root.path().join(SPAWN_IDENTITY_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!String::from_utf8_lossy(&held).contains("capability"));
}

#[test]
fn spawn_identity_refuses_symlinks_corrupt_records_and_nil_generations() {
    let root = tempfile::tempdir().unwrap();
    let session = ContainedDir::open_root(root.path()).unwrap();
    let binding = VmBinding::new("vm".into(), uuid::Uuid::new_v4()).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), b"outside survives").unwrap();
    let record = root.path().join(SPAWN_IDENTITY_FILE);
    std::os::unix::fs::symlink(outside.path(), &record).unwrap();
    assert!(read_spawn_identity(&session).is_err());
    assert!(write_spawn_identity(&session, &binding).is_err());
    assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside survives");
    std::fs::remove_file(&record).unwrap();
    std::fs::write(&record, b"corrupt").unwrap();
    assert!(write_spawn_identity(&session, &binding).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), b"corrupt");
    let invalid = serde_json::json!({"schema_version":1,"binding":{"id":"vm","generation":uuid::Uuid::nil()}});
    std::fs::write(&record, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(read_spawn_identity(&session).is_err());
    assert!(write_spawn_identity(&session, &binding).is_err());
}
