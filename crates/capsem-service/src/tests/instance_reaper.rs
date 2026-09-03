use super::*;

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        name: "../evil",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must start with") || err.contains("must contain only"),
        "expected name validation error, got: {err}"
    );
}

#[test]
fn child_reapers_start_after_instance_registration() {
    let source = include_str!("../main.rs");
    for (function, next_function, reaper) in [
        (
            "    fn provision_sandbox(",
            "    fn resume_sandbox(",
            "instance_reaper::spawn_exit_reaper(",
        ),
        (
            "    fn resume_sandbox(",
            "    fn has_existing_resume_checkpoint(",
            "instance_reaper::spawn_exit_reaper(",
        ),
    ] {
        let start = source.find(function).expect("launch function exists");
        let end = source[start..]
            .find(next_function)
            .map(|offset| start + offset)
            .expect("following function exists");
        let body = &source[start..end];
        let insertion = body
            .find("instances.insert(")
            .expect("launch function registers its instance");
        let reaper = body.find(reaper).expect("launch function starts its child reaper");

        assert!(
            insertion < reaper,
            "{function} must publish the instance before its child reaper can run"
        );
    }
}

/// There is one child reaper. The resume path had its own, which removed the
/// instance and nothing else: no session-index stop, no checkpoint or defunct
/// bookkeeping in the persistent registry, no crash evidence. A resumed
/// persistent VM that suspended again was never marked suspended, and one
/// that crashed was never marked defunct.
#[test]
fn the_resume_path_has_no_reaper_of_its_own() {
    let source = include_str!("../instance_reaper.rs");
    assert!(
        !source.contains("fn spawn_resume"),
        "every capsem-process child exits through spawn_exit_reaper"
    );
}

#[tokio::test]
async fn the_reaper_marks_a_crashed_persistent_vm_defunct() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent").join("crash-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), "boot: kernel panic\n").unwrap();
    let entry = test_persistent_entry("crashy", session_dir.clone());
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("crashy".to_string(), entry);
    insert_fake_instance_with_session_dir(&state, &id, std::process::id(), session_dir.clone());
    let uds_path = state.run_dir.join("instances").join(format!("{id}.sock"));

    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 3"])
        .spawn()
        .expect("spawn a child that crashes");
    crate::instance_reaper::spawn_exit_reaper(
        child,
        id.clone(),
        "crashy".to_string(),
        Arc::clone(&state),
        uds_path,
        session_dir,
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.instances.lock().unwrap().contains_key(&id) {
        assert!(
            std::time::Instant::now() < deadline,
            "reaper never removed the instance"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (defunct, suspended, last_error) = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("crashy")
        .map(|entry| (entry.defunct, entry.suspended, entry.last_error.clone()))
        .expect("entry survives the crash");
    assert!(defunct, "an unexpected exit must mark the persistent VM defunct");
    assert!(!suspended);
    assert!(
        last_error.as_deref().is_some_and(|tail| tail.contains("kernel panic")),
        "last_error carries the process log tail: {last_error:?}"
    );
}
