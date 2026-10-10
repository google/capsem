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
        shutdown_vm_process(&state, "absent", ShutdownMode::Discard, Some(uuid::Uuid::new_v4()))
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn selected_spawn_generation_is_durable_and_corruption_cannot_be_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let generation = uuid::Uuid::new_v4();
    persist_spawn_identity(root.path(), "selected-vm", generation).unwrap();
    let session = capsem_foundation::unix::contained::ContainedDir::open_root(root.path()).unwrap();
    let stored = capsem_core::session::read_spawn_identity(&session).unwrap().unwrap();
    assert_eq!(stored.id(), "selected-vm");
    assert_eq!(stored.generation(), generation);
    std::fs::write(root.path().join("spawn-identity.json"), b"corrupt").unwrap();
    assert!(persist_spawn_identity(root.path(), "selected-vm", uuid::Uuid::new_v4()).is_err());
    assert_eq!(
        std::fs::read(root.path().join("spawn-identity.json")).unwrap(),
        b"corrupt"
    );
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
    assert!(!grant.is_revoked());
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
    assert!(grant.is_revoked());

    let replacement_target = test_instance();
    let replaced = replacement_target.authority.grant();
    state.instances.lock().unwrap().insert("vm".into(), replacement_target);
    state.instances.lock().unwrap().insert("vm".into(), test_instance());
    tokio::time::timeout(std::time::Duration::from_millis(100), replaced.revoked())
        .await
        .expect("registry replacement revokes the displaced generation");
}
