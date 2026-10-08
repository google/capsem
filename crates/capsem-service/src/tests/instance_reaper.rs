use super::*;

#[tokio::test]
async fn exited_generation_is_revoked_while_its_pid_is_still_reserved() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "unreaped-owner";
    let session = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session).unwrap();
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "sleep 0.05; exit 11"])
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    insert_fake_instance_with_session_dir(&state, id, pid, session);
    let generation = state.instances.lock().unwrap()[id].generation;
    let grant = state.instances.lock().unwrap()[id].authority.grant();

    let revoked = crate::instance_reaper::revoke_exited_generation(&child, id, &state, generation).await;

    assert!(!state.instances.lock().unwrap().contains_key(id));
    tokio::time::timeout(std::time::Duration::from_millis(100), grant.revoked())
        .await
        .expect("exit observation revokes grants before the removed instance is dropped");
    let pid = capsem_foundation::unix::process::ProcessId::try_from(pid).unwrap();
    assert!(
        capsem_foundation::unix::process::child_has_exited(pid).unwrap(),
        "generation revocation must happen before the child is reaped"
    );
    drop(revoked);
    assert_eq!(child.wait().await.unwrap().code(), Some(11));
}

#[tokio::test]
async fn generation_retirement_waits_for_reaper_after_instance_map_removal() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "retirement-owner";
    let session = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session).unwrap();
    let uds = state.instance_socket_path(id).unwrap();
    std::fs::create_dir_all(uds.parent().unwrap()).unwrap();
    std::fs::write(uds.with_extension("ready"), b"original").unwrap();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    insert_fake_instance_with_session_dir(&state, id, child.id().unwrap(), session.clone());
    let generation = state.instances.lock().unwrap().get(id).unwrap().generation;
    let retirement = state.retirements.register(id, generation).unwrap();
    let resume = state.lifecycle.vz.write().await;
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        uds.clone(),
        session,
        retirement,
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state.instances.lock().unwrap().contains_key(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let completion = {
        let state = Arc::clone(&state);
        tokio::spawn(async move { state.retirements.wait(id, generation).await })
    };
    tokio::task::yield_now().await;
    assert!(!completion.is_finished(), "map removal is not completed retirement");
    assert!(uds.with_extension("ready").exists());
    drop(resume);
    reaper.await.unwrap();
    assert!(completion.await.unwrap().unwrap());
    assert!(
        state.retirements.wait(id, generation).await.unwrap(),
        "a later reconciliation sees the same completed generation"
    );
    assert!(!uds.with_extension("ready").exists());
}

#[tokio::test]
async fn aborted_reaper_never_reports_generation_retired() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "aborted-retirement";
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exec sleep 30"])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    insert_fake_instance_with_session_dir(&state, id, child.id().unwrap(), state.run_dir.join("sessions").join(id));
    let generation = state.instances.lock().unwrap().get(id).unwrap().generation;
    let retirement = state.retirements.register(id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        state.instance_socket_path(id).unwrap(),
        state.run_dir.join("sessions").join(id),
        retirement,
    );
    reaper.abort();
    assert!(reaper.await.unwrap_err().is_cancelled());
    assert!(state.retirements.wait(id, generation).await.is_err());
}

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        name: "../evil",
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
    state
        .lifecycle
        .begin_restart(|| !state.instances.lock().unwrap().is_empty())
        .unwrap();
}

#[test]
fn accepted_restart_refuses_both_launch_paths_before_any_session_mutation() {
    let (state, _dir) = make_test_state_with_tempdir();
    state
        .lifecycle
        .begin_restart(|| !state.instances.lock().unwrap().is_empty())
        .unwrap();
    let provision = state.provision_sandbox(ProvisionOptions {
        id: "never-started",
        name: "never-started",
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    let resume = state.resume_sandbox("never-started", None, None);
    for error in [provision.unwrap_err(), resume.unwrap_err()] {
        assert_eq!(
            error.downcast_ref::<capsem_service::lifecycle::RestartDenied>(),
            Some(&capsem_service::lifecycle::RestartDenied::AlreadyRequested),
        );
    }
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!state.persistent_registry.lock().unwrap().contains("never-started"));
    assert!(!state.run_dir.join("persistent/never-started").exists());
    assert!(!state.run_dir.join("sessions/never-started").exists());
}

#[test]
fn child_reapers_start_after_instance_registration() {
    for (function, body) in [
        ("provision_sandbox", include_str!("../vm_lifecycle/provision.rs")),
        ("resume_sandbox", include_str!("../vm_lifecycle/resume_process.rs")),
    ] {
        let insertion = body
            .find("instances.insert(")
            .expect("launch function registers its instance");
        let reaper = body
            .find("instance_reaper::spawn_exit_reaper(")
            .expect("launch function starts its child reaper");

        assert!(
            insertion < reaper,
            "{function} must publish the instance before its child reaper can run"
        );
    }
}

#[test]
fn child_exit_authority_is_revoked_before_reaping() {
    let source = include_str!("../instance_reaper.rs")
        .split_once("pub(super) fn spawn_exit_reaper")
        .unwrap()
        .1;
    let revoke = source
        .find("revoke_exited_generation(&child")
        .expect("the reaper observes exit and revokes the registered generation");
    let reap = source
        .find("child.wait().await")
        .expect("the reaper eventually consumes the child exit status");
    assert!(
        revoke < reap,
        "the kernel must still reserve the worker PID when its authority is revoked"
    );
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
    let uds_path = state.run_dir.join("instances").join(format!("{id}.sock"));

    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 3"])
        .spawn()
        .expect("spawn a child that crashes");
    insert_fake_instance_with_session_dir(&state, &id, child.id().unwrap(), session_dir.clone());
    let retirement_id: &str = id.as_ref();
    let generation = state
        .instances
        .lock()
        .unwrap()
        .get(retirement_id)
        .filter(|instance| Some(instance.pid) == child.id())
        .map(|instance| instance.generation)
        .unwrap_or_else(uuid::Uuid::new_v4);
    let retirement = state.retirements.register(retirement_id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.clone(),
        "crashy".to_string(),
        Arc::clone(&state),
        uds_path,
        session_dir,
        retirement,
    );

    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert!(!state.instances.lock().unwrap().contains_key(&id));
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

#[tokio::test]
async fn exit_cleanup_waits_for_resume_and_preserves_the_replacement() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let entry = test_persistent_entry("resume-vm", session_dir.clone());
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("resume-vm".into(), entry);
    let uds_path = state.instance_socket_path(&id).unwrap();
    std::fs::create_dir_all(uds_path.parent().unwrap()).unwrap();

    let resume = state.lifecycle.vz.write().await;
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    insert_fake_instance_with_session_dir(&state, &id, pid, session_dir.clone());
    let retirement_id: &str = id.as_ref();
    let generation = state
        .instances
        .lock()
        .unwrap()
        .get(retirement_id)
        .filter(|instance| Some(instance.pid) == child.id())
        .map(|instance| instance.generation)
        .unwrap_or_else(uuid::Uuid::new_v4);
    let retirement = state.retirements.register(retirement_id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.clone(),
        "resume-vm".into(),
        Arc::clone(&state),
        uds_path.clone(),
        session_dir.clone(),
        retirement,
    );
    assert!(wait_for_process_exit(pid, std::time::Duration::from_secs(5)).await);
    let blocked_during_resume = !reaper.is_finished();

    // A warm-restore fallback can replace the exited child while holding the
    // exclusive lifecycle lock. The old reaper must recognize its successor.
    insert_fake_instance_with_session_dir(&state, &id, std::process::id(), session_dir.clone());
    let db_path = session_dir.join("session.db");
    tokio::task::spawn_blocking(move || {
        capsem_logger::DbWriter::open(&db_path, 16).unwrap().shutdown_blocking();
    })
    .await
    .unwrap();
    let db_handle = state.register_session_db_handle(&id, &session_dir).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), "replacement").unwrap();
    drop(resume);
    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert!(
        blocked_during_resume,
        "exit cleanup must wait until restore releases its lifecycle lock"
    );
    assert_eq!(
        state.instances.lock().unwrap().get(&id).unwrap().pid,
        std::process::id()
    );
    assert!(
        Arc::ptr_eq(state.session_db_handles.lock().unwrap().get(&id).unwrap(), &db_handle,),
        "old cleanup unregistered the replacement's logger-owned DB handle"
    );
    assert!(
        std::os::unix::net::UnixStream::connect(&uds_path).is_ok(),
        "old cleanup unlinked the replacement socket"
    );
    assert_eq!(
        std::fs::read_to_string(uds_path.with_extension("ready")).unwrap(),
        "replacement"
    );
    drop(listener);
}

#[tokio::test]
async fn a_crashed_restore_reports_exit_before_the_resume_lock_is_released() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "crashed-restore";
    let session_dir = state.run_dir.join("persistent").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let uds_path = state.instance_socket_path(id).unwrap();
    let resume = state.lifecycle.vz.write().await;
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 3"])
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    insert_fake_instance_with_session_dir(&state, id, pid, session_dir.clone());
    let retirement_id: &str = id;
    let generation = state
        .instances
        .lock()
        .unwrap()
        .get(retirement_id)
        .filter(|instance| Some(instance.pid) == child.id())
        .map(|instance| instance.generation)
        .unwrap_or_else(uuid::Uuid::new_v4);
    let retirement = state.retirements.register(retirement_id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        uds_path.clone(),
        session_dir,
        retirement,
    );
    assert!(wait_for_process_exit(pid, std::time::Duration::from_secs(5)).await);
    let result = wait_for_vm_ready(&uds_path, 1, Some(&state), Some(id)).await;
    drop(resume);
    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err(), "capsem-process exited before signalling ready");
}

#[tokio::test]
async fn an_already_replaced_child_cannot_claim_the_new_instance() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "already-replaced";
    let session_dir = state.run_dir.join("persistent").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let uds_path = state.instance_socket_path(id).unwrap();
    std::fs::create_dir_all(uds_path.parent().unwrap()).unwrap();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    insert_fake_instance_with_session_dir(&state, id, std::process::id(), session_dir.clone());
    let listener = std::os::unix::net::UnixListener::bind(&uds_path).unwrap();
    let retirement_id: &str = id;
    let generation = state
        .instances
        .lock()
        .unwrap()
        .get(retirement_id)
        .filter(|instance| Some(instance.pid) == child.id())
        .map(|instance| instance.generation)
        .unwrap_or_else(uuid::Uuid::new_v4);
    let retirement = state.retirements.register(retirement_id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        uds_path.clone(),
        session_dir,
        retirement,
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.instances.lock().unwrap().get(id).unwrap().pid, std::process::id());
    assert!(std::os::unix::net::UnixStream::connect(&uds_path).is_ok());
    drop(listener);
}

#[tokio::test]
async fn stale_reaper_cannot_remove_replacement_with_the_same_pid() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "same-pid-replacement";
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let uds_path = state.instance_socket_path(id).unwrap();
    std::fs::create_dir_all(uds_path.parent().unwrap()).unwrap();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    insert_fake_instance_with_session_dir(&state, id, pid, session_dir.clone());
    let original = state.instances.lock().unwrap().get(id).unwrap().generation;
    let retirement_id: &str = id;
    let generation = state
        .instances
        .lock()
        .unwrap()
        .get(retirement_id)
        .filter(|instance| Some(instance.pid) == child.id())
        .map(|instance| instance.generation)
        .unwrap_or_else(uuid::Uuid::new_v4);
    let retirement = state.retirements.register(retirement_id, generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(&state),
        uds_path.clone(),
        session_dir.clone(),
        retirement,
    );
    // The current-thread runtime has not polled the reaper yet. Reusing the
    // PID models the ABA that a PID comparison alone cannot distinguish.
    insert_fake_instance_with_session_dir(&state, id, pid, session_dir);
    let replacement = state.instances.lock().unwrap().get(id).unwrap().generation;
    assert_ne!(original, replacement);
    let listener = std::os::unix::net::UnixListener::bind(&uds_path).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), reaper)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.instances.lock().unwrap().get(id).unwrap().generation, replacement);
    assert!(std::os::unix::net::UnixStream::connect(&uds_path).is_ok());
    drop(listener);
}
