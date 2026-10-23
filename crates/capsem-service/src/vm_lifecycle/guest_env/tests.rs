use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn staged_guest_env_is_private_and_preserves_user_values() {
    let dir = tempfile::tempdir().unwrap();
    let env = HashMap::from([("TOKEN".to_string(), "secret=tail".to_string())]);
    stage(dir.path(), "vm-id", "vm-name", Some(&env)).unwrap();
    let path = dir.path().join(FILE_NAME);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.first(), Some(&0x81), "the handoff is a named MessagePack record");
    let values = rmp_serde::from_slice::<capsem_proto::guest_env::GuestEnvironment>(&bytes)
        .unwrap()
        .values;
    assert!(values.contains(&("CAPSEM_VM_ID".into(), "vm-id".into())));
    assert!(values.contains(&("CAPSEM_VM_NAME".into(), "vm-name".into())));
    assert!(values.contains(&("TOKEN".into(), "secret=tail".into())));
}

#[test]
fn oversized_guest_environment_is_refused_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let env = HashMap::from([("TOKEN".to_string(), "x".repeat(capsem_proto::guest_env::MAX_BYTES))]);
    assert!(stage(dir.path(), "vm", "name", Some(&env)).is_err());
    assert!(!dir.path().join(FILE_NAME).exists());
}

#[test]
fn launch_paths_never_put_guest_values_on_process_argv() {
    for source in [include_str!("../provision.rs"), include_str!("../resume_process.rs")] {
        assert!(source.contains("guest_env::stage("));
        assert!(!source.contains("arg(\"--env\")"));
    }
}
