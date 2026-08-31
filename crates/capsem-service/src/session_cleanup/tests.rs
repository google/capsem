use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_state_in, route_request};

#[tokio::test]
async fn failed_run_preservation_noops_when_watcher_wins_after_shutdown_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("code-1");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"child exited with code 1").unwrap();
    insert_fake_instance_with_session_dir(&state, "code-1", 42, session_dir.clone());

    let shutdown_snapshot = {
        let instances = state.instances.lock().unwrap();
        let info = instances.get("code-1").unwrap();
        (info.session_dir.clone(), info.persistent, info.pid)
    };
    let watcher_state = Arc::clone(&state);
    let watcher_result = tokio::task::spawn_blocking(move || {
        let info = watcher_state
            .instances
            .lock()
            .unwrap()
            .remove("code-1")
            .expect("watcher should win the instance ownership race");
        watcher_state
            .preserve_failed_session_dir(&info.session_dir, "code-1")
            .expect("child watcher should preserve the failed session")
    })
    .await
    .unwrap();

    let shutdown_claimed = claim_shutdown_instance(&state, "code-1");
    assert!(
        !shutdown_claimed,
        "a stale snapshot is not shutdown ownership"
    );
    let result = preserve_failed_run_shutdown_result(
        Arc::clone(&state),
        "code-1".to_string(),
        shutdown_claimed.then_some(shutdown_snapshot),
    )
    .await
    .unwrap();

    assert!(result.is_none());
    assert_eq!(
        find_failed_session_dir(&state.run_dir, "code-1"),
        Some(watcher_result.clone())
    );
    assert_eq!(
        std::fs::read_dir(state.run_dir.join("sessions"))
            .unwrap()
            .count(),
        1,
        "the provisioning owner must not create a second preservation result"
    );
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&watcher_result.join("process.log"), usize::MAX)
            .unwrap(),
        "child exited with code 1"
    );
}

#[tokio::test]
async fn failed_run_preservation_uses_shutdown_owned_instance_once() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("code-1");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"ready wait failed").unwrap();

    let preserved = preserve_failed_run_shutdown_result(
        Arc::clone(&state),
        "code-1".to_string(),
        Some((session_dir.clone(), false, 42)),
    )
    .await
    .unwrap()
    .expect("the provisioning owner must preserve its claimed session dir");

    assert!(!session_dir.exists());
    assert_eq!(
        find_failed_session_dir(&state.run_dir, "code-1"),
        Some(preserved.clone())
    );
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved.join("process.log"), usize::MAX).unwrap(),
        "ready wait failed"
    );
}

#[tokio::test]
async fn failed_session_route_preserves_runtime_ipc_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let id = "code-doctor-ipc";
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(
        session_dir.join("process.log"),
        b"process channel closed during doctor",
    )
    .unwrap();
    insert_fake_instance_with_session_dir(&state, id, 0, session_dir.clone());

    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::POST,
        &format!("/vms/{id}/preserve-failure"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert!(!session_dir.exists());
    let preserved = find_failed_session_dir(&state.run_dir, id)
        .expect("unexpected doctor IPC loss must retain the process evidence");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved.join("process.log"), usize::MAX).unwrap(),
        "process channel closed during doctor"
    );
}

#[tokio::test]
async fn one_shot_completion_deletes_success_and_preserves_ipc_failure() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());

    let clean_id = "code-run-clean";
    let clean_dir = state.run_dir.join("sessions").join(clean_id);
    std::fs::create_dir_all(&clean_dir).unwrap();
    std::fs::write(clean_dir.join("process.log"), b"command completed").unwrap();
    finalize_one_shot_session(
        Arc::clone(&state),
        clean_id.to_string(),
        Some((clean_dir.clone(), false, 0)),
        false,
    )
    .await
    .unwrap();
    assert!(
        !clean_dir.exists(),
        "successful one-shot state must be deleted"
    );

    let failed_id = "code-run-ipc";
    let failed_dir = state.run_dir.join("sessions").join(failed_id);
    std::fs::create_dir_all(&failed_dir).unwrap();
    std::fs::write(failed_dir.join("process.log"), b"exec IPC closed").unwrap();
    finalize_one_shot_session(
        Arc::clone(&state),
        failed_id.to_string(),
        Some((failed_dir.clone(), false, 0)),
        true,
    )
    .await
    .unwrap();
    assert!(!failed_dir.exists());
    let preserved = find_failed_session_dir(&state.run_dir, failed_id)
        .expect("one-shot exec IPC loss must retain the process evidence");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved.join("process.log"), usize::MAX).unwrap(),
        "exec IPC closed"
    );
}
