//! `POST /vms/{id}/fork`: the owner-run clone, its refusals and its registry entry.

use super::*;

/// A running sandbox whose fake owner clones on `CloneState`, or refuses.
fn running_fork_source(
    state: &Arc<ServiceState>,
    dir: &tempfile::TempDir,
    refusal: Option<&'static str>,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    running_fork_source_gated(state, dir, refusal, None)
}

fn running_fork_source_gated(
    state: &Arc<ServiceState>,
    dir: &tempfile::TempDir,
    refusal: Option<&'static str>,
    gate: Option<Arc<ForkGate>>,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    std::fs::write(session_dir.join("guest/workspace/notes.txt"), b"hello").unwrap();
    let uds_path = dir.path().join("fork-src.sock");
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            environment: None,
            terminal: None,
            browser_request: None,
            browser_revision: 0,
            id: "fork-src".into(),
            name: "fork-src".into(),
            uds_path: uds_path.clone(),
            session_dir: session_dir.clone(),
            labels: Some(HashMap::from([("suite".into(), "eval".into())])),
            ..test_instance()
        },
    );
    match gate {
        None => spawn_fake_fork_owner(&uds_path, session_dir, refusal),
        Some(gate) => spawn_fake_fork_owner_gated(&uds_path, session_dir, refusal, Some(gate)),
    }
}

#[tokio::test]
async fn fork_stale_expectation_refuses_before_source_freeze_or_destination_allocation() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let owner = running_fork_source(&state, &directory, Some("source must not be frozen for a stale plan"));
    let mut expected = state
        .lifecycle_selection("fork-src", capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    expected.generation = Some("00000000-0000-4000-8000-000000000009".into());
    let app = crate::build_service_router(state.clone());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/vms/fork-src/fork",
        Some(json!({
            "name":"blocked-fork", "expected": expected,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        !owner.is_finished(),
        "stale selection must not ask the native source to freeze"
    );
    assert!(state.persistent_registry.lock().unwrap().get("blocked-fork").is_none());
    assert_eq!(
        std::fs::read_dir(state.run_dir.join("persistent"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    owner.abort();
    let _ = owner.await;
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    running_fork_source(&state, &dir, None);
    let result = handle_fork(
        crate::tests::operator_actor(),
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            expected: None,
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
        crate::tests::operator_actor(),
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            expected: None,
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
        crate::tests::operator_actor(),
        State(state),
        Path("ghost".into()),
        Json(ForkRequest {
            expected: None,
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
            environment: None,
            terminal: None,
            browser_request: None,
            browser_revision: 0,
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
        crate::tests::operator_actor(),
        State(state.clone()),
        Path("dup-src".into()),
        Json(ForkRequest {
            expected: None,
            name: "same-name".into(),
            description: None,
            labels: None,
        }),
    )
    .await
    .unwrap();
    // Second fork with same name returns CONFLICT
    let err = handle_fork(
        crate::tests::operator_actor(),
        State(state),
        Path("dup-src".into()),
        Json(ForkRequest {
            expected: None,
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
                environment: None,
                id: vm_id.clone(),
                created_at: "2026-01-01T00:00:00Z".into(),
                labels: Some(HashMap::from([("suite".into(), "eval".into())])),
                ..test_persistent_entry("pers-vm", session_dir.clone())
            },
        );
    }
    // state is already Arc<ServiceState> from make_test_state*
    let result = handle_fork(
        crate::tests::operator_actor(),
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            expected: None,
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
        crate::tests::operator_actor(),
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            expected: None,
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
        crate::tests::operator_actor(),
        State(state.clone()),
        Path(vm_id.clone()),
        Json(ForkRequest {
            expected: None,
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
        crate::tests::operator_actor(),
        State(state.clone()),
        Path(vm_id),
        Json(ForkRequest {
            expected: None,
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
        environment: None,
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
        crate::tests::operator_actor(),
        State(Arc::clone(&state)),
        Path(vm_id),
        Json(ForkRequest {
            expected: None,
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

fn fork_reference(revision: u64) -> capsem_logger::events::EnvironmentRevisionParent {
    capsem_logger::events::EnvironmentRevisionParent {
        environment_id: "00000000-0000-4000-8000-000000000001".into(),
        revision_id: format!("00000000-0000-4000-8000-{revision:012x}"),
    }
}

#[tokio::test]
async fn fork_target_running_generation_and_revision_expectations_are_literal_preconditions() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let owner = running_fork_source(&state, &directory, Some("stale selection must not freeze"));
    state.instances.lock().unwrap().get_mut("fork-src").unwrap().environment = Some(
        crate::environment_launch::EnvironmentBinding::starting(fork_reference(2), "pending".into()),
    );
    let original = state
        .lifecycle_selection("fork-src", capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    let app = crate::build_service_router(state.clone());
    for (case, expected_status) in [
        ("target", StatusCode::BAD_REQUEST),
        ("generation", StatusCode::CONFLICT),
        ("running", StatusCode::CONFLICT),
        ("revision", StatusCode::CONFLICT),
        ("absence", StatusCode::CONFLICT),
    ] {
        let mut expected = original.clone();
        match case {
            "target" => expected.id = "other-vm".into(),
            "generation" => expected.generation = Some(capsem_crypto::uuid_v4().to_string()),
            "running" => expected.running = false,
            "revision" => expected.environment.as_mut().unwrap().revision_id = fork_reference(3).revision_id,
            "absence" => expected.environment = None,
            _ => unreachable!(),
        }
        let (status, body) = route_request(
            app.clone(),
            axum::http::Method::POST,
            "/vms/fork-src/fork",
            Some(json!({"name":format!("blocked-{case}"), "expected":expected})),
        )
        .await;
        assert_eq!(status, expected_status, "case={case} {body}");
        assert!(!owner.is_finished());
        assert!(state
            .persistent_registry
            .lock()
            .unwrap()
            .get(&format!("blocked-{case}"))
            .is_none());
    }
    assert_eq!(
        std::fs::read_dir(state.run_dir.join("persistent"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    owner.abort();
    let _ = owner.await;
}

#[tokio::test]
async fn qualified_fork_carries_desired_revision_but_never_source_spawn_authority() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let owner = running_fork_source(&state, &directory, None);
    let (session, generation) = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut("fork-src").unwrap();
        let mut binding = crate::environment_launch::EnvironmentBinding::starting(fork_reference(2), "pending".into());
        binding.desired = fork_reference(3);
        instance.environment = Some(binding);
        let captured = (instance.session_dir.clone(), instance.generation);
        drop(instances);
        captured
    };
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(&session).unwrap();
    capsem_core::session::write_spawn_identity(
        &root,
        &capsem_core::managed_sessions::VmBinding::new("fork-src".into(), generation).unwrap(),
    )
    .unwrap();
    let expected = state
        .lifecycle_selection("fork-src", capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    let app = crate::build_service_router(state.clone());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/vms/fork-src/fork",
        Some(json!({"name":"qualified-fork", "expected":expected})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entry = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("qualified-fork")
        .unwrap()
        .clone();
    assert_eq!(entry.environment, Some(fork_reference(3)));
    assert!(!entry.suspended);
    assert_eq!(
        std::fs::read(entry.session_dir.join("guest/workspace/notes.txt")).unwrap(),
        b"hello"
    );
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(&entry.session_dir).unwrap();
    assert!(capsem_core::session::read_spawn_identity(&root).unwrap().is_none());
    assert_eq!(owner.await.unwrap().len(), 2);
}

#[tokio::test]
async fn qualified_stopped_fork_uses_retained_generation_and_saved_revision() {
    let (state, _directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session = state.run_dir.join("persistent/source");
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::write(session.join("system/rootfs.img"), b"source disk").unwrap();
    let mut entry = test_persistent_entry("saved-source", session.clone());
    entry.environment = Some(fork_reference(2));
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let root = capsem_foundation::unix::contained::ContainedDir::open_root(&session).unwrap();
    let generation = capsem_crypto::uuid_v4();
    capsem_core::session::write_spawn_identity(
        &root,
        &capsem_core::managed_sessions::VmBinding::new(id.clone(), generation).unwrap(),
    )
    .unwrap();
    let expected = state
        .lifecycle_selection(&id, capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    assert_eq!(expected.generation.as_deref(), Some(generation.to_string().as_str()));
    assert!(!expected.running);
    let app = crate::build_service_router(state.clone());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        &format!("/vms/{id}/fork"),
        Some(json!({"name":"stopped-fork", "expected":expected})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entry = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("stopped-fork")
        .unwrap()
        .clone();
    assert_eq!(entry.environment, Some(fork_reference(2)));
    assert_eq!(entry.forked_from.as_deref(), Some(id.as_str()));
    assert!(!entry.session_dir.join("spawn-identity.json").exists());
}

#[tokio::test]
async fn fork_compares_queued_expectation_after_acquiring_policy_exclusion() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let owner = running_fork_source(&state, &directory, Some("queued stale source must not freeze"));
    let expected = state
        .lifecycle_selection("fork-src", capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let app = crate::build_service_router(state.clone());
    let request = tokio::spawn(async move {
        route_request(
            app,
            axum::http::Method::POST,
            "/vms/fork-src/fork",
            Some(json!({"name":"queued-fork", "expected":expected})),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    state.instances.lock().unwrap().get_mut("fork-src").unwrap().environment = Some(
        crate::environment_launch::EnvironmentBinding::starting(fork_reference(2), "pending".into()),
    );
    drop(mutation);
    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!owner.is_finished());
    assert!(state.persistent_registry.lock().unwrap().get("queued-fork").is_none());
    owner.abort();
    let _ = owner.await;
}

#[tokio::test]
async fn cancelled_fork_http_waiter_retains_owned_native_copy_until_registry_publication() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let gate = Arc::new(ForkGate::default());
    let owner = running_fork_source_gated(&state, &directory, None, Some(gate.clone()));
    let expected = state
        .lifecycle_selection("fork-src", capsem_api::VmLifecycleIntent::Fork)
        .unwrap();
    let request_state = state.clone();
    let request = tokio::spawn(async move {
        handle_fork(
            crate::tests::operator_actor(),
            State(request_state),
            Path("fork-src".into()),
            Json(ForkRequest {
                name: "retained-fork".into(),
                expected: Some(expected),
                description: None,
                labels: None,
            }),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.frozen.notified())
        .await
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        state.lifecycle.vz.try_write().is_err(),
        "native copy exclusion belongs to the service job"
    );
    gate.release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), owner)
            .await
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), state.lifecycle.wait_for_launches())
        .await
        .unwrap();
    let entry = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("retained-fork")
        .unwrap()
        .clone();
    assert_eq!(
        std::fs::read(entry.session_dir.join("guest/workspace/notes.txt")).unwrap(),
        b"hello"
    );
    assert!(!entry.session_dir.join("spawn-identity.json").exists());
    assert!(state.lifecycle.vz.try_write().is_ok());
}

#[tokio::test]
async fn fork_persistence_failure_identifies_the_retained_copy_for_reconciliation() {
    let (state, _directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session = state.run_dir.join("persistent/source");
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::write(session.join("system/rootfs.img"), b"source disk").unwrap();
    let source = test_persistent_entry("source", session);
    let id = source.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(source.name.clone(), source);
    std::fs::create_dir_all(state.run_dir.join("persistent_registry.json")).unwrap();
    let app = crate::build_service_router(state.clone());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        &format!("/vms/{id}/fork"),
        Some(json!({"name":"write-refused"})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let retained = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("write-refused")
        .unwrap()
        .clone();
    assert_eq!(
        body["vm_id"], retained.id,
        "a copied fork with unconfirmed persistence must remain identifiable"
    );
    assert_eq!(
        std::fs::read(retained.session_dir.join("system/rootfs.img")).unwrap(),
        b"source disk"
    );
}

#[tokio::test]
async fn fork_publication_name_race_removes_only_its_unregistered_destination() {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let gate = Arc::new(ForkGate::default());
    let owner = running_fork_source_gated(&state, &directory, None, Some(gate.clone()));
    let request_state = state.clone();
    let request = tokio::spawn(async move {
        handle_fork(
            crate::tests::operator_actor(),
            State(request_state),
            Path("fork-src".into()),
            Json(ForkRequest {
                name: "raced-fork".into(),
                expected: None,
                description: None,
                labels: None,
            }),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.frozen.notified())
        .await
        .unwrap();
    // Another registration path may claim the display name while the guest
    // snapshot completes. Its distinct VM identity must not be overwritten.
    let existing = test_persistent_entry("raced-fork", state.run_dir.join("persistent/other-source"));
    let existing_id = existing.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(existing.name.clone(), existing);
    gate.release.notify_one();
    let error = request.await.unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT, "{}", error.body.error);
    assert_eq!(
        state.persistent_registry.lock().unwrap().get("raced-fork").unwrap().id,
        existing_id
    );
    assert_eq!(std::fs::read_dir(state.run_dir.join("persistent")).unwrap().count(), 0);
    assert_eq!(owner.await.unwrap().len(), 2);
}
