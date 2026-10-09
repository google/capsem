use super::*;
use crate::tests::{make_test_state, test_instance};

#[tokio::test]
async fn stale_shutdown_binding_leaves_replacement_socket_and_ledger_intact() {
    let state = make_test_state();
    let socket = state.run_dir.join("replacement.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let ready = socket.with_extension("ready");
    std::fs::write(&ready, b"replacement ready").unwrap();
    let mut instance = test_instance();
    instance.id = "replacement".into();
    instance.pid = 0;
    instance.uds_path = socket.clone();
    let generation = instance.generation;
    state.instances.lock().unwrap().insert("replacement".into(), instance);
    let ledger_dir = tempfile::tempdir().unwrap();
    let writer = capsem_logger::DbWriter::open(&ledger_dir.path().join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    let ledger = state
        .register_session_db_handle("replacement", ledger_dir.path())
        .unwrap();
    let result = shutdown_vm_process(&state, "replacement", ShutdownMode::Discard, Some(uuid::Uuid::new_v4())).await;
    assert_eq!(result.unwrap_err().0, StatusCode::CONFLICT);
    assert_eq!(
        state.instances.lock().unwrap().get("replacement").unwrap().generation,
        generation
    );
    assert!(Arc::ptr_eq(
        state.session_db_handles.lock().unwrap().get("replacement").unwrap(),
        &ledger
    ));
    assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
    assert_eq!(std::fs::read(&ready).unwrap(), b"replacement ready");
    drop(listener);
}

#[tokio::test]
async fn bound_shutdown_reports_missing_ownership_without_fabricating_completion() {
    let state = make_test_state();
    assert!(
        shutdown_vm_process(&state, "absent", ShutdownMode::Discard, Some(capsem_crypto::uuid_v4()))
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn selected_spawn_generation_is_durable_and_corruption_cannot_be_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let generation = capsem_crypto::uuid_v4();
    persist_spawn_identity(root.path(), "selected-vm", generation).unwrap();
    let session = capsem_foundation::unix::contained::ContainedDir::open_root(root.path()).unwrap();
    let stored = capsem_core::session::read_spawn_identity(&session).unwrap().unwrap();
    assert_eq!(stored.id(), "selected-vm");
    assert_eq!(stored.generation(), generation);
    std::fs::write(root.path().join("spawn-identity.json"), b"corrupt").unwrap();
    assert!(persist_spawn_identity(root.path(), "selected-vm", capsem_crypto::uuid_v4()).is_err());
    assert_eq!(
        std::fs::read(root.path().join("spawn-identity.json")).unwrap(),
        b"corrupt"
    );
}

#[test]
fn selected_spawn_resume_requires_completed_original_generation_and_never_replays() {
    let root = tempfile::tempdir().unwrap();
    let first = capsem_crypto::uuid_v4();
    persist_spawn_identity(root.path(), "selected-vm", first).unwrap();
    let binding = capsem_core::managed_sessions::VmBinding::new("selected-vm".into(), first).unwrap();
    let record = root.path().join("spawn-owner.json");
    assert!(record.exists(), "launch must prepare a durable admission witness");
    let second = capsem_crypto::uuid_v4();
    persist_spawn_identity(root.path(), "selected-vm", second).unwrap();
    assert!(capsem_core::session::spawn_owner::enter(root.path(), &binding, std::process::id()).is_err());
    assert!(persist_spawn_identity(root.path(), "selected-vm", second).is_err());
    let current = capsem_core::managed_sessions::VmBinding::new("selected-vm".into(), second).unwrap();
    let contents: serde_json::Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    assert_eq!(contents["parent_pid"], std::process::id());
    assert_eq!(contents["binding"], serde_json::to_value(current).unwrap());
}

#[tokio::test]
async fn normal_native_exit_persists_retirement_before_next_host_boot_resume() {
    let state = make_test_state();
    let session = state.run_dir.join("sessions/receipt-vm");
    std::fs::create_dir_all(&session).unwrap();
    let generation = capsem_crypto::uuid_v4();
    let binding = capsem_core::managed_sessions::VmBinding::new("receipt-vm".into(), generation).unwrap();
    persist_spawn_identity(&session, binding.id(), generation).unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "managed_sessions::tests::recovery_native_child_fixture",
            "--nocapture",
        ])
        .env_clear()
        .env("CAPSEM_TEST_RECOVERY_ROOT", &session)
        .env("CAPSEM_TEST_RECOVERY_BINDING", serde_json::to_string(&binding).unwrap())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true);
    let (mut child, retirement) = state.retirements.spawn(binding.id(), generation, &mut command).unwrap();
    let record_path = session.join("spawn-owner.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let record: serde_json::Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        if record["pid"].as_u64() == child.id().map(u64::from) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child admission was not published"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    drop(child.stdin.take());
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        binding.id().into(),
        binding.id().into(),
        Arc::clone(&state),
        state.instance_socket_path(binding.id()).unwrap(),
        session.clone(),
        retirement,
    );
    reaper.await.unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        record["retired"], true,
        "normal stop must persist its native exit receipt"
    );
    record["scope"]["boot"] = serde_json::json!(capsem_crypto::uuid_v4().to_string());
    std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    persist_spawn_identity(&session, binding.id(), capsem_crypto::uuid_v4()).unwrap();
}

#[test]
fn nil_selected_generation_refuses_provision_before_any_session_side_effect() {
    let state = make_test_state();
    let result = state.provision_sandbox_generation(
        ProvisionOptions {
            id: "never-started",
            name: "never-started",
            ram_mb: 2048,
            cpus: 2,
            scratch_disk_size_gb: 16,
            version_override: None,
            persistent: false,
            env: None,
            from: None,
            description: None,
        },
        uuid::Uuid::nil(),
    );
    assert!(result.unwrap_err().to_string().contains("generation"));
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!state.run_dir.join("sessions/never-started").exists());
}

#[test]
fn teardown_claim_is_bound_to_generation_even_when_id_and_pid_are_reused() {
    let state = make_test_state();
    let original = test_instance();
    let generation = original.generation;
    let pid = original.pid;
    state.instances.lock().unwrap().insert("vm".into(), original);
    let mut replacement = test_instance();
    replacement.pid = pid;
    let replacement_generation = replacement.generation;
    assert_ne!(generation, replacement_generation);
    state.instances.lock().unwrap().insert("vm".into(), replacement);
    assert!(!claim_shutdown_instance(&state, "vm", generation));
    assert_eq!(
        state.instances.lock().unwrap().get("vm").unwrap().generation,
        replacement_generation
    );
    assert!(claim_shutdown_instance(&state, "vm", replacement_generation));
    assert!(!claim_shutdown_instance(&state, "vm", replacement_generation));
}

#[tokio::test]
async fn removing_or_replacing_an_instance_revokes_its_grants() {
    let state = make_test_state();
    let original = test_instance();
    let generation = original.generation;
    let grant = original.authority.grant();
    let disposable = original.authority.grant();
    drop(disposable);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), grant.revoked())
            .await
            .is_err(),
        "dropping a non-owning grant cannot revoke its worker"
    );
    state.instances.lock().unwrap().insert("vm".into(), original);
    assert!(state.evict_instance("vm", generation).is_some());
    tokio::time::timeout(std::time::Duration::from_millis(100), grant.revoked())
        .await
        .expect("registry removal revokes the generation");

    let replacement_target = test_instance();
    let replaced = replacement_target.authority.grant();
    state.instances.lock().unwrap().insert("vm".into(), replacement_target);
    state.instances.lock().unwrap().insert("vm".into(), test_instance());
    tokio::time::timeout(std::time::Duration::from_millis(100), replaced.revoked())
        .await
        .expect("registry replacement revokes the displaced generation");
}
