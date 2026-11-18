use super::*;
use capsem_api::{
    requests::{RequestApplicationState, RequestState},
    EnvironmentCreateRequest, EnvironmentMutationResponse,
};

fn intent() -> serde_json::Value {
    serde_json::json!({"request_id": id(100), "specification": {"name": "development", "variables": {"EDITOR": "vim"}}, "reason": "create a blueprint"})
}

#[tokio::test]
async fn operator_creation_http_publishes_and_reconciles_exact_request_identity() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state.clone());
    let request = intent();
    let (status, body) = route_request(app.clone(), Method::POST, "/environments/create", Some(request.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created: EnvironmentMutationResponse = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(created.decision, RequestState::Approved);
    assert_eq!(created.application, RequestApplicationState::Applied);
    assert!(created.result.is_some());
    let (status, current) = route_request(
        app.clone(),
        Method::GET,
        &format!("/environments/{}/info", created.environment_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["document"]["specification"]["variables"]["EDITOR"], "vim");
    assert_eq!(current["request_id"], id(100));
    let (status, repeated) =
        route_request(app.clone(), Method::POST, "/environments/create", Some(request.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repeated, body);
    for field in ["reason", "name"] {
        let mut changed = request.clone();
        if field == "reason" {
            changed["reason"] = "changed intent".into();
        } else {
            changed["specification"]["name"] = "changed-intent".into();
        }
        let (status, _) = route_request(app.clone(), Method::POST, "/environments/create", Some(changed)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }
    // The durable view remains the same after conflicting retries.
    let current = state.environments.load_current(&created.environment_id).await.unwrap();
    assert_eq!(
        current.document.identity.revision_id,
        created.result.unwrap().revision_id
    );
}

#[tokio::test]
async fn creation_rejects_authority_labels_targets_and_caller_pins() {
    let app = crate::router_runtime::build_service_router(make_test_state());
    let mut cases = Vec::new();
    let mut actor = intent();
    actor["actor"] = "operator".into();
    cases.push(actor);
    let mut target = intent();
    target["requesting_sandbox"] = serde_json::json!({"session_id":id(3),"generation":id(4)});
    cases.push(target);
    let mut pin = intent();
    pin["specification"]["image"] =
        serde_json::json!({"reference":"codex-cli","resolved_digest":format!("sha256:{}", "a".repeat(64))});
    cases.push(pin);
    for request in cases {
        let (status, _) = route_request(app.clone(), Method::POST, "/environments/create", Some(request)).await;
        assert!(
            matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY),
            "{status}"
        );
    }
}

#[tokio::test]
async fn delegated_creation_cannot_borrow_operator_authority() {
    use crate::tests::OperatorRequestExt;
    let state = make_test_state();
    let (_, bearer) = delegate(&state, capsem_api::AgentOperation::Propose, vec![id(1)]);
    let app = crate::router_runtime::build_service_router(state);
    let request = axum::http::Request::post("/environments/create")
        .header("authorization", bearer)
        .header("content-type", "application/json")
        .header("x-capsem-actor", "operator")
        .body(axum::body::Body::from(serde_json::to_vec(&intent()).unwrap()))
        .unwrap();
    assert_eq!(
        app.operator_request(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn unresolved_sources_do_not_persist_an_approvable_request() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state.clone());
    let mut value = intent();
    value["specification"]["skills"] = serde_json::json!({"selected":[{"id":"example","version":"1.0.0"}]});
    let parsed: EnvironmentCreateRequest = serde_json::from_value(value.clone()).unwrap();
    parsed.validate().unwrap();
    let (status, body) = route_request(app, Method::POST, "/environments/create", Some(value)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(100))
        .await
        .unwrap()
        .is_none());
    assert!(!body.to_string().contains("vim"));
}
