//! Stop success is a completed ephemeral cleanup, not a detached task.

use super::*;

fn insert_owned_instance(state: &Arc<ServiceState>, id: &str, session_dir: PathBuf) -> tokio::task::JoinHandle<()> {
    let child = crate::tests::isolated_test_command("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    insert_fake_instance_with_session_dir(state, id, child.id().unwrap(), session_dir.clone());
    let generation = state.instances.lock().unwrap().get(id).unwrap().generation;
    let retirement = state.retirements.register(id, generation).unwrap();
    crate::instance_reaper::spawn_exit_reaper(
        child,
        id.into(),
        id.into(),
        Arc::clone(state),
        state.instance_socket_path(id).unwrap(),
        session_dir,
        retirement,
    )
}

#[tokio::test]
async fn ephemeral_stop_propagates_session_directory_cleanup_failure() {
    let (state, _dir) = make_test_state_with_tempdir();
    let id = "stop-cleanup-failure";
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(session_dir.parent().unwrap()).unwrap();
    // A real filesystem error, independent of scheduling and effective uid:
    // this session's directory has been replaced by a regular file.
    std::fs::write(&session_dir, b"unremoved session state").unwrap();
    let reaper = insert_owned_instance(&state, id, session_dir.clone());

    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::POST,
        &format!("/vms/{id}/stop"),
        None,
    )
    .await;
    reaper.await.unwrap();

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "stop must not acknowledge ephemeral cleanup whose filesystem deletion failed: {body}"
    );
    assert!(body["error"].as_str().unwrap().contains("cleanup"), "{body}");
    assert_eq!(std::fs::read(&session_dir).unwrap(), b"unremoved session state");
    assert!(
        !state.instances.lock().unwrap().contains_key(id),
        "the process was stopped"
    );
}

#[tokio::test]
async fn ephemeral_stop_uses_the_session_deletion_owner() {
    let (state, dir) = make_test_state_with_tempdir();
    let id = "stop-cleanup-link";
    let outside = dir.path().join("outside-session");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("marker"), b"keep").unwrap();
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(session_dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &session_dir).unwrap();
    let reaper = insert_owned_instance(&state, id, session_dir.clone());

    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::POST,
        &format!("/vms/{id}/stop"),
        None,
    )
    .await;
    reaper.await.unwrap();

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(std::fs::symlink_metadata(&session_dir)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(outside.join("marker")).unwrap(), b"keep");
}

#[tokio::test]
async fn stop_success_removes_ephemeral_state_and_preserves_persistent_state() {
    let (state, _dir) = make_test_state_with_tempdir();
    for persistent in [false, true] {
        let id = if persistent {
            "stop-keep-state"
        } else {
            "stop-remove-state"
        };
        let session_dir = state.run_dir.join("sessions").join(id);
        std::fs::create_dir_all(session_dir.join("guest")).unwrap();
        std::fs::write(session_dir.join("guest/marker"), b"session state").unwrap();
        let reaper = insert_owned_instance(&state, id, session_dir.clone());
        state.instances.lock().unwrap().get_mut(id).unwrap().persistent = persistent;

        let (status, body) = route_request(
            build_service_router(Arc::clone(&state)),
            axum::http::Method::POST,
            &format!("/vms/{id}/stop"),
            None,
        )
        .await;
        reaper.await.unwrap();

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({"success": true, "persistent": persistent}));
        assert_eq!(
            session_dir.exists(),
            persistent,
            "stop success must reflect completed cleanup"
        );
        if persistent {
            assert_eq!(
                std::fs::read(session_dir.join("guest/marker")).unwrap(),
                b"session state"
            );
        }
    }
}
