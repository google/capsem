use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn staged_guest_env_is_private_and_preserves_user_values() {
    let dir = tempfile::tempdir().unwrap();
    let env = HashMap::from([("TOKEN".to_string(), "secret=tail".to_string())]);
    stage(dir.path(), "vm-id", "vm-name", Some(&env)).unwrap();
    let path = dir.path().join(FILE_NAME);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let values: Vec<(String, String)> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(values.contains(&("CAPSEM_VM_ID".into(), "vm-id".into())));
    assert!(values.contains(&("CAPSEM_VM_NAME".into(), "vm-name".into())));
    assert!(values.contains(&("TOKEN".into(), "secret=tail".into())));
}

#[test]
fn launch_paths_never_put_guest_values_on_process_argv() {
    for source in [include_str!("../provision.rs"), include_str!("../resume_process.rs")] {
        assert!(source.contains("guest_env::stage("));
        assert!(!source.contains("arg(\"--env\")"));
    }
}
