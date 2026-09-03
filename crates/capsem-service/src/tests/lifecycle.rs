use super::*;

#[test]
fn tempdir_test_states_use_distinct_session_index_databases() {
    let (first, first_dir) = make_test_state_with_tempdir();
    let (second, second_dir) = make_test_state_with_tempdir();

    let first_db = first.main_db_path();
    let second_db = second.main_db_path();

    assert!(
        first_db.starts_with(first_dir.path()),
        "first test database escaped its tempdir: {}",
        first_db.display()
    );
    assert!(
        second_db.starts_with(second_dir.path()),
        "second test database escaped its tempdir: {}",
        second_db.display()
    );
    assert_ne!(
        first_db, second_db,
        "independent test states must not share a session index database"
    );

    let first_owned = make_test_state();
    let second_owned = make_test_state();
    assert_ne!(
        first_owned.main_db_path(),
        second_owned.main_db_path(),
        "test states that own their tempdirs must not share a session index database"
    );
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    // Create a real session dir for the fake instance
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            id: "fork-src".into(),
            name: "fork-src".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/fork-src.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
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
    drop(registry);
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
    let (state, _dir) = make_test_state_with_tempdir();
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
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/dup-src.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
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
                name: "pers-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                session_dir: session_dir.clone(),
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
async fn handle_persist_preserves_profile_identity() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("sessions/persist-src");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.instances.lock().unwrap().insert(
        "persist-src".into(),
        InstanceInfo {
            id: "persist-src".into(),
            name: "persist-src".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/persist-src.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let _ = handle_persist(
        State(state.clone()),
        Path("persist-src".into()),
        Json(PersistRequest {
            name: "persisted".into(),
        }),
    )
    .await
    .unwrap();

    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("persisted").unwrap();
    assert_eq!(entry.id, "persist-src");
    assert_eq!(entry.name, "persisted");
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
    drop(registry);

    let instances = state.instances.lock().unwrap();
    let info = instances.get("persist-src").unwrap();
    assert_eq!(info.id, "persist-src");
    assert_eq!(info.profile_id, "code");
    assert_eq!(info.profile_revision, test_profile_revision());
    assert_eq!(info.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(info.asset_pins, test_asset_pins());
    assert!(info.persistent);
    drop(instances);
}

#[test]
fn resume_rejects_profile_revision_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/revision-drift");
    std::fs::create_dir_all(&session_dir).unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "revision-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "revision-drift".into(),
                profile_id: "code".into(),
                profile_revision: "old-revision".into(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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

    let err = state.resume_sandbox(&vm_id, None, None).unwrap_err();
    assert!(
        err.to_string().contains("revision mismatch"),
        "resume must fail closed on profile revision drift, got: {err}"
    );
}

#[test]
fn persistent_resume_uses_saved_profile_when_current_profile_revision_advances() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/older-profile");
    let runtime_profile = state.profile_for_runtime("code").unwrap();
    let active_profile_path = state
        .materialize_active_profile(&runtime_profile, &session_dir)
        .unwrap();
    let mut active_profile: ActiveProfileFile =
        toml::from_str(&std::fs::read_to_string(&active_profile_path).unwrap()).unwrap();
    active_profile.revision = "older-supported-revision".to_string();
    std::fs::write(&active_profile_path, toml::to_string_pretty(&active_profile).unwrap()).unwrap();
    let rootfs = capsem_core::guest_share_dir(&session_dir).join("system/rootfs.img");
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(u64::from(runtime_profile.config().vm.scratch_disk_size_gb) * 1024 * 1024 * 1024)
        .unwrap();

    let mut entry = test_persistent_entry("older-profile", session_dir);
    entry.profile_revision = active_profile.revision;

    assert_eq!(
        state.persistent_entry_resume_state_cached(&entry),
        (VmLifecycleState::Stopped, true, None),
        "a newer current profile must not invalidate a verified persistent VM pin"
    );
}

#[test]
fn persistent_resume_rejects_a_corrupt_saved_active_profile() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/corrupt-saved-profile");
    let active_profile = session_dir.join(ACTIVE_PROFILE_DIR).join(ACTIVE_PROFILE_FILE);
    std::fs::create_dir_all(active_profile.parent().unwrap()).unwrap();
    std::fs::write(&active_profile, "not = [valid toml").unwrap();
    let entry = test_persistent_entry("corrupt-saved-profile", session_dir);

    let (status, can_resume, reason) = state.persistent_entry_resume_state_cached(&entry);
    assert_eq!(status, VmLifecycleState::Incompatible);
    assert!(!can_resume);
    assert!(
        reason.unwrap().contains("parse saved active profile"),
        "corrupt saved policy must fail closed"
    );
}

#[test]
fn persistent_resume_allows_deprecated_pins_but_blocks_explicit_revocation() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/deprecated-pin");
    let runtime_profile = state.profile_for_runtime("code").unwrap();
    state
        .materialize_active_profile(&runtime_profile, &session_dir)
        .unwrap();
    let rootfs = capsem_core::guest_share_dir(&session_dir).join("system/rootfs.img");
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(u64::from(runtime_profile.config().vm.scratch_disk_size_gb) * 1024 * 1024 * 1024)
        .unwrap();
    let entry = test_persistent_entry("deprecated-pin", session_dir);
    let hash = entry.asset_pins.rootfs.hash.strip_prefix("blake3:").unwrap();
    let manifest_path = state.assets_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "profiles": {
                "code": {
                    "status": "supported",
                    "architectures": [{
                        "images": [{
                            "kind": "rootfs",
                            "status": "deprecated",
                            "digest": {"blake3": hash}
                        }]
                    }]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        state.persistent_entry_resume_state_cached(&entry),
        (VmLifecycleState::Stopped, true, None),
        "deprecation blocks new selection, not an existing VM pin"
    );

    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["profiles"]["code"]["architectures"][0]["images"][0]["status"] = serde_json::json!("revoked");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let (status, can_resume, reason) = state.persistent_entry_resume_state_cached(&entry);
    assert_eq!(status, VmLifecycleState::Incompatible);
    assert!(!can_resume);
    assert!(reason.unwrap().contains("explicitly revoked image"));
}

#[test]
fn resume_rejects_profile_payload_hash_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/payload-hash-drift");
    std::fs::create_dir_all(&session_dir).unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-hash-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "payload-hash-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000".into(),
                asset_pins: test_asset_pins(),
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

    let err = state.resume_sandbox(&vm_id, None, None).unwrap_err();
    assert!(
        err.to_string().contains("payload hash mismatch"),
        "resume must fail closed on profile payload hash drift, got: {err}"
    );
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

#[test]
fn provision_rejects_nonexistent_source_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        name: "vm1",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("ghost-sandbox".into()),
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found"), "expected sandbox not found, got: {err}");
}

#[test]
fn provision_rejects_source_with_different_profile() {
    let (state, _dir) = make_test_state_with_tempdir();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "other-profile-source".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "other-profile-source".into(),
                profile_id: "other-profile".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/other-profile-source"),
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
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        name: "vm1",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("other-profile-source".into()),
        description: None,
    });
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("uses profile 'other-profile', not 'code'"),
        "source profile mismatch must fail, got: {err}"
    );
}

// Suspend/resume registry fixes (issues #4-8)

#[tokio::test]
async fn handle_list_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let suspended_dir = state.run_dir.join("persistent/susp-vm");
    let stopped_dir = state.run_dir.join("persistent/stop-vm");
    capsem_core::create_virtiofs_session(&suspended_dir, 64).unwrap();
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();

    // Register a suspended persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "susp-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "susp-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: suspended_dir,
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    // Register a stopped (not suspended) persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "stop-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "stop-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 1024,
                cpus: 1,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: stopped_dir,
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

    let list: ListResponse = decode_response_json(handle_list(State(state)).await).await;

    let susp = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("susp-vm"))
        .unwrap();
    assert_ne!(susp.id, "susp-vm");
    assert_eq!(
        susp.status,
        VmLifecycleState::Suspended,
        "suspended VM should show Suspended status"
    );

    let stop = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("stop-vm"))
        .unwrap();
    assert_ne!(stop.id, "stop-vm");
    assert_eq!(
        stop.status,
        VmLifecycleState::Stopped,
        "non-suspended VM should show Stopped status"
    );
}

#[tokio::test]
async fn handle_info_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/info-susp");
    capsem_core::create_virtiofs_session(&session_dir, 64).unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "info-susp".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "info-susp".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let result = handle_info(State(state), Path(vm_id)).await;
    let Json(info) = result.unwrap();
    assert_eq!(info.status, VmLifecycleState::Suspended);
}

#[tokio::test]
async fn handle_info_reports_storage_diagnostics_for_persistent_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-info");
    std::fs::create_dir_all(session_dir.join("guest/system")).unwrap();
    let rootfs = session_dir.join("guest/system/rootfs.img");
    let file = std::fs::File::create(&rootfs).unwrap();
    file.set_len(8 * 1024 * 1024 * 1024).unwrap();

    let entry = test_persistent_entry("storage-info", session_dir.clone());
    let vm_id = entry.id.clone();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert("storage-info".into(), entry);
    }

    let Json(info) = handle_info(State(state), Path(vm_id)).await.unwrap();
    let storage = info.storage.expect("info must include storage diagnostics");
    assert_eq!(storage.rootfs_image_path, rootfs.to_string_lossy().to_string());
    assert_eq!(storage.rootfs_image_logical_bytes, 8 * 1024 * 1024 * 1024);
    assert!(
        storage.rootfs_image_physical_bytes < storage.rootfs_image_logical_bytes,
        "sparse rootfs image should report allocated blocks separately from logical size"
    );
    assert!(storage.host_available_bytes > 0);
    assert_eq!(storage.guest_overlay_device, "/dev/vdb");
    assert_eq!(storage.guest_overlay_mount, "/");
}

#[tokio::test]
async fn handle_vm_status_reports_storage_diagnostics_for_persistent_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-status");
    capsem_core::create_virtiofs_session(&session_dir, 4).unwrap();
    let rootfs = session_dir.join("guest/system/rootfs.img");

    let entry = test_persistent_entry("storage-status", session_dir);
    let vm_id = entry.id.clone();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert("storage-status".into(), entry);
    }

    let Json(status) = handle_vm_status(State(state), Path(vm_id)).await.unwrap();
    let storage = status.storage.expect("status must include storage diagnostics");
    assert_eq!(storage.rootfs_image_path, rootfs.to_string_lossy().to_string());
    assert_eq!(storage.rootfs_image_logical_bytes, 4 * 1024 * 1024 * 1024);
    assert!(storage.host_free_bytes > 0);
    assert_eq!(storage.guest_overlay_device, "/dev/vdb");
    assert_eq!(storage.guest_overlay_mount, "/");
}

#[tokio::test]
async fn handle_list_marks_profile_payload_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-drift".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "payload-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000".into(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/payload-drift"),
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

    let list: ListResponse = decode_response_json(handle_list(State(state)).await).await;
    let vm = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("payload-drift"))
        .unwrap();
    assert_ne!(vm.id, "payload-drift");
    assert_eq!(vm.status, VmLifecycleState::Incompatible);
    assert!(!vm.can_resume);
    assert!(vm
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("payload hash mismatch"));
}

#[tokio::test]
async fn handle_info_marks_profile_payload_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-drift-info".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "payload-drift-info".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000".into(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/payload-drift-info"),
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

    let Json(info) = handle_info(State(state), Path(vm_id)).await.unwrap();
    assert_eq!(info.status, VmLifecycleState::Incompatible);
    assert!(!info.can_resume);
    assert!(info
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("payload hash mismatch"));
}

#[tokio::test]
async fn handle_list_marks_profile_rootfs_size_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/rootfs-size-drift");
    capsem_core::create_virtiofs_session(&session_dir, 2).unwrap();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "rootfs-size-drift".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "rootfs-size-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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

    let list: ListResponse = decode_response_json(handle_list(State(state.clone())).await).await;
    let vm = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("rootfs-size-drift"))
        .unwrap();
    assert_ne!(vm.id, "rootfs-size-drift");
    assert_eq!(vm.status, VmLifecycleState::Incompatible);
    assert!(!vm.can_resume);
    let reason = vm.resume_blocked_reason.as_deref().unwrap_or_default();
    assert!(reason.contains("rootfs.img logical size mismatch"), "{reason}");
    assert!(reason.contains("2 GiB"), "{reason}");
    assert!(reason.contains("64 GiB"), "{reason}");
    assert_eq!(
        vm.available_actions,
        VmLifecycleState::Incompatible.available_actions(false)
    );

    let Json(info) = handle_info(State(state.clone()), Path(vm.id.clone())).await.unwrap();
    assert_eq!(info.status, VmLifecycleState::Incompatible);
    assert!(!info.can_resume);
    assert!(info
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("rootfs.img logical size mismatch"));

    let Json(status) = handle_vm_status(State(state), Path(vm.id.clone())).await.unwrap();
    assert_eq!(status.status, VmLifecycleState::Incompatible);
    assert!(!status.can_resume);
    assert!(status
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("rootfs.img logical size mismatch"));
}

#[tokio::test]
async fn handle_vm_operation_status_reports_idle_for_existing_vm() {
    let state = make_test_state();
    insert_fake_instance(&state, "ops-vm", 5150);

    let Json(save) = handle_vm_save_status(State(Arc::clone(&state)), Path("ops-vm".into()))
        .await
        .unwrap();
    assert_eq!(save.vm_id, "ops-vm");
    assert_eq!(save.operation, "save");
    assert_eq!(save.status, "idle");
    assert!(!save.in_progress);

    let Json(fork) = handle_vm_fork_status(State(state), Path("ops-vm".into()))
        .await
        .unwrap();
    assert_eq!(fork.operation, "fork");
    assert_eq!(fork.status, "idle");
    assert!(!fork.in_progress);
}

#[tokio::test]
async fn handle_vm_operation_status_rejects_unknown_vm() {
    let state = make_test_state();

    let err = handle_vm_save_status(State(state), Path("missing-vm".into()))
        .await
        .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_suspend_rejects_ephemeral_vm() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Insert an ephemeral VM in instances
    {
        let mut instances = state.instances.lock().unwrap();
        instances.insert(
            "eph-vm".into(),
            InstanceInfo {
                id: "eph-vm".into(),
                name: "eph-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                pid: 0,
                uds_path: state.run_dir.join("instances/eph-vm.sock"),
                session_dir: state.run_dir.join("sessions/eph-vm"),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                forked_from: None,
            },
        );
    }

    let result = handle_suspend(State(state), Path("eph-vm".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("ephemeral"));
}

#[tokio::test]
async fn handle_suspend_returns_not_found_for_missing_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = handle_suspend(State(state), Path("nonexistent".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[test]
fn archive_failed_restore_checkpoint_moves_checkpoint_aside() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let checkpoint = session_dir.join("checkpoint.vzsave");
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(&checkpoint, b"bad checkpoint").unwrap();
    std::fs::write(&complete, b"ok\n").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: session_dir.clone(),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let archived = state
        .archive_failed_restore_checkpoint(&vm_id)
        .expect("checkpoint should be archived");

    assert!(!checkpoint.exists(), "original checkpoint must be moved");
    assert!(!complete.exists(), "completion marker must be moved");
    assert!(
        archived.exists(),
        "archived checkpoint should exist: {}",
        archived.display()
    );
    let archived_complete = session_dir.join(format!("{}.complete", archived.file_name().unwrap().to_string_lossy()));
    assert!(
        archived_complete.exists(),
        "archived completion marker should exist: {}",
        archived_complete.display()
    );
    assert!(archived
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("checkpoint.vzsave.failed-restore-"));
}

#[tokio::test]
async fn failed_restore_teardown_clears_running_instance_before_cold_fallback() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = new_persistent_vm_id();
    let session_dir = state.run_dir.join("persistent").join(&vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let uds_path = state.run_dir.join("instances").join(format!("{vm_id}.sock"));
    std::fs::create_dir_all(uds_path.parent().unwrap()).unwrap();
    std::fs::write(&uds_path, b"stale socket").unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"stale ready").unwrap();

    state.instances.lock().unwrap().insert(
        vm_id.clone(),
        InstanceInfo {
            id: vm_id.clone(),
            name: "resume-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 0,
            uds_path: uds_path.clone(),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: true,
            env: None,
            forked_from: None,
        },
    );

    stop_failed_restore_process_under_lock(&state, &vm_id).await;

    assert!(
        !state.instances.lock().unwrap().contains_key(&vm_id),
        "failed warm restore must not remain registered before cold fallback"
    );
    assert!(!uds_path.exists(), "stale UDS socket should be removed");
    assert!(
        !uds_path.with_extension("ready").exists(),
        "stale ready sentinel should be removed"
    );
}

#[test]
fn existing_resume_checkpoint_requires_completion_marker() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let checkpoint = session_dir.join("checkpoint.vzsave");
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(&checkpoint, b"partial checkpoint").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    assert!(
        !state.has_existing_resume_checkpoint(&vm_id),
        "bare checkpoint without completion marker must not be resumable"
    );

    std::fs::write(&complete, b"ok\n").unwrap();
    assert!(
        state.has_existing_resume_checkpoint(&vm_id),
        "checkpoint with completion marker should be resumable"
    );
}

#[test]
fn clear_resume_checkpoint_removes_completion_marker() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(session_dir.join("checkpoint.vzsave"), b"checkpoint").unwrap();
    std::fs::write(&complete, b"ok\n").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    state.clear_resume_checkpoint(&vm_id);
    assert!(
        !complete.exists(),
        "completion marker must be removed once checkpoint state is cleared"
    );
    let reg = state.persistent_registry.lock().unwrap();
    let entry = reg.get("resume-vm").unwrap();
    assert!(!entry.suspended);
    assert!(entry.checkpoint_path.is_none());
    drop(reg);
}

// main_db_path

#[test]
fn main_db_path_resolves_to_sessions_dir() {
    let state = make_test_state();
    // run_dir = /tmp/capsem-test-svc => parent = /tmp => main.db = /tmp/sessions/main.db
    let path = state.main_db_path();
    assert!(path.ends_with("sessions/main.db"), "got: {}", path.display());
}

#[test]
fn profile_mutation_db_startup_initializes_session_index_schema() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();

    let handle = ServiceState::open_profile_mutation_db_handle(&run_dir).unwrap();
    drop(handle);

    let db_path = main_db_path_for_run_dir(&run_dir);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(session_count, 0);

    let mutation_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM profile_mutation_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mutation_count, 0);
}

#[test]
fn session_index_start_records_uuid_id_not_display_name() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let id = new_persistent_vm_id();
    uuid::Uuid::parse_str(&id).expect("VM route id should be a UUID");
    let display_name = "code-1";

    state
        .record_session_index_start(&id, false, 16, 2048, Some("blake3:abc"), Some("1.3.1782496403"), None)
        .unwrap();

    let conn = rusqlite::Connection::open(state.main_db_path()).unwrap();
    let by_id: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE id = ?1", [&id], |row| row.get(0))
        .unwrap();
    let by_name: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE id = ?1", [display_name], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(by_id, 1);
    assert_eq!(by_name, 0);
}

// -----------------------------------------------------------------------
// SandboxInfo::new
// -----------------------------------------------------------------------

#[test]
fn sandbox_info_new_defaults_telemetry_to_none() {
    let info = SandboxInfo::new("test".into(), "code".into(), 1, VmLifecycleState::Running, false);
    assert_eq!(info.id, "test");
    assert_eq!(info.pid, 1);
    assert!(!info.persistent);
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
}

#[tokio::test]
async fn vm_list_and_info_are_in_memory_only() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/list-hot-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let file_event = capsem_logger::FileEvent {
        event_id: Some("abcdef123456".into()),
        timestamp: std::time::SystemTime::now(),
        action: capsem_logger::FileAction::Created,
        path: "/root/list-hot-proof.txt".into(),
        size: Some(12),
        trace_id: Some("tracelisthot".into()),
        credential_ref: None,
    };
    let db_path = session_dir.join("session.db");
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::FileEvent(file_event));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, "list-hot-vm", 4242, session_dir);

    let list: ListResponse = decode_response_json(handle_list(State(Arc::clone(&state))).await).await;
    let listed = list
        .sandboxes
        .iter()
        .find(|vm| vm.id == "list-hot-vm")
        .expect("running VM listed");
    assert!(
        listed.total_input_tokens.is_none(),
        "/vms/list is a hot route and must not read session.db telemetry"
    );
    assert!(listed.model_call_count.is_none());

    let Json(info) = handle_info(State(state), Path("list-hot-vm".into()))
        .await
        .expect("detail route stays lifecycle/storage only");
    assert!(
        info.total_file_events.is_none(),
        "/vms/{{id}}/info must not inline raw telemetry SQL; use ledger DB APIs"
    );
    assert!(info.model_call_count.is_none());
}

#[test]
fn vm_lifecycle_available_actions_are_contractual() {
    use api::VmAction;

    assert_eq!(
        VmLifecycleState::Running.available_actions(false),
        vec![VmAction::Pause, VmAction::Stop, VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Stopped.available_actions(true),
        vec![VmAction::Start, VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Stopped.available_actions(false),
        vec![VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Suspended.available_actions(true),
        vec![VmAction::Resume, VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Suspended.available_actions(false),
        vec![VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Defunct.available_actions(false),
        vec![VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Incompatible.available_actions(false),
        vec![VmAction::Delete]
    );
}

#[test]
fn sandbox_info_telemetry_fields_serialize_when_present() {
    let mut info = SandboxInfo::new("test".into(), "code".into(), 1, VmLifecycleState::Running, false);
    info.total_input_tokens = Some(1000);
    info.total_estimated_cost = Some(0.42);
    info.model_call_count = Some(5);
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"total_input_tokens\":1000"));
    assert!(json.contains("\"total_estimated_cost\":0.42"));
    assert!(json.contains("\"model_call_count\":5"));
}

#[test]
fn sandbox_info_telemetry_fields_omitted_when_none() {
    let info = SandboxInfo::new("test".into(), "code".into(), 1, VmLifecycleState::Running, false);
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("total_input_tokens"));
    assert!(!json.contains("total_estimated_cost"));
    assert!(!json.contains("model_call_count"));
    assert!(!json.contains("uptime_secs"));
}

#[test]
fn sandbox_info_rejects_missing_profile_id() {
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
    let err = serde_json::from_str::<SandboxInfo>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn profile_vm_resources_drive_new_session_defaults() {
    let profile = ProfileConfigFile::builtin_primary();

    let default_resources = resolve_profile_vm_resources(&profile, None, None);
    assert_eq!(default_resources.cpus, profile.vm.cpu_count);
    assert_eq!(default_resources.ram_mb, u64::from(profile.vm.ram_gb) * 1024);
    assert_eq!(default_resources.scratch_disk_size_gb, profile.vm.scratch_disk_size_gb);

    let customized_resources = resolve_profile_vm_resources(&profile, Some(3072), Some(2));
    assert_eq!(customized_resources.cpus, 2);
    assert_eq!(customized_resources.ram_mb, 3072);
    assert_eq!(
        customized_resources.scratch_disk_size_gb, profile.vm.scratch_disk_size_gb,
        "scratch image size is profile-owned and must not fall back to hidden service defaults"
    );
}

// -----------------------------------------------------------------------
// StatsResponse
// -----------------------------------------------------------------------

#[test]
fn stats_response_serializes() {
    let resp = StatsResponse {
        global: capsem_core::session::GlobalStats {
            total_sessions: 10,
            total_input_tokens: 5000,
            total_output_tokens: 2000,
            total_estimated_cost: 1.50,
            total_tool_calls: 100,
            total_file_events: 300,
            total_requests: 400,
            total_allowed: 380,
            total_denied: 20,
        },
        sessions: vec![],
        top_providers: vec![],
        top_tools: vec![],
        top_mcp_tools: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"total_sessions\":10"));
    assert!(json.contains("\"total_estimated_cost\":1.5"));
    assert!(json.contains("\"top_providers\":[]"));
}

// -----------------------------------------------------------------------
// handle_list includes uptime_secs for running VMs
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_includes_uptime_for_running_vms() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", 100);
    let list: ListResponse = decode_response_json(handle_list(State(state)).await).await;
    assert_eq!(list.sandboxes.len(), 1);
    assert!(list.sandboxes[0].uptime_secs.is_some());
}

// -----------------------------------------------------------------------
// handle_stats with tempdir
// -----------------------------------------------------------------------

#[tokio::test]
async fn db_boundary_route_contract_handle_stats_returns_global_data() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    // Create main.db with a test session
    let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
    let record = capsem_core::session::SessionRecord {
        id: "20260412-120000-abcd".into(),
        mode: "virtiofs".into(),
        command: Some("echo hello".into()),
        status: "stopped".into(),
        created_at: "2026-04-12T12:00:00Z".into(),
        stopped_at: Some("2026-04-12T12:05:00Z".into()),
        scratch_disk_size_gb: 16,
        ram_bytes: 4294967296,
        total_requests: 50,
        allowed_requests: 45,
        denied_requests: 5,
        total_input_tokens: 10000,
        total_output_tokens: 3000,
        total_estimated_cost: 0.42,
        total_tool_calls: 25,
        total_file_events: 100,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: "virtiofs".into(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    };
    idx.create_session(&record).unwrap();
    drop(idx);

    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let result = handle_stats(State(state)).await;
    if let Err(error) = &result {
        panic!("stats route must read seeded main.db rows through the logger DB handle: {error:?}");
    }
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["global"]["total_sessions"], 1);
    assert_eq!(resp["global"]["total_input_tokens"], 10000);
    assert_eq!(resp["global"]["total_estimated_cost"], 0.42);
    assert_eq!(resp["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(resp["sessions"][0]["id"], "20260412-120000-abcd");
}

#[tokio::test]
async fn stats_detail_route_reads_session_db_ledger() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("stats-detail-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "stats-detail-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some("abc123abc123".to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: "google".to_string(),
        protocol: Some("google".to_string()),
        model: Some("gemini-3.5-flash".to_string()),
        process_name: Some("agy".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1internal:streamGenerateContent".to_string(),
        stream: true,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 1,
        request_bytes: 32,
        request_body_preview: Some(r#"{"contents":[{"text":"write"}]}"#.to_string()),
        request_body_full: Some(r#"{"contents":[{"text":"write full bounded body"}]}"#.to_string()),
        message_id: Some("msg-1".to_string()),
        status_code: Some(200),
        text_content: Some("created poem.md".to_string()),
        thinking_content: Some("plan file write".to_string()),
        response_body_full: Some(r#"{"candidates":[{"content":{"parts":[{"text":"created poem.md"}]}}]}"#.to_string()),
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(12),
        output_tokens: Some(7),
        usage_details: BTreeMap::from([("thinking".to_string(), 5)]),
        duration_ms: 25,
        response_bytes: 64,
        estimated_cost_usd: 0.001,
        trace_id: Some("trace-stats-detail".to_string()),
        credential_ref: None,
        tool_calls: vec![capsem_logger::ToolCallEntry {
            call_index: 0,
            call_id: "tool-1".to_string(),
            tool_name: "Create".to_string(),
            arguments: Some(r#"{"path":"/root/poem.md"}"#.to_string()),
            origin: "native".to_string(),
            trace_id: Some("trace-stats-detail".to_string()),
        }],
        tool_responses: vec![capsem_logger::ToolResponseEntry {
            call_id: "tool-1".to_string(),
            content_preview: Some("Wrote 4 lines to poem.md".to_string()),
            is_error: false,
            trace_id: Some("trace-stats-detail".to_string()),
            credential_ref: None,
        }],
    }));
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("def456def456".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "generativelanguage.googleapis.com".to_string(),
            port: 443,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("agy".to_string()),
            pid: Some(42),
            method: Some("POST".to_string()),
            path: Some("/v1internal:streamGenerateContent".to_string()),
            query: None,
            status_code: Some(200),
            bytes_sent: 32,
            bytes_received: 64,
            duration_ms: 21,
            matched_rule: Some("profiles.rules.ai_google_http_googleapis".to_string()),
            request_headers: Some("content-type: application/json".to_string()),
            response_headers: Some("content-type: application/json".to_string()),
            request_body_preview: Some(r#"{"model":"gemini-3.5-flash"}"#.to_string()),
            response_body_preview: Some(r#"{"ok":true}"#.to_string()),
            request_body_full: Some(
                r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#.to_string(),
            ),
            response_body_full: Some(r#"{"ok":true,"body":"full response body from gateway"}"#.to_string()),
            conn_type: Some("https".to_string()),
            policy_mode: None,
            policy_action: Some("allow".to_string()),
            policy_rule: Some("profiles.rules.ai_google_http_googleapis".to_string()),
            policy_reason: None,
            trace_id: Some("trace-stats-detail".to_string()),
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/stats-detail-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["model_stats"][0]["provider"], "google");
    assert_eq!(body["model_stats"][0]["model"], "gemini-3.5-flash");
    assert_eq!(body["model_stats"][0]["call_count"], 1);
    assert_eq!(body["model_stats"][0]["input_tokens"], 12);
    assert_eq!(body["model_stats"][0]["output_tokens"], 7);
    assert_eq!(body["model_events"][0]["event_id"], "abc123abc123");
    assert_eq!(body["model_events"][0]["input_tokens"], 12);
    assert_eq!(
        body["model_events"].as_array().unwrap().len(),
        body["model_stats"][0]["call_count"].as_u64().unwrap() as usize,
        "model_stats.call_count must agree with model_events"
    );
    assert!(body["model_events"][0].get("request_body_preview").is_none());
    assert!(body["model_events"][0].get("response_body_preview").is_none());
    assert_eq!(body["tool_events"][0]["tool_name"], "Create");
    assert_eq!(body["tool_events"][0]["call_id"], "tool-1");
    assert_eq!(body["tool_events"][0]["source"], "native");
    assert_eq!(body["tool_events"][0]["model_parent_missing"], 0);
    assert!(body["tool_events"][0]["model_call_id"].as_i64().is_some());
    assert_eq!(body["tool_events"][0]["arguments"], r#"{"path":"/root/poem.md"}"#);
    assert_eq!(body["tool_events"][0]["response_preview"], "Wrote 4 lines to poem.md");
    assert_eq!(body["http_events"][0]["event_id"], "def456def456");
    assert_eq!(body["http_events"][0]["domain"], "generativelanguage.googleapis.com");
    assert!(body["http_events"][0].get("request_body_preview").is_none());
    assert!(body["http_events"][0].get("response_body_preview").is_none());
    assert_eq!(body["body_blobs"]["abc123abc123"][0]["direction"], "request");
    assert_eq!(
        body["body_blobs"]["abc123abc123"][0]["body"],
        r#"{"contents":[{"text":"write full bounded body"}]}"#
    );
    assert_eq!(body["body_blobs"]["abc123abc123"][1]["direction"], "response");
    assert_eq!(
        body["body_blobs"]["abc123abc123"][1]["body"],
        r#"{"candidates":[{"content":{"parts":[{"text":"created poem.md"}]}}]}"#
    );
    assert_eq!(body["body_blobs"]["def456def456"][0]["direction"], "request");
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["body"],
        r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["stored_bytes"],
        r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#.len()
    );
    assert_eq!(body["body_blobs"]["def456def456"][1]["direction"], "response");
    assert_eq!(
        body["body_blobs"]["def456def456"][1]["body"],
        r#"{"ok":true,"body":"full response body from gateway"}"#
    );

    let (status, summary) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/stats-detail-vm/stats/summary",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(
        summary,
        serde_json::json!({
            "total_requests": 1,
            "allowed_requests": 1,
            "denied_requests": 0,
            "total_input_tokens": 12,
            "total_thinking_tokens": 5,
            "total_output_tokens": 7,
            "total_tool_calls": 1,
            "total_estimated_cost": 0.001,
        }),
        "toolbar summary must remain compact and agree with the session ledger"
    );

    let (status, info) = route_request(app, axum::http::Method::GET, "/vms/stats-detail-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        info.get("model_call_count"),
        None,
        "/vms/{{id}}/info stays lifecycle/storage only; stats/detail is the ledger surface"
    );
    assert_eq!(info.get("total_input_tokens"), None);
    assert_eq!(info.get("total_output_tokens"), None);
    assert_eq!(info.get("total_tool_calls"), None);
}

async fn write_test_model_call(db_path: &std::path::Path, provider: &str, model: &str, event_id: &str) {
    let writer = capsem_logger::DbWriter::open(db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some(event_id.to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: provider.to_string(),
        protocol: Some(provider.to_string()),
        model: Some(model.to_string()),
        process_name: Some("agy".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1internal:streamGenerateContent".to_string(),
        stream: true,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 0,
        request_bytes: 32,
        request_body_preview: None,
        request_body_full: None,
        message_id: Some(format!("{event_id}-message")),
        status_code: Some(200),
        text_content: Some("ok".to_string()),
        thinking_content: None,
        response_body_full: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(12),
        output_tokens: Some(7),
        usage_details: BTreeMap::new(),
        duration_ms: 25,
        response_bytes: 64,
        estimated_cost_usd: 0.001,
        trace_id: Some(format!("trace-{event_id}")),
        credential_ref: None,
        tool_calls: vec![],
        tool_responses: vec![],
    }));
    writer.shutdown_blocking();
}

#[tokio::test]
async fn stats_detail_route_reopens_session_db_handle_when_vm_id_rebinds_to_new_path() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let old_session_dir = dir.path().join("sessions").join("co-work1-old");
    let selected_session_dir = dir.path().join("sessions").join("co-work1-selected");
    std::fs::create_dir_all(&old_session_dir).unwrap();
    std::fs::create_dir_all(&selected_session_dir).unwrap();

    write_test_model_call(
        &old_session_dir.join("session.db"),
        "ollama",
        "llama3.2",
        "badbadbadbad",
    )
    .await;
    write_test_model_call(
        &selected_session_dir.join("session.db"),
        "google",
        "gemini-3.5-flash",
        "abcabcabcabc",
    )
    .await;
    let conn = rusqlite::Connection::open(selected_session_dir.join("session.db")).unwrap();
    let direct_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_calls", [], |row| row.get(0))
        .unwrap();
    assert_eq!(direct_count, 1, "selected DB fixture must contain one model call");

    state
        .register_session_db_handle("co-work1", &old_session_dir)
        .expect("test installs stale cached DB handle");
    insert_fake_instance_with_session_dir(&state, "co-work1", std::process::id(), selected_session_dir.clone());

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/co-work1/stats/detail", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["model_stats"][0]["provider"], "google",
        "stats/detail must use the DB resolved for the selected session id, not a stale cached handle: {body}"
    );
    assert_eq!(body["model_stats"][0]["model"], "gemini-3.5-flash");
    assert_eq!(body["model_events"][0]["event_id"], "abcabcabcabc");
    assert_eq!(
        state.session_db_handle("co-work1").unwrap().path(),
        selected_session_dir.join("session.db").as_path(),
        "the stale cached handle must be replaced with the selected session DB"
    );
}

#[tokio::test]
async fn persistent_session_routes_keep_uuid_id_separate_from_display_name() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = "11111111-1111-4111-8111-111111111111";
    let session_dir = state.run_dir.join("persistent").join(vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut entry = test_persistent_entry("co-work1", session_dir.clone());
    entry.id = vm_id.to_string();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("co-work1".to_string(), entry);

    let listing: ListResponse = decode_response_json(handle_list(State(Arc::clone(&state))).await).await;
    let row = listing
        .sandboxes
        .iter()
        .find(|row| row.name.as_deref() == Some("co-work1"))
        .expect("persistent session appears by display name");
    assert_eq!(row.id, vm_id);
    assert_eq!(row.name.as_deref(), Some("co-work1"));

    let info = handle_info(State(Arc::clone(&state)), Path(vm_id.to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(info.id, vm_id);
    assert_eq!(info.name.as_deref(), Some("co-work1"));

    let status = handle_vm_status(State(state), Path(vm_id.to_string())).await.unwrap().0;
    assert_eq!(status.id, vm_id);
}

#[test]
fn resume_sandbox_requires_uuid_route_id_not_display_name() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = "22222222-2222-4222-8222-222222222222";
    let session_dir = state.run_dir.join("persistent").join(vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut entry = test_persistent_entry("co-work1", session_dir);
    entry.id = vm_id.to_string();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("co-work1".to_string(), entry);

    let err = state.resume_sandbox("co-work1", None, None).unwrap_err();
    assert!(
        err.to_string().contains("no persistent VM with id"),
        "display names must be translated before service routes call resume: {err}"
    );
}

#[tokio::test]
async fn resume_sandbox_passes_profile_scratch_disk_size_to_process() {
    let (mut state, _dir) = make_test_state_with_tempdir();
    let run_dir = state.run_dir.clone();
    let argv_path = run_dir.join("resume-argv.txt");
    let argv_tmp_path = run_dir.join("resume-argv.txt.tmp");
    let process_path = run_dir.join("record-process-argv.sh");
    std::fs::write(
        &process_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nmv '{}' '{}'\nsleep 1\n",
            argv_tmp_path.display(),
            argv_tmp_path.display(),
            argv_path.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&process_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&process_path, perms).unwrap();
    }
    Arc::get_mut(&mut state).unwrap().process_binary = process_path;
    install_test_profile_assets(&state);

    let vm_id = new_persistent_vm_id();
    let session_dir = state.run_dir.join("persistent").join(&vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let rootfs = capsem_core::guest_share_dir(&session_dir).join("system/rootfs.img");
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(u64::from(materialized_test_profile().vm.scratch_disk_size_gb) * 1024 * 1024 * 1024)
        .unwrap();
    let mut entry = test_persistent_entry("resume-size", session_dir);
    entry.id = vm_id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("resume-size".to_string(), entry);

    assert_eq!(state.resume_sandbox(&vm_id, None, None).unwrap(), vm_id);
    for _ in 0..50 {
        if argv_path.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let argv = std::fs::read_to_string(&argv_path).expect("resume process argv should be recorded");
    let args: Vec<&str> = argv.lines().collect();
    let size_flag = args
        .windows(2)
        .find(|window| window[0] == "--scratch-disk-size-gb")
        .map(|window| window[1]);
    let expected_size = materialized_test_profile().vm.scratch_disk_size_gb.to_string();
    assert_eq!(
        size_flag,
        Some(expected_size.as_str()),
        "resume must preserve the profile-owned system overlay size; argv={args:?}"
    );
}

#[tokio::test]
async fn db_boundary_route_contract_db_handle_route_rewire() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-handle-route-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "db-handle-route-vm", std::process::id(), session_dir.clone());

    assert!(
        state.session_db_handle("db-handle-route-vm").is_none(),
        "session handles are registered lazily after capsem-process creates session.db"
    );
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_111_000_000,
                "abcdef123456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow),
        ))
        .await;
    writer.shutdown_blocking();

    let (status, stats_detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats_detail}");
    assert_eq!(stats_detail["model_stats"], json!([]));
    assert_eq!(stats_detail["body_blobs"], json!({}));

    let (status, security_status) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/security/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security_status}");
    assert_eq!(security_status["total"], 1);
    assert_eq!(security_status["by_action"][0]["rule_action"], "allow");
    assert!(
        state.session_db_handle("db-handle-route-vm").is_some(),
        "first ledger route registers the external DB reader once session.db exists"
    );
}

#[tokio::test]
async fn db_boundary_route_contract_stats_routes_do_not_return_empty_on_broken_schema() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("broken-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "broken-db-vm", std::process::id(), session_dir.clone());
    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("DROP TABLE net_events", []).unwrap();
    conn.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(conn);

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/broken-db-vm/stats/detail", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let body_text = body.to_string();
    assert!(
        body_text.contains("stats_detail ledger")
            && (body_text.contains("not ready")
                || body_text.contains("no such column")
                || body_text.contains("missing required column")),
        "broken schemas must fail loudly, not return empty fake data: {body}"
    );
}

#[test]
fn logged_data_routes_do_not_bypass_logger_db_boundary() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("service source must be readable");
    let forbidden = [
        "ready_blocking(",
        "query_raw_blocking(",
        "with_reader_blocking(",
        "DbReader::open(",
        "SessionIndex::open(",
        "SessionDb::new(",
        "read_stats_response_from_main_db(&state.main_db_path())",
        "_projection",
    ];
    for needle in forbidden {
        assert!(
            !source.contains(needle),
            "{needle} reintroduced a logged-data route bypass. See AGENTS.md, \
             skills/dev-testing/SKILL.md Logged-data DB ownership, and \
             skills/dev-rust-patterns/SKILL.md Logger DB boundary: routes own query intent, \
             capsem-logger owns DB execution/storage, and missing schemas fail loudly."
        );
    }
}

#[tokio::test]
async fn session_db_handle_state_contract() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-state-old");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "db-state-old", std::process::id(), session_dir.clone());
    state
        .register_session_db_handle("db-state-old", &session_dir)
        .expect("test installs external reader after session.db exists");

    let original = state
        .session_db_handle("db-state-old")
        .expect("session registration must install a DB handle");
    original.ready().await.unwrap();

    state.rename_session_db_handle("db-state-old", "db-state-new");
    assert!(
        state.session_db_handle("db-state-old").is_none(),
        "renaming a session must not leave a stale DB handle under the old id"
    );
    let renamed = state
        .session_db_handle("db-state-new")
        .expect("renaming a session must move its DB handle");
    assert!(
        Arc::ptr_eq(&original, &renamed),
        "renaming must move the existing DB handle instead of opening a second rail"
    );

    state.unregister_session_db_handle("db-state-new");
    assert!(
        state.session_db_handle("db-state-new").is_none(),
        "unregistering a session must remove the DB handle"
    );
}

#[test]
fn session_db_handle_registration_is_idempotent_for_same_session_path() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-state-idempotent");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();

    let first = state
        .register_session_db_handle("db-state-idempotent", &session_dir)
        .expect("first handle registration succeeds");
    let second = state
        .register_session_db_handle("db-state-idempotent", &session_dir)
        .expect("second registration for the same session path reuses the handle");

    assert!(
        Arc::ptr_eq(&first, &second),
        "route races must not create parallel external reader handles for the same session DB; \
         the UI polls stats and security ledgers concurrently, and multiple reader workers each \
         syncing hot tables from disk can surface SQLite table-lock errors"
    );
}

#[tokio::test]
async fn service_rehydrates_session_db_handles() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions").join("startup-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    // Routes address a persistent session by its runtime id, never by its
    // name: a handle hydrated under the name was invisible to every route.
    let entry = test_persistent_entry("startup-db-vm", session_dir);
    let vm_id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("startup-db-vm".to_string(), entry);

    assert!(
        state.session_db_handle(&vm_id).is_none(),
        "test must prove startup hydration installs the handle"
    );
    state.hydrate_session_db_handles();

    let handle = state
        .session_db_handle(&vm_id)
        .expect("startup hydration must install a persistent-session DB handle under the runtime id");
    handle
        .ready()
        .await
        .expect("hydrated handle must prove schema readiness");
}

#[tokio::test]
async fn status_reports_db_readiness() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "status-db-vm", std::process::id(), session_dir);

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/status-db-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_db"]["ready"], true,
        "session status must expose DB readiness from the service-owned DbHandle: {body}"
    );
    assert!(
        body["session_db"].get("error").is_none(),
        "ready session DB status must not invent an error: {body}"
    );
}

#[tokio::test]
async fn info_route_reports_db_readiness_without_inline_ledger_stats() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("toolbar-stats-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut usage_details = BTreeMap::new();
    usage_details.insert("thinking".to_string(), 3);
    usage_details.insert("reasoning".to_string(), 4);
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some("abc123abc123".to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: "openai".to_string(),
        protocol: Some("openai".to_string()),
        model: Some("gpt-5-demo".to_string()),
        process_name: Some("codex".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1/responses".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 1,
        request_bytes: 32,
        request_body_preview: None,
        request_body_full: None,
        message_id: Some("msg-toolbar".to_string()),
        status_code: Some(200),
        text_content: Some("done".to_string()),
        thinking_content: Some("checking stats".to_string()),
        response_body_full: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(12),
        output_tokens: Some(7),
        usage_details,
        duration_ms: 25,
        response_bytes: 64,
        estimated_cost_usd: 0.001,
        trace_id: Some("trace-toolbar".to_string()),
        credential_ref: None,
        tool_calls: vec![capsem_logger::ToolCallEntry {
            call_index: 0,
            call_id: "tool-toolbar".to_string(),
            tool_name: "Read".to_string(),
            arguments: Some(r#"{"path":"/root/demo.md"}"#.to_string()),
            origin: "model".to_string(),
            trace_id: Some("trace-toolbar".to_string()),
        }],
        tool_responses: vec![],
    }));
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "toolbar-stats-vm", std::process::id(), session_dir);

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/toolbar-stats-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["session_db"]["ready"], true);
    assert!(
        body.get("model_call_count").is_none(),
        "/vms/{{id}}/info must not inline ledger counters; use /vms/{{id}}/stats/detail"
    );
    assert!(body.get("total_input_tokens").is_none());
    assert!(body.get("total_thinking_tokens").is_none());
    assert!(body.get("total_output_tokens").is_none());
    assert!(body.get("total_tool_calls").is_none());
    assert!(body.get("total_estimated_cost").is_none());
}

#[tokio::test]
async fn broken_session_db_schema_is_explicit_error_for_session_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-broken-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(session_dir.join("session.db")).unwrap();
    conn.execute("DROP TABLE net_events", []).unwrap();
    conn.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(conn);
    let entry = test_persistent_entry("status-broken-db-vm", session_dir);
    let vm_id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("status-broken-db-vm".to_string(), entry);
    state.hydrate_session_db_handles();
    let handle = state
        .session_db_handle(&vm_id)
        .expect("startup hydration installs the handle so routes surface the schema error explicitly");
    assert!(
        handle.ready().await.is_err(),
        "a malformed session schema must fail readiness instead of being treated as ready"
    );

    let (status, body) = route_request(app, axum::http::Method::GET, &format!("/vms/{vm_id}/info"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_db"]["ready"], false,
        "broken session schemas must be visible in status instead of being treated as ready: {body}"
    );
    let error = body["session_db"]["error"]
        .as_str()
        .expect("broken DB status must carry the explicit DB readiness error");
    assert!(
        error.contains("not ready") || error.contains("missing required column") || error.contains("no such column"),
        "broken DB status must expose the schema failure, got: {error}"
    );
}

#[tokio::test]
async fn stats_detail_ledger_exposes_orphan_tool_parent_inconsistency() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("ledger-inconsistent-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "ledger-inconsistent-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO tool_calls (
            event_id, timestamp, model_call_id, provider, status, call_index,
            call_id, tool_name, arguments, origin, server_name, method,
            decision, duration_ms, trace_id, turn_id, credential_ref
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17
         )",
        rusqlite::params![
            "badbad000001",
            "2026-06-24T01:02:03Z",
            99_999_i64,
            "google",
            "observed",
            0_i64,
            "orphan-tool",
            "Write",
            r#"{"path":"/root/orphan.md","content":"ledger proof"}"#,
            "model",
            "model",
            "tool.call",
            "allowed",
            13_i64,
            "trace-orphan-tool",
            "trace-orphan-tool",
            "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tool_responses (
            model_call_id, call_id, content_preview, is_error, trace_id, turn_id,
            credential_ref
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            99_999_i64,
            "orphan-tool",
            "Wrote orphan.md",
            0_i64,
            "trace-orphan-tool",
            "trace-orphan-tool",
            "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    )
    .unwrap();
    drop(conn);

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_events"].as_array().unwrap().len(), 0);
    assert_eq!(body["model_stats"].as_array().unwrap().len(), 0);
    assert_eq!(body["tool_events"].as_array().unwrap().len(), 1);
    let tool = &body["tool_events"][0];
    assert_eq!(tool["event_id"], "badbad000001");
    assert_eq!(tool["call_id"], "orphan-tool");
    assert_eq!(tool["model_call_id"], 99_999);
    assert_eq!(tool["model_parent_missing"], 1);
    assert_eq!(tool["source"], "model");
    assert_eq!(tool["server_name"], "model");
    assert_eq!(tool["tool_name"], "Write");
    assert_eq!(
        tool["arguments"],
        r#"{"path":"/root/orphan.md","content":"ledger proof"}"#
    );
    assert_eq!(tool["response_preview"], "Wrote orphan.md");
    assert_eq!(
        tool["credential_ref"],
        "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );

    let (status, info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        info.get("total_tool_calls"),
        None,
        "/vms/{{id}}/info stays lifecycle/storage only; stats/detail is the ledger surface"
    );
    assert!(
        info.get("model_call_count").is_none() || info["model_call_count"] == serde_json::Value::Null,
        "orphan tool diagnostics must not invent a model count"
    );

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/timeline?trace_id=trace-orphan-tool&layers=tool&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let rows = timeline["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{timeline}");
    assert_eq!(rows[0][1], "tool");
    assert_eq!(rows[0][3], "model/Write (call_id=orphan-tool)");
    assert_eq!(rows[0][4], "allowed");
    assert_eq!(rows[0][6], "trace-orphan-tool");
}

// -----------------------------------------------------------------------
