use super::*;

#[tokio::test]
async fn warm_restore_disabled_without_consuming_saved_state() {
    let (state, _directory) = make_test_state_with_tempdir();
    let id = capsem_crypto::uuid_v4().to_string();
    let session = state.run_dir.join("persistent").join(&id);
    std::fs::create_dir_all(&session).unwrap();
    let checkpoint = session.join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"existing checkpoint").unwrap();
    let entry = PersistentVmEntry {
        id: id.clone(),
        suspended: true,
        checkpoint_path: Some("checkpoint.vzsave".into()),
        ..test_persistent_entry("saved", session.clone())
    };
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("saved".into(), entry);
    let error = handle_resume(State(Arc::clone(&state)), Path(id.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(error.body.error, capsem_proto::WARM_SUSPEND_UNAVAILABLE);
    assert_eq!(std::fs::read(&checkpoint).unwrap(), b"existing checkpoint");
    assert!(state
        .resume_sandbox(&id, None, None)
        .unwrap_err()
        .to_string()
        .contains("disabled"));
    assert_eq!(std::fs::read(checkpoint).unwrap(), b"existing checkpoint");
    assert!(state.persistent_registry.lock().unwrap().data.vms["saved"].suspended);
    assert!(state.instances.lock().unwrap().is_empty());
}

#[tokio::test]
async fn warm_suspend_disabled_before_owner_or_checkpoint_changes() {
    let (state, _directory) = make_test_state_with_tempdir();
    let id = capsem_crypto::uuid_v4().to_string();
    let session = state.run_dir.join("sessions").join(&id);
    std::fs::create_dir_all(&session).unwrap();
    let checkpoint = session.join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"existing checkpoint").unwrap();
    insert_fake_instance_with_session_dir(&state, &id, std::process::id(), session);
    let generation = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut(&id).unwrap();
        instance.persistent = true;
        let generation = instance.generation;
        drop(instances);
        generation
    };
    let router = build_service_router(Arc::clone(&state));
    let (status, error) = route_request(
        router.clone(),
        axum::http::Method::POST,
        &format!("/vms/{id}/pause"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{error}");
    assert_eq!(error["error"], capsem_proto::WARM_SUSPEND_UNAVAILABLE);
    assert_eq!(state.instances.lock().unwrap().get(&id).unwrap().generation, generation);
    assert_eq!(std::fs::read(checkpoint).unwrap(), b"existing checkpoint");
    let (status, inventory) = route_request(router, axum::http::Method::GET, "/vms/list", None).await;
    assert_eq!(status, StatusCode::OK);
    let row = inventory["sandboxes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap();
    assert_eq!(row["status"], "Running");
    assert!(!row["available_actions"].as_array().unwrap().contains(&json!("pause")));
}
