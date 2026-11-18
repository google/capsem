use super::*;
use crate::tests::{
    install_test_runtime_assets, make_test_state_with_tempdir, route_request, test_persistent_entry,
    test_provision_options,
};

fn stale_source() -> (Arc<ServiceState>, tempfile::TempDir, crate::CloneFrom) {
    let (state, directory) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    let session = state.run_dir.join("persistent/source");
    std::fs::create_dir_all(&session).unwrap();
    let mut entry = test_persistent_entry("source", session);
    // Keep the pre-regression path safely before any real process spawn.
    entry.legacy_profile_id = Some("legacy".into());
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let source = crate::CloneFrom {
        source: "source".into(),
        replace_image: false,
        cancel: None,
        expected: Some(capsem_api::VmLifecycleSelection {
            id,
            generation: Some("00000000-0000-4000-8000-000000000009".into()),
            running: false,
            environment: None,
        }),
    };
    (state, directory, source)
}

#[tokio::test]
async fn managed_create_from_refuses_stale_source_before_allocating_ownership() {
    let (state, _directory, from) = stale_source();
    let request_id = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";
    let (status, body) = route_request(
        crate::build_service_router(state.clone()),
        axum::http::Method::POST,
        "/vms/create",
        Some(serde_json::json!({"from":from.source,"expected_source":from.expected,
            "managed":{"request_id":request_id,"ownership_token":"af".repeat(32)}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!state
        .run_dir
        .join("managed-ownership")
        .join(format!("{request_id}.json"))
        .exists());
    assert!(state.instances.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_managed_source_producer_rechecks_cancellation_after_selection_wait() {
    let (state, _directory, mut from) = stale_source();
    from.expected = None;
    let cancel = tokio_util::sync::CancellationToken::new();
    from.cancel = Some([cancel.clone(), tokio_util::sync::CancellationToken::new()]);
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let producer_state = state.clone();
    let producer = tokio::task::spawn_blocking(move || {
        producer_state.provision_sandbox(ProvisionOptions {
            from: Some(from),
            ..test_provision_options("clone-target", "clone-target")
        })
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    cancel.cancel();
    drop(mutation);
    let error = producer.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(!state.run_dir.join("sessions/clone-target").exists());
    assert!(state.instances.lock().unwrap().is_empty());
}

#[test]
fn clone_source_expectation_is_consumed_by_the_actual_producer_before_target_allocation() {
    let (state, _directory, from) = stale_source();
    let failure = state
        .provision_sandbox(ProvisionOptions {
            from: Some(from),
            ..test_provision_options("clone-target", "clone-target")
        })
        .unwrap_err();
    let refusal = failure.downcast_ref::<crate::lifecycle_readiness::SelectionRefusal>();
    assert!(
        refusal.is_some(),
        "source expectation must be compared before ordinary source validation: {failure}"
    );
    assert_eq!(refusal.unwrap().0.status, StatusCode::CONFLICT);
    assert!(!state.run_dir.join("sessions/clone-target").exists());
    assert!(state.instances.lock().unwrap().is_empty());
}

#[test]
fn source_identity_and_each_expectation_field_are_matched_before_legacy_validation() {
    for by_id in [false, true] {
        for field in ["match", "generation", "running", "environment", "identity", "alias"] {
            let (state, _directory, mut from) = stale_source();
            let expected = from.expected.as_mut().unwrap();
            expected.generation = None;
            if by_id {
                from.source = expected.id.clone();
            }
            match field {
                "generation" => expected.generation = Some(capsem_crypto::uuid_v4().to_string()),
                "running" => {
                    expected.running = true;
                    expected.generation = Some(capsem_crypto::uuid_v4().to_string());
                }
                "environment" => {
                    expected.environment = Some(capsem_api::environments::EnvironmentRevisionReference {
                        environment_id: capsem_crypto::uuid_v4().to_string(),
                        revision_id: capsem_crypto::uuid_v4().to_string(),
                    })
                }
                "identity" => expected.id = capsem_crypto::uuid_v4().to_string(),
                "alias" => {
                    let mut registry = state.persistent_registry.lock().unwrap();
                    let mut entry = registry.data.vms.remove("source").unwrap();
                    entry.name = "renamed".into();
                    registry.data.vms.insert(entry.name.clone(), entry);
                }
                _ => {}
            }
            let error = state
                .provision_sandbox(ProvisionOptions {
                    from: Some(from),
                    ..test_provision_options("clone-target", "clone-target")
                })
                .unwrap_err();
            if field == "match" || (field == "alias" && by_id) {
                assert!(
                    error.to_string().contains("profiles no longer exist"),
                    "{by_id}/{field}: {error}"
                );
            } else {
                let refusal = error
                    .downcast_ref::<crate::lifecycle_readiness::SelectionRefusal>()
                    .unwrap();
                assert_eq!(refusal.0.status, StatusCode::CONFLICT, "{by_id}/{field}: {error}");
            }
            assert!(!state.run_dir.join("sessions/clone-target").exists());
        }
    }
}

#[tokio::test]
async fn create_source_revision_is_rechecked_after_the_producer_waits_for_selection() {
    let (state, _directory, mut from) = stale_source();
    from.expected.as_mut().unwrap().generation = None;
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let app = crate::build_service_router(state.clone());
    let request = tokio::spawn(async move {
        route_request(
            app,
            axum::http::Method::POST,
            "/vms/create",
            Some(serde_json::json!({
                "name":"clone-target", "from":from.source, "expected_source":from.expected
            })),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .get_mut("source")
        .unwrap()
        .environment = Some(capsem_logger::events::EnvironmentRevisionParent {
        environment_id: capsem_crypto::uuid_v4().to_string(),
        revision_id: capsem_crypto::uuid_v4().to_string(),
    });
    drop(mutation);
    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(state.persistent_registry.lock().unwrap().get("clone-target").is_none());
    assert!(state.instances.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_create_http_waiter_keeps_native_exclusion_until_producer_joins() {
    let (state, _directory, mut from) = stale_source();
    from.expected.as_mut().unwrap().generation = None;
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let app = crate::build_service_router(state.clone());
    let request = tokio::spawn(async move {
        route_request(
            app,
            axum::http::Method::POST,
            "/vms/create",
            Some(serde_json::json!({
                "name":"clone-target", "from":from.source, "expected_source":from.expected
            })),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        state.lifecycle.vz.try_write().is_err(),
        "cancelled waiter dropped native exclusion"
    );
    drop(mutation);
    tokio::time::timeout(std::time::Duration::from_secs(5), state.lifecycle.wait_for_launches())
        .await
        .unwrap();
    assert!(state.lifecycle.vz.try_write().is_ok());
    assert!(state.persistent_registry.lock().unwrap().get("clone-target").is_none());
    assert!(state.instances.lock().unwrap().is_empty());
}

#[tokio::test]
async fn managed_http_close_joins_a_source_producer_queued_on_selection() {
    let (state, _directory, mut from) = stale_source();
    from.expected.as_mut().unwrap().generation = None;
    let request_id = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";
    let token = "af".repeat(32);
    let mutation = state.policy_mutation.begin().await;
    let contended = state.policy_mutation.contended.notified();
    let app = crate::build_service_router(state.clone());
    let create_app = app.clone();
    let create_token = token.clone();
    let create = tokio::spawn(async move {
        route_request(
            create_app,
            axum::http::Method::POST,
            "/vms/create",
            Some(serde_json::json!({
                "from":from.source, "expected_source":from.expected,
                "managed":{"request_id":request_id,"ownership_token":create_token}
            })),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), contended)
        .await
        .unwrap();
    let close_app = app.clone();
    let close_token = token.clone();
    let close = tokio::spawn(async move {
        route_request(
            close_app,
            axum::http::Method::POST,
            &format!("/vms/managed/{request_id}/close"),
            Some(serde_json::json!({"ownership_token":close_token,"outcome":"cancelled"})),
        )
        .await
    });
    // Observe the real controller's close edge while the owned producer is
    // still blocked. Closed cannot be published until that producer joins.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (status, claim) = route_request(
                app.clone(),
                axum::http::Method::POST,
                &format!("/vms/managed/{request_id}/claim"),
                Some(serde_json::json!({"ownership_token":token})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{claim}");
            if claim["state"] == "closing" {
                break;
            }
            assert_eq!(claim["state"], "pending", "{claim}");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!close.is_finished());
    assert!(state.lifecycle.vz.try_write().is_err());
    drop(mutation);
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(5), close)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "closed");
    let (status, body) = create.await.unwrap();
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(state.lifecycle.vz.try_write().is_ok());
    assert!(state.instances.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(state.run_dir.join("sessions")).unwrap().count(), 0);
    assert!(state.persistent_registry.lock().unwrap().get("source").is_some());
}

#[tokio::test]
async fn reclaiming_a_stale_source_requires_a_new_source_expectation() {
    let (state, _directory, mut from) = stale_source();
    let id = from.expected.as_ref().unwrap().id.clone();
    let session = state
        .persistent_registry
        .lock()
        .unwrap()
        .get("source")
        .unwrap()
        .session_dir
        .clone();
    let mut child = tokio::process::Command::new("true").kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    child.wait().await.unwrap();
    crate::tests::insert_fake_instance_with_session_dir(&state, &id, pid, session);
    state.instances.lock().unwrap().get_mut(&id).unwrap().persistent = true;
    from.expected = Some(
        state
            .lifecycle_selection(&from.source, capsem_api::VmLifecycleIntent::Clone)
            .unwrap(),
    );
    let target = id.clone();
    // A colliding stale target forces the same reclamation used at capacity.
    // Its source owner disappears between the first and second comparisons.
    let error = state
        .off_worker(move |state| {
            state.provision_sandbox(ProvisionOptions {
                from: Some(from),
                ..test_provision_options(&target, "clone-target")
            })
        })
        .await
        .unwrap()
        .unwrap_err();
    let refusal = error
        .downcast_ref::<crate::lifecycle_readiness::SelectionRefusal>()
        .unwrap();
    assert_eq!(refusal.0.status, StatusCode::CONFLICT);
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(!state.run_dir.join("sessions").join(id).exists());
}

#[tokio::test]
async fn create_from_http_refuses_stale_source_before_launching_or_registering_a_target() {
    let (state, _directory, from) = stale_source();
    let app = crate::build_service_router(state.clone());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/vms/create",
        Some(serde_json::json!({
            "name":"clone-target", "from":from.source, "expected_source":from.expected,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(state.persistent_registry.lock().unwrap().get("clone-target").is_none());
    assert!(state.instances.lock().unwrap().is_empty());
}
