//! `POST /vms/{id}/fork`: the owner-run clone, its refusals and its registry entry.

use super::*;

/// A running sandbox whose fake owner clones on `CloneState`, or refuses.
fn running_fork_source(state: &Arc<ServiceState>, dir: &tempfile::TempDir, refusal: Option<&'static str>) {
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    std::fs::write(session_dir.join("guest/workspace/notes.txt"), b"hello").unwrap();
    let uds_path = dir.path().join("fork-src.sock");
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            id: "fork-src".into(),
            name: "fork-src".into(),
            uds_path: uds_path.clone(),
            session_dir: session_dir.clone(),
            ..test_instance()
        },
    );
    spawn_fake_fork_owner(&uds_path, session_dir, refusal);
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    running_fork_source(&state, &dir, None);
    let result = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: Some("test".into()),
        }),
    )
    .await
    .unwrap();
    assert_ne!(result.0.id, "my-fork");
    uuid::Uuid::parse_str(&result.0.id).expect("fork response id should be a UUID");
    assert_eq!(result.0.name, "my-fork");
    assert!(result.0.size_bytes > 0);
    // Verify fork created a persistent sandbox entry in the registry
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("my-fork").unwrap();
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
    assert_eq!(entry.forked_from, Some("fork-src".into()));
    assert_eq!(entry.description, Some("test".into()));
    assert_eq!(entry.base_version, "0.0.0");
    // The owner cloned into the directory the service registered.
    assert_eq!(
        std::fs::read(entry.session_dir.join("guest/workspace/notes.txt")).unwrap(),
        b"hello"
    );
    drop(registry);
}

#[tokio::test]
async fn a_fork_whose_guest_will_not_freeze_fails_and_leaves_nothing() {
    // Copying a live ext4 overlay without the freeze can yield an image the
    // fork cannot boot, so a refused freeze is a refused fork.
    let (state, dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    running_fork_source(&state, &dir, Some("timed out waiting for SnapshotReady"));
    let error = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert!(error.1.contains("SnapshotReady"), "{}", error.1);
    assert!(state.persistent_registry.lock().unwrap().get("my-fork").is_none());
    let persistent = state.run_dir.join("persistent");
    let leftovers = std::fs::read_dir(&persistent).map(|dir| dir.count()).unwrap_or(0);
    assert_eq!(
        leftovers,
        0,
        "a failed fork left a directory under {}",
        persistent.display()
    );
}

#[tokio::test]
async fn handle_fork_not_found() {
    let (state, _dir) = make_test_state_with_tempdir();
    // state is already Arc<ServiceState> from make_test_state*
    let err = handle_fork(
        State(state),
        Path("ghost".into()),
        Json(ForkRequest {
            name: "img".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_fork_duplicate_returns_conflict() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("sessions/dup-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "dup-src".into(),
        InstanceInfo {
            id: "dup-src".into(),
            name: "dup-src".into(),
            uds_path: dir.path().join("dup-src.sock"),
            session_dir: session_dir.clone(),
            ..test_instance()
        },
    );
    spawn_fake_fork_owner(&dir.path().join("dup-src.sock"), session_dir, None);
    // state is already Arc<ServiceState> from make_test_state*
    // First fork succeeds
    let _ = handle_fork(
        State(state.clone()),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    // Second fork with same name returns CONFLICT
    let err = handle_fork(
        State(state),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn handle_fork_from_persistent_registry() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/pers-vm");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pers-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                created_at: "2026-01-01T00:00:00Z".into(),
                ..test_persistent_entry("pers-vm", session_dir.clone())
            },
        );
    }
    // state is already Arc<ServiceState> from make_test_state*
    let result = handle_fork(
        State(state.clone()),
        Path(vm_id),
        Json(ForkRequest {
            name: "from-pers".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    assert_ne!(result.0.id, "from-pers");
    uuid::Uuid::parse_str(&result.0.id).expect("fork response id should be a UUID");
    assert_eq!(result.0.name, "from-pers");
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("from-pers").unwrap();
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
    drop(registry);
}

#[tokio::test]
async fn handle_fork_rejects_asset_pin_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/pin-drift");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let mut pins = test_asset_pins();
    pins.rootfs.hash = "blake3:0000000000000000000000000000000000000000000000000000000000000000".into();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pin-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "pin-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: pins,
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let err = handle_fork(
        State(state),
        Path(vm_id),
        Json(ForkRequest {
            name: "blocked-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::PRECONDITION_FAILED);
    assert!(
        err.1.contains("asset pins changed"),
        "fork must fail closed on asset pin drift, got: {}",
        err.1
    );
}
