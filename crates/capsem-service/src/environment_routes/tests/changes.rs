use super::*;
use capsem_api::{requests::RequestApplicationState, EnvironmentMutationResponse};
use serde_json::json;

fn intent(n: u64) -> serde_json::Value {
    json!({"request_id": id(n), "base_revision_id":id(2), "specification":{"name":"development","variables":{"EDITOR":"emacs"}},"reason":"change development environment"})
}

#[tokio::test]
async fn delegated_proposal_freezes_its_original_generation_and_historical_base_for_review() {
    use crate::tests::{insert_fake_instance_with_session_dir, OperatorRequestExt};

    let state = make_test_state();
    seed(&state).await;
    let session_id = id(3);
    insert_fake_instance_with_session_dir(&state, &session_id, std::process::id(), state.run_dir.join("scope"));
    let generation = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut(&session_id).unwrap();
        let mut binding = crate::environment_launch::EnvironmentBinding::starting(
            capsem_logger::events::EnvironmentRevisionParent {
                environment_id: id(1),
                revision_id: id(2),
            },
            "acknowledged".into(),
        );
        binding.acknowledged = true;
        instance.environment = Some(binding);
        instance.generation.to_string()
    };
    let (access, bearer) = delegate_sessions(
        &state,
        capsem_api::AgentOperation::Propose,
        vec![],
        vec![capsem_api::AgentSessionScope::new(session_id.clone(), generation.clone()).unwrap()],
    );
    let app = crate::router_runtime::build_service_router(state.clone());
    let path = format!("/environments/{}/changes", id(1));
    let mut shared_edit = intent(102);
    shared_edit["specification"]["variables"]["EDITOR"] = "nano".into();
    assert_eq!(
        route_request(app.clone(), Method::POST, &path, Some(shared_edit))
            .await
            .0,
        StatusCode::OK
    );
    let committed_head = state.environments.load_current(&id(1)).await.unwrap().document;
    assert_ne!(committed_head.identity.revision_id, id(2));
    let send = |payload: serde_json::Value| {
        axum::http::Request::post(&path)
            .header("authorization", &bearer)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap()
    };
    for attempt in 0..2 {
        let response = app.clone().operator_request(send(intent(103))).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "submission {attempt}");
        let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024).await.unwrap();
        let accepted: EnvironmentMutationResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(accepted.decision, capsem_api::requests::RequestState::Pending);
        assert_eq!(accepted.application, RequestApplicationState::Pending);
        assert_eq!(accepted.result, None);
    }
    let pending = state
        .host_ledger
        .environment_request_snapshot(&id(103))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.record.actor, format!("agent:{access}"));
    assert_eq!(pending.record.base_revision_id, Some(id(2)));
    let requester = pending.record.requester.unwrap();
    assert_eq!((requester.session_id, requester.generation), (session_id, generation));
    assert!(matches!(
        *pending.payload,
        capsem_logger::events::environment::request::EnvironmentRequestPayload::Change { specification }
            if specification.variables.get("EDITOR").is_some_and(|value| value == "emacs")
    ));
    assert_eq!(
        state.environments.load_current(&id(1)).await.unwrap().document,
        committed_head
    );

    let mut different = intent(103);
    different["reason"] = "different proposal".into();
    assert_eq!(
        app.clone().operator_request(send(different)).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    let mut forged = intent(104);
    forged["requesting_sandbox"] = json!({"session_id": id(8), "generation": id(9)});
    assert_eq!(
        app.operator_request(send(forged)).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(104))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn operator_changes_publish_linear_history_and_reconcile_without_reapplying() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    let path = format!("/environments/{}/changes", id(1));
    let (status, body) = route_request(app.clone(), Method::POST, &path, Some(intent(100))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let changed: EnvironmentMutationResponse = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(changed.environment_id, id(1));
    assert_eq!(changed.application, RequestApplicationState::Applied);
    let current = state.environments.load_current(&id(1)).await.unwrap();
    assert_eq!(current.revision.previous_revision_id, Some(id(2)));
    assert_eq!(current.document.specification.variables["EDITOR"], "emacs");
    assert_eq!(
        current.document.identity.revision_id,
        changed.result.unwrap().revision_id
    );
    assert_eq!(
        route_request(app.clone(), Method::POST, &path, Some(intent(100))).await,
        (StatusCode::OK, body)
    );
    let (status, _) = route_request(app.clone(), Method::POST, &path, Some(intent(101))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(101))
        .await
        .unwrap()
        .is_none());
    let mut contradictory = intent(100);
    contradictory["reason"] = "changed meaning".into();
    assert_eq!(
        route_request(app.clone(), Method::POST, &path, Some(contradictory))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, view) = route_request(app, Method::GET, &format!("/environments/{}/info", id(1)), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["document"]["specification"]["variables"]["EDITOR"], "emacs");
}

#[tokio::test]
async fn changes_reject_forged_authority_requesters_and_self_attested_pins() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    let path = format!("/environments/{}/changes", id(1));
    for field in ["actor", "requesting_sandbox", "pin"] {
        let mut value = intent(100);
        match field {
            "actor" => value["actor"] = "operator".into(),
            "requesting_sandbox" => value["requesting_sandbox"] = json!({"session_id":id(3),"generation":id(4)}),
            _ => {
                value["specification"]["image"] =
                    json!({"reference":"development:stable","resolved_digest":format!("sha256:{}","a".repeat(64))})
            }
        }
        let (status, body) = route_request(app.clone(), Method::POST, &path, Some(value)).await;
        assert!(
            matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY),
            "{status}: {body}"
        );
        assert!(!body.to_string().contains("emacs"));
    }
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(100))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn changes_distinguish_malformed_missing_and_untrusted_callers() {
    use crate::tests::OperatorRequestExt;
    let state = make_test_state();
    seed(&state).await;
    let (_, bearer) = delegate(&state, capsem_api::AgentOperation::Propose, vec![id(1)]);
    let app = crate::router_runtime::build_service_router(state);
    for (id, expected) in [
        ("not-a-uuid".to_string(), StatusCode::BAD_REQUEST),
        (id(99), StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            route_request(
                app.clone(),
                Method::POST,
                &format!("/environments/{id}/changes"),
                Some(intent(100))
            )
            .await
            .0,
            expected
        );
    }
    let request = axum::http::Request::post(format!("/environments/{}/changes", id(1)))
        .header("authorization", bearer)
        .header("content-type", "application/json")
        .header("x-capsem-actor", "operator")
        .body(axum::body::Body::from(serde_json::to_vec(&intent(100)).unwrap()))
        .unwrap();
    assert_eq!(
        app.operator_request(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}
