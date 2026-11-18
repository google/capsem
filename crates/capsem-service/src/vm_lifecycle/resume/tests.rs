use super::*;
use crate::tests::{make_test_state_with_tempdir, route_request, test_persistent_entry};

fn source(defunct: bool) -> (Arc<ServiceState>, tempfile::TempDir, String, PathBuf) {
    let (state, directory) = make_test_state_with_tempdir();
    let mut entry = test_persistent_entry("saved-source", PathBuf::new());
    let id = entry.id.clone();
    let session = state.run_dir.join("persistent").join(&id);
    std::fs::create_dir_all(&session).unwrap();
    entry.session_dir = session.clone();
    entry.defunct = defunct;
    entry.last_error = defunct.then(|| "prior boot failed".into());
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    (state, directory, id, session)
}

fn expectation(id: &str) -> serde_json::Value {
    serde_json::json!({"expected":{"id":id, "generation":null, "running":false, "environment":null}})
}

#[tokio::test]
async fn stale_start_and_resume_expectations_refuse_before_saved_state_changes() {
    let (state, _directory) = make_test_state_with_tempdir();
    let session = state.run_dir.join("persistent/stale-source");
    std::fs::create_dir_all(&session).unwrap();
    let mut entry = test_persistent_entry("saved-source", session.clone());
    entry.defunct = true;
    entry.last_error = Some("prior boot failed".into());
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let uds = state.instance_socket_path(&id).unwrap();
    std::fs::create_dir_all(uds.parent().unwrap()).unwrap();
    std::fs::write(&uds, b"previous socket").unwrap();
    std::fs::write(uds.with_extension("ready"), b"previous sentinel").unwrap();
    let app = crate::build_service_router(state.clone());
    for action in ["start", "resume"] {
        let (status, body) = route_request(
            app.clone(),
            axum::http::Method::POST,
            &format!("/vms/{id}/{action}"),
            Some(serde_json::json!({"expected":{"id":id,
                "generation":"00000000-0000-4000-8000-000000000009", "running":false, "environment":null}})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(std::fs::read(&uds).unwrap(), b"previous socket");
        assert_eq!(
            std::fs::read(uds.with_extension("ready")).unwrap(),
            b"previous sentinel"
        );
        assert!(!session.join("vm/active_policy.toml").exists());
        assert!(state.instances.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn resume_failure_does_not_report_an_existing_session_as_missing() {
    let (state, _directory) = make_test_state_with_tempdir();
    let entry = test_persistent_entry("unresumable", state.run_dir.join("missing-session-directory"));
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let router = crate::build_service_router(state);
    let (status, error) = route_request(
        router.clone(),
        axum::http::Method::POST,
        &format!("/vms/{id}/resume"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert!(error["error"].as_str().unwrap().contains("resume failed"));
    assert_ne!(error["code"], serde_json::to_value(ErrorCode::VmNotFound).unwrap());
    let (listed, inventory) = route_request(router.clone(), axum::http::Method::GET, "/vms/list", None).await;
    assert_eq!(listed, StatusCode::OK, "{inventory}");
    assert!(inventory["sandboxes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == id));

    let missing = capsem_crypto::uuid_v4();
    let (status, error) = route_request(
        router,
        axum::http::Method::POST,
        &format!("/vms/{missing}/resume"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["code"], serde_json::to_value(ErrorCode::VmNotFound).unwrap());
}

#[tokio::test]
async fn empty_legacy_requests_remain_valid_and_invalid_expectations_never_enter_launch() {
    use crate::tests::OperatorRequestExt;
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    let (state, _directory, id, session) = source(true);
    let app = crate::build_service_router(state.clone());
    for action in ["start", "resume"] {
        let path = format!("/vms/{id}/{action}");
        for body in [None, Some(serde_json::json!({})), Some(expectation(&id))] {
            let (status, body) = route_request(app.clone(), Method::POST, &path, body).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
            assert!(body["error"].as_str().unwrap().contains("prior boot failed"));
        }
        for content_type in ["application/json", "application/octet-stream"] {
            let response = app
                .clone()
                .operator_request(
                    Request::post(&path)
                        .header("content-type", content_type)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        let mut wrong_target = expectation("different-vm");
        wrong_target["expected"]["generation"] = serde_json::json!("00000000-0000-4000-8000-000000000009");
        let mut invalid_generation = expectation(&id);
        invalid_generation["expected"]["generation"] = serde_json::json!(uuid::Uuid::nil());
        let mut active_without_generation = expectation(&id);
        active_without_generation["expected"]["running"] = serde_json::json!(true);
        for body in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({"actor":"operator"}),
            serde_json::json!({"startup_timeout_seconds":1}),
            wrong_target,
            invalid_generation,
            active_without_generation,
        ] {
            let (status, response) = route_request(app.clone(), Method::POST, &path, Some(body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "request={body} response={response}");
        }
    }
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!session.join("spawn-identity.json").exists());
    assert!(!session.join("vm/active_policy.toml").exists());
}

#[tokio::test]
async fn queued_resume_compares_the_selection_after_policy_exclusion_is_acquired() {
    let (state, _directory, id, session) = source(true);
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let body = expectation(&id);
    let app = crate::build_service_router(state.clone());
    let request = tokio::spawn(async move {
        route_request(app, axum::http::Method::POST, &format!("/vms/{id}/resume"), Some(body)).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .get_mut("saved-source")
        .unwrap()
        .environment = Some(capsem_logger::events::EnvironmentRevisionParent {
        environment_id: "00000000-0000-4000-8000-000000000001".into(),
        revision_id: "00000000-0000-4000-8000-000000000002".into(),
    });
    drop(mutation);
    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!session.join("vm/active_policy.toml").exists());
    assert!(!session.join("spawn-identity.json").exists());
}

#[tokio::test]
async fn cancelling_http_waiter_cannot_release_the_blocking_resume_producers_native_exclusion() {
    let (state, _directory, id, session) = source(true);
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let mut body = expectation(&id);
    body["expected"]["generation"] = serde_json::json!("00000000-0000-4000-8000-000000000009");
    let request_state = state.clone();
    let request = tokio::spawn(async move {
        handle_resume(
            crate::tests::operator_actor(),
            State(request_state),
            Path(id),
            axum::body::Bytes::from(serde_json::to_vec(&body).unwrap()),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        state.lifecycle.vz.try_read().is_err(),
        "HTTP cancellation cannot release native producer exclusion"
    );
    drop(mutation);
    tokio::time::timeout(std::time::Duration::from_secs(5), state.lifecycle.wait_for_launches())
        .await
        .unwrap();
    assert!(state.lifecycle.vz.try_read().is_ok());
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!session.join("spawn-identity.json").exists());
}

#[tokio::test]
async fn stale_owner_reconciliation_is_followed_by_a_second_expectation_check() {
    use crate::tests::insert_fake_instance_with_session_dir;
    let (state, _directory, id, session) = source(false);
    let mut child = tokio::process::Command::new("true").kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    child.wait().await.unwrap();
    insert_fake_instance_with_session_dir(&state, &id, pid, session.clone());
    state.instances.lock().unwrap().get_mut(&id).unwrap().persistent = true;
    let expected = state
        .lifecycle_selection(&id, capsem_api::VmLifecycleIntent::Start)
        .unwrap();
    let failure = state
        .off_worker(move |state| state.resume_sandbox(&id, None, None, Some(&expected)))
        .await
        .unwrap()
        .unwrap_err()
        .downcast::<crate::lifecycle_readiness::SelectionRefusal>()
        .unwrap();
    assert_eq!(failure.0.status, StatusCode::CONFLICT);
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!session.join("spawn-identity.json").exists());
    assert!(!session.join("vm/active_policy.toml").exists());
}

#[tokio::test]
async fn native_restore_exclusion_ends_before_workload_preparation_settles() {
    for completion in [Completion::Original, Completion::Replaced, Completion::Ended] {
        native_restore_with_completion(completion).await;
    }
}

#[derive(Clone, Copy)]
enum Completion {
    Original,
    Replaced,
    Ended,
}

async fn native_restore_with_completion(completion: Completion) {
    use crate::tests::{insert_fake_instance_with_session_dir, EnvVarGuard, SETTINGS_ENV_LOCK};
    use capsem_core::container::{STAGE, STAGE_PREPARING, STAGE_RUNNING};
    let _settings = SETTINGS_ENV_LOCK.lock().await;
    let (state, directory, id, session) = source(false);
    let _store = EnvVarGuard::set(
        "CAPSEM_CREDENTIAL_STORE_PATH",
        directory.path().join("credentials.json"),
    );
    capsem_credentials::CredentialStore::global().clear_for_test();
    insert_fake_instance_with_session_dir(&state, &id, std::process::id(), session.clone());
    let uds = state.instance_socket_path(&id).unwrap();
    {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut(&id).unwrap();
        instance.uds_path = uds.clone();
        instance.persistent = true;
        drop(instances);
    }
    std::fs::write(uds.with_extension("ready"), b"ready\n").unwrap();
    let stage = session.join("guest/workspace").join(STAGE);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join(STAGE_PREPARING), b"preparing\n").unwrap();
    let generation = state.containers.stage_for_tests(&id, "registry.example/app:1");
    assert_eq!(state.containers.claim_attach(&id).unwrap(), generation);
    let expected = state
        .lifecycle_selection(&id, capsem_api::VmLifecycleIntent::Start)
        .unwrap();
    let body = serde_json::to_vec(&capsem_api::VmStartRequest {
        expected: Some(expected),
    })
    .unwrap();
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let request_state = state.clone();
    let request_id = id.clone();
    let request = tokio::spawn(async move {
        handle_resume(
            crate::tests::operator_actor(),
            State(request_state),
            Path(request_id),
            axum::body::Bytes::from(body),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    assert!(state.lifecycle.vz.try_read().is_err());
    drop(mutation);
    let read = tokio::time::timeout(std::time::Duration::from_secs(5), state.lifecycle.vz.read())
        .await
        .unwrap();
    let host = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        acquire_vz_host_lock(startup::VzHostLockMode::Shared),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !request.is_finished(),
        "pending workload preparation remains owned after native restore exclusion ends"
    );
    drop(host);
    drop(read);
    match completion {
        Completion::Original => {}
        Completion::Replaced => {
            state.instances.lock().unwrap().get_mut(&id).unwrap().generation = capsem_crypto::uuid_v4()
        }
        Completion::Ended => {
            state.instances.lock().unwrap().remove(&id);
        }
    }
    if !matches!(completion, Completion::Ended) {
        std::fs::write(stage.join(STAGE_RUNNING), b"running\n").unwrap();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap();
    match completion {
        Completion::Original => assert_eq!(result.unwrap().0.id, id),
        Completion::Replaced | Completion::Ended => assert_eq!(result.unwrap_err().status, StatusCode::CONFLICT),
    }
    state.containers.cancel_and_wait(&id).await;
    state.instances.lock().unwrap().remove(&id);
    capsem_credentials::CredentialStore::global().clear_for_test();
}

#[tokio::test]
async fn failed_resume_cleanup_cannot_retire_a_replacement_generation() {
    use crate::tests::{insert_fake_instance_with_session_dir, test_instance};
    let (state, _directory, id, session) = source(false);
    let child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let generation = capsem_crypto::uuid_v4();
    insert_fake_instance_with_session_dir(&state, &id, child.id().unwrap(), session.clone());
    state.instances.lock().unwrap().get_mut(&id).unwrap().generation = generation;
    let uds = state.instances.lock().unwrap()[&id].uds_path.clone();
    std::fs::write(&uds, b"replacement socket").unwrap();
    let retirement = state.retirements.register(&id, generation).unwrap();
    let guard = state.lifecycle.vz.write().await;
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        id.clone(),
        "saved-source".into(),
        state.clone(),
        uds.clone(),
        session,
        retirement,
    );
    let old_generation = test_instance().generation;
    assert_ne!(old_generation, generation);
    let error = stop_failed_resume_process_under_lock(&state, &id, old_generation)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.body.error.contains("generation changed"));
    assert!(state.retirements.is_live(&id, generation).unwrap());
    assert_eq!(std::fs::read(&uds).unwrap(), b"replacement socket");
    signal_vm_process(&state, &id, generation, process_control::Signal::Terminate)
        .await
        .unwrap();
    drop(guard);
    tokio::time::timeout(std::time::Duration::from_secs(5), reaper)
        .await
        .unwrap()
        .unwrap();
}
