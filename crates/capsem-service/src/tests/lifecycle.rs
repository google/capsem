use super::*;

#[test]
fn tempdir_test_states_use_distinct_host_ledgers() {
    let (first, first_dir) = make_test_state_with_tempdir();
    let (second, second_dir) = make_test_state_with_tempdir();

    let first_db = first.host_ledger.path().to_path_buf();
    let second_db = second.host_ledger.path().to_path_buf();

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
        "independent test states must not share a host ledger"
    );

    let first_owned = make_test_state();
    let second_owned = make_test_state();
    assert_ne!(
        first_owned.host_ledger.path().to_path_buf(),
        second_owned.host_ledger.path().to_path_buf(),
        "test states that own their tempdirs must not share a host ledger"
    );
}

#[tokio::test]
async fn handle_persist_preserves_asset_pins() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("sessions/persist-src");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.instances.lock().unwrap().insert(
        "persist-src".into(),
        InstanceInfo {
            id: "persist-src".into(),
            name: "persist-src".into(),
            uds_path: PathBuf::from("/tmp/persist-src.sock"),
            session_dir: session_dir.clone(),
            ..test_instance()
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
    assert_eq!(entry.asset_pins, test_asset_pins());
    drop(registry);

    let instances = state.instances.lock().unwrap();
    let info = instances.get("persist-src").unwrap();
    assert_eq!(info.id, "persist-src");
    assert_eq!(info.asset_pins, test_asset_pins());
    assert!(info.persistent);
    drop(instances);
}

fn profile_era_entry(state: &ServiceState, name: &str) -> PersistentVmEntry {
    let session_dir = state.run_dir.join("persistent").join(name);
    let rootfs = capsem_core::session::system_overlay_image_path(&session_dir);
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(u64::from(DEFAULT_SCRATCH_DISK_GB) * 1024 * 1024 * 1024)
        .unwrap();
    PersistentVmEntry {
        legacy_profile_id: Some("code".into()),
        ..test_persistent_entry(name, session_dir)
    }
}

fn register_entry(state: &ServiceState, entry: PersistentVmEntry) {
    let name = entry.name.clone();
    state.persistent_registry.lock().unwrap().data.vms.insert(name, entry);
}

/// A VM created while VMs had profiles is refused at resume with the reason,
/// never adapted to the current shape.
#[test]
fn resume_refuses_a_profile_era_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let entry = profile_era_entry(&state, "profile-era");
    let vm_id = entry.id.clone();
    register_entry(&state, entry);

    let err = state.resume_sandbox(&vm_id, None, None).unwrap_err().to_string();
    assert!(
        err.contains("created from the 'code' profile, and profiles no longer exist"),
        "resume must refuse a profile-era VM, got: {err}"
    );
}

#[test]
fn persistent_resume_state_marks_a_profile_era_vm_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let entry = profile_era_entry(&state, "profile-era-status");

    let (status, can_resume, reason) = state.persistent_entry_resume_state_cached(&entry);
    assert_eq!(status, VmLifecycleState::Incompatible);
    assert!(!can_resume);
    assert!(reason.unwrap().contains("profiles no longer exist"));
}

#[test]
fn persistent_resume_allows_deprecated_pins_but_blocks_explicit_revocation() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("persistent/deprecated-pin");
    let rootfs = capsem_core::session::system_overlay_image_path(&session_dir);
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(u64::from(DEFAULT_SCRATCH_DISK_GB) * 1024 * 1024 * 1024)
        .unwrap();
    let entry = test_persistent_entry("deprecated-pin", session_dir);
    let hash = entry.asset_pins.rootfs.hash.strip_prefix("blake3:").unwrap();
    let manifest_path = state.assets_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "runtime": {
                "status": "supported",
                "architectures": [{
                    "images": [{
                        "kind": "rootfs",
                        "status": "deprecated",
                        "digest": {"blake3": hash}
                    }]
                }]
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
    manifest["runtime"]["architectures"][0]["images"][0]["status"] = serde_json::json!("revoked");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let (status, can_resume, reason) = state.persistent_entry_resume_state_cached(&entry);
    assert_eq!(status, VmLifecycleState::Incompatible);
    assert!(!can_resume);
    assert!(reason.unwrap().contains("explicitly revoked image"));
}

#[test]
fn provision_rejects_nonexistent_source_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        name: "vm1",
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
fn provision_refuses_to_clone_a_profile_era_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    register_entry(&state, profile_era_entry(&state, "profile-era-source"));
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        name: "vm1",
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("profile-era-source".into()),
        description: None,
    });
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("profiles no longer exist"),
        "a clone must not launder a profile-era VM, got: {err}"
    );
    assert!(!state.run_dir.join("sessions/vm1").exists());
}

// Suspend/resume registry fixes (issues #4-8)

#[tokio::test]
async fn handle_list_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
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
                suspended: true,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                ..test_persistent_entry("susp-vm", suspended_dir)
            },
        );
    }

    // Register a stopped (not suspended) persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "stop-vm".into(),
            PersistentVmEntry {
                ram_mb: 1024,
                cpus: 1,
                ..test_persistent_entry("stop-vm", stopped_dir)
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
    install_test_runtime_assets(&state);
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
                legacy_profile_id: None,
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
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-info");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    let rootfs = session_dir.join("system/rootfs.img");
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
    install_test_runtime_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-status");
    capsem_core::create_virtiofs_session(&session_dir, 4).unwrap();
    let rootfs = session_dir.join("system/rootfs.img");

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
async fn handle_list_marks_a_profile_era_vm_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    register_entry(&state, profile_era_entry(&state, "profile-era-list"));

    let list: ListResponse = decode_response_json(handle_list(State(state)).await).await;
    let vm = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("profile-era-list"))
        .unwrap();
    assert_eq!(vm.status, VmLifecycleState::Incompatible);
    assert!(!vm.can_resume);
    assert_eq!(vm.available_actions, vec![VmAction::Delete], "it can only be deleted");
    assert!(vm
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("profiles no longer exist"));
}

#[tokio::test]
async fn handle_info_marks_a_profile_era_vm_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let entry = profile_era_entry(&state, "profile-era-info");
    let vm_id = entry.id.clone();
    register_entry(&state, entry);

    let Json(info) = handle_info(State(state), Path(vm_id)).await.unwrap();
    assert_eq!(info.status, VmLifecycleState::Incompatible);
    assert!(!info.can_resume);
    assert!(info
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("profiles no longer exist"));
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
                uds_path: state.run_dir.join("instances/eph-vm.sock"),
                session_dir: state.run_dir.join("sessions/eph-vm"),
                pid: 0,
                ..test_instance()
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
                suspended: true,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                ..test_persistent_entry("resume-vm", session_dir.clone())
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
            uds_path: uds_path.clone(),
            session_dir,
            pid: 0,
            persistent: true,
            ..test_instance()
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
                legacy_profile_id: None,
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
                legacy_profile_id: None,
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

// -----------------------------------------------------------------------
// SandboxInfo::new
// -----------------------------------------------------------------------

#[test]
fn sandbox_info_new_defaults_telemetry_to_none() {
    let info = SandboxInfo::new("test".into(), 1, VmLifecycleState::Running, false);
    assert_eq!(info.id, "test");
    assert_eq!(info.pid, 1);
    assert!(!info.persistent);
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
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
    let mut info = SandboxInfo::new("test".into(), 1, VmLifecycleState::Running, false);
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
    let info = SandboxInfo::new("test".into(), 1, VmLifecycleState::Running, false);
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("total_input_tokens"));
    assert!(!json.contains("total_estimated_cost"));
    assert!(!json.contains("model_call_count"));
    assert!(!json.contains("uptime_secs"));
}

#[test]
fn sandbox_info_needs_no_profile_id() {
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false,"available_actions":[]}"#;
    assert!(serde_json::from_str::<SandboxInfo>(json).is_ok());
}

#[test]
fn create_flags_override_the_service_default_resources() {
    let defaults = resolve_vm_resources(None, None);
    assert_eq!(defaults.cpus, 4);
    assert_eq!(defaults.ram_mb, 12 * 1024);
    assert_eq!(defaults.scratch_disk_size_gb, 64);

    let customized = resolve_vm_resources(Some(3072), Some(2));
    assert_eq!(customized.cpus, 2);
    assert_eq!(customized.ram_mb, 3072);
    assert_eq!(
        customized.scratch_disk_size_gb, 64,
        "the scratch image size is not a create flag"
    );
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
        request_body: Some(r#"{"contents":[{"text":"write full bounded body"}]}"#.as_bytes().to_vec()),
        message_id: Some("msg-1".to_string()),
        status_code: Some(200),
        text_content: Some("created poem.md".to_string()),
        thinking_content: Some("plan file write".to_string()),
        response_body: Some(
            r#"{"candidates":[{"content":{"parts":[{"text":"created poem.md"}]}}]}"#
                .as_bytes()
                .to_vec(),
        ),
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
            event_id: None,
            call_index: 0,
            call_id: "tool-1".to_string(),
            tool_name: "Create".to_string(),
            arguments: Some(r#"{"path":"/root/poem.md"}"#.to_string()),
            origin: "native".to_string(),
            trace_id: Some("trace-stats-detail".to_string()),
        }],
        tool_responses: vec![capsem_logger::ToolResponseEntry {
            event_id: None,
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
            request_body: Some(
                r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#
                    .as_bytes()
                    .to_vec(),
            ),
            response_body: Some(r#"{"ok":true,"body":"full response body from gateway"}"#.as_bytes().to_vec()),
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
    assert_eq!(body["tool_events"][0]["model_parent_missing"], false);
    assert!(body["tool_events"][0]["model_call_id"].as_i64().is_some());
    assert_eq!(body["tool_events"][0]["arguments"], r#"{"path":"/root/poem.md"}"#);
    assert_eq!(body["tool_events"][0]["response_preview"], "Wrote 4 lines to poem.md");
    assert_eq!(body["http_events"][0]["event_id"], "def456def456");
    assert_eq!(body["http_events"][0]["domain"], "generativelanguage.googleapis.com");
    assert!(body["http_events"][0].get("request_body_preview").is_none());
    assert!(body["http_events"][0].get("response_body_preview").is_none());
    assert_eq!(body["body_blobs"]["abc123abc123"][0]["direction"], "request");
    assert!(body["body_blobs"]["abc123abc123"][0].get("body").is_none());
    assert_eq!(body["body_blobs"]["abc123abc123"][1]["direction"], "response");
    assert!(body["body_blobs"]["abc123abc123"][1].get("body").is_none());
    assert_eq!(body["body_blobs"]["def456def456"][0]["direction"], "request");
    assert!(body["body_blobs"]["def456def456"][0].get("body").is_none());
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["stored_bytes"],
        r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#.len()
    );
    assert_eq!(body["body_blobs"]["def456def456"][1]["direction"], "response");
    assert!(body["body_blobs"]["def456def456"][1].get("body").is_none());

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
    // /info carries the same totals the toolbar summary reports.
    assert_eq!(info["total_input_tokens"], summary["total_input_tokens"]);
    assert_eq!(info["total_output_tokens"], summary["total_output_tokens"]);
    assert_eq!(info["total_tool_calls"], summary["total_tool_calls"]);
    assert_eq!(info["total_requests"], summary["total_requests"]);
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
        request_body: None,
        message_id: Some(format!("{event_id}-message")),
        status_code: Some(200),
        text_content: Some("ok".to_string()),
        thinking_content: None,
        response_body: None,
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
async fn resume_sandbox_passes_the_session_scratch_disk_size_to_process() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let settings_dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, _) = install_empty_settings_env(&settings_dir);
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
    install_test_runtime_assets(&state);

    let vm_id = new_persistent_vm_id();
    let session_dir = state.run_dir.join("persistent").join(&vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let rootfs = capsem_core::session::system_overlay_image_path(&session_dir);
    std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
    std::fs::File::create(rootfs)
        .unwrap()
        .set_len(24 * 1024 * 1024 * 1024)
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
    assert_eq!(
        size_flag,
        Some("24"),
        "resume must keep the size the session's system overlay was created with; argv={args:?}"
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
    let source = ["main.rs", "ledger_routes/vm_info.rs"]
        .into_iter()
        .map(|path| {
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(path))
                .expect("service source must be readable")
        })
        .collect::<Vec<_>>()
        .join("\n");
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
    assert_eq!(tool["model_parent_missing"], true);
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
        info["model_call_count"], 0,
        "orphan tool diagnostics must not invent a model count: {info}"
    );

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/timeline?trace_id=trace-orphan-tool&layers=tool&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let rows = timeline["events"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{timeline}");
    assert_eq!(rows[0]["layer"], "tool");
    assert_eq!(rows[0]["summary"], "model/Write (call_id=orphan-tool)");
    assert_eq!(rows[0]["status"], "allowed");
    assert_eq!(rows[0]["trace_id"], "trace-orphan-tool");
}

// -----------------------------------------------------------------------
