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
            labels: Some(HashMap::from([("suite".into(), "eval".into())])),
            ..test_instance()
        },
    );
    spawn_fake_fork_owner(&uds_path, session_dir, refusal);
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    running_fork_source(&state, &dir, None);
    let result = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: Some("test".into()),
            labels: None,
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
    assert_eq!(entry.asset_pins, test_asset_pins());
    assert!(entry.legacy_profile_id.is_none());
    assert_eq!(entry.asset_pins, test_asset_pins());
    assert_eq!(entry.forked_from, Some("fork-src".into()));
    assert_eq!(entry.description, Some("test".into()));
    assert_eq!(
        entry.labels,
        Some(HashMap::from([("suite".into(), "eval".into())])),
        "forked VM must inherit source labels by default"
    );
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
    install_test_runtime_assets(&state);
    running_fork_source(&state, &dir, Some("timed out waiting for SnapshotReady"));
    let error = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: None,
            labels: None,
        }),
    )
    .await
    .unwrap_err();
    assert!(error.body.error.contains("SnapshotReady"), "{}", error.body.error);
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
            labels: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_fork_duplicate_returns_conflict() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
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
            labels: None,
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
            labels: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn handle_fork_from_persistent_registry() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
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
                labels: Some(HashMap::from([("suite".into(), "eval".into())])),
                ..test_persistent_entry("pers-vm", session_dir.clone())
            },
        );
    }
    // state is already Arc<ServiceState> from make_test_state*
    let result = handle_fork(
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            name: "from-pers".into(),
            description: None,
            labels: None,
        }),
    )
    .await
    .unwrap();
    assert_ne!(result.0.id, "from-pers");
    uuid::Uuid::parse_str(&result.0.id).expect("fork response id should be a UUID");
    assert_eq!(result.0.name, "from-pers");
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("from-pers").unwrap();
    assert_eq!(entry.asset_pins, test_asset_pins());
    assert!(entry.legacy_profile_id.is_none());
    assert_eq!(entry.asset_pins, test_asset_pins());
    assert_eq!(
        entry.labels,
        Some(HashMap::from([("suite".into(), "eval".into())])),
        "forked persistent VM must inherit source labels by default"
    );
    drop(registry);

    // Overriding labels on fork replaces source labels; empty map `{}` clears them to `None`.
    let overridden = HashMap::from([("suite".into(), "override".into())]);
    let _ = handle_fork(
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            name: "from-pers-override".into(),
            description: None,
            labels: Some(overridden.clone()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .persistent_registry
            .lock()
            .unwrap()
            .get("from-pers-override")
            .unwrap()
            .labels,
        Some(overridden)
    );

    let _ = handle_fork(
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            name: "from-pers-cleared".into(),
            description: None,
            labels: Some(HashMap::new()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .persistent_registry
            .lock()
            .unwrap()
            .get("from-pers-cleared")
            .unwrap()
            .labels,
        None,
        "empty labels map on fork must clear inherited labels to None"
    );

    let bad = handle_fork(
        State(state.clone()),
        Path(vm_id),
        Json(ForkRequest {
            name: "from-pers-bad".into(),
            description: None,
            labels: Some(HashMap::from([("dot.key".into(), "v".into())])),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body.error);
}

/// A VM from before profiles were removed is refused, not laundered into a
/// fork in the current shape.
#[tokio::test]
async fn handle_fork_refuses_a_profile_era_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("persistent/profile-era");
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    let entry = PersistentVmEntry {
        legacy_profile_id: Some("code".into()),
        ..test_persistent_entry("profile-era", session_dir)
    };
    let vm_id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("profile-era".into(), entry);

    let err = handle_fork(
        State(Arc::clone(&state)),
        Path(vm_id),
        Json(ForkRequest {
            name: "blocked-fork".into(),
            description: None,
            labels: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, StatusCode::PRECONDITION_FAILED);
    assert!(
        err.body.error.contains("profiles no longer exist"),
        "{}",
        err.body.error
    );
    assert!(!state.persistent_registry.lock().unwrap().contains("blocked-fork"));
}
