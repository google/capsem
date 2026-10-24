use super::*;

#[test]
fn native_file_fixture_obeys_the_real_guest_workspace_boundary() {
    let path = guest_file_path();
    let relative = StdPath::new(&path)
        .strip_prefix(capsem_proto::GUEST_WORKSPACE)
        .expect("native fixture must reach FileContent, not the guest path refusal");
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join(relative);
    std::fs::write(&file, BYTES).unwrap();
    capsem_proto::validate_file_path_safe(file.to_str().unwrap(), workspace.path()).unwrap();
}

#[tokio::test]
async fn native_startup_cannot_substitute_a_unit_proxy_worker() {
    let mut state = crate::tests::make_test_state();
    // Keep this negative fixture alive, but select the same non-fixture mode
    // as real startup. No fake state is used by the native positive owner.
    let directory = Arc::get_mut(&mut state).unwrap()._test_tempdir.take();
    assert!(state
        .proxy_worker("missing-native-owner", capsem_crypto::uuid_v4())
        .is_err());
    drop(directory);
}

#[test]
fn missing_or_foreign_native_generation_is_rejected_before_startup() {
    let directory = tempfile::tempdir().unwrap();
    assert!(verify(directory.path(), b"{}").is_err());
    let receipt = serde_json::json!({
        "schema": "capsem.native-file-content.v1",
        "source_commit": "a".repeat(40),
        "source_tree": "b".repeat(40),
        "source_digest": "d".repeat(64),
        "binaries": {"capsem-process": "c".repeat(64)}
    });
    assert!(verify(directory.path(), &serde_json::to_vec(&receipt).unwrap()).is_err());
}

#[test]
fn complete_receipt_rejects_replaced_or_symlinked_companion_bytes() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let mut binaries = BTreeMap::new();
    for role in roles() {
        let path = directory.path().join(&role);
        std::fs::write(&path, role.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        let digest = capsem_crypto::blake3::hash(role.as_bytes()).to_hex().to_string();
        binaries.insert(role, digest);
    }
    let receipt = serde_json::to_vec(&serde_json::json!({
        "schema": "capsem.native-file-content.v1",
        "source_commit": "a".repeat(40), "source_tree": "b".repeat(40),
        "source_digest": "d".repeat(64),
        "binaries": binaries,
    }))
    .unwrap();
    verify(directory.path(), &receipt).unwrap();
    let process = directory.path().join("capsem-process");
    std::fs::set_permissions(&process, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&process, b"foreign generation").unwrap();
    assert!(verify(directory.path(), &receipt)
        .unwrap_err()
        .to_string()
        .contains("foreign host generation"));
    std::fs::remove_file(&process).unwrap();
    symlink(directory.path().join("capsem-service"), &process).unwrap();
    assert!(verify(directory.path(), &receipt)
        .unwrap_err()
        .to_string()
        .contains("invalid host role"));
}
