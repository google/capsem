use super::*;
use crate::tests::{make_test_state, test_instance};

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
