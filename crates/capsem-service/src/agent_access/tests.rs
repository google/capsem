use crate::tests::{make_test_state, route_request};
use axum::http::{Method, StatusCode};
use std::sync::Arc;

fn request(id: &str) -> capsem_api::CreateAgentAccessRequest {
    capsem_api::CreateAgentAccessRequest {
        request_id: id.into(),
        ownership_token: capsem_api::ManagedOwnershipToken::new("11".repeat(32)).unwrap(),
        scope: capsem_api::AgentAccessScope::new(vec![capsem_api::AgentOperation::Discover], vec![], vec![]).unwrap(),
        lease_seconds: 60,
    }
}
fn headers(id: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {id}.{}", "11".repeat(32)).parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn operator_issuance_is_audited_retry_stable_and_revocable() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state.clone());
    let id = capsem_crypto::uuid_v4().to_string();
    let body = serde_json::to_value(request(&id)).unwrap();
    let (status, issued) = route_request(app.clone(), Method::POST, "/agent-access", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let (status, retried) = route_request(app.clone(), Method::POST, "/agent-access", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    assert_eq!(issued, retried, "retry must never renew or broaden authority");
    assert_eq!(issued["access"]["id"], id);
    assert!(!issued.to_string().contains(&"11".repeat(32)));
    let events = state.host_ledger.host_events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, capsem_logger::HostEventKind::GrantMinted);
    assert_eq!(events[0].actor, "operator");
    assert!(!events[0].detail.windows(32).any(|bytes| bytes == [0x11; 32]));
    assert!(!events[0]
        .detail
        .windows(64)
        .any(|bytes| bytes == "11".repeat(32).as_bytes()));
    assert!(state.caller_authority.authenticate_gateway(&headers(&id)).is_ok());
    let (status, _) = route_request(app.clone(), Method::DELETE, &format!("/agent-access/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.caller_authority.authenticate_gateway(&headers(&id)).is_err());
    let events = state.host_ledger.host_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, capsem_logger::HostEventKind::GrantRevoked);
    assert_eq!(events[1].actor, "operator");
    assert_eq!(
        route_request(
            app,
            Method::POST,
            "/agent-access",
            Some(serde_json::to_value(request(&id)).unwrap())
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn issuance_refuses_changed_capability_scope_and_invalid_body() {
    use crate::tests::OperatorRequestExt;
    use axum::{body::Body, http::Request};
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state.clone());
    let id = capsem_crypto::uuid_v4().to_string();
    let original = serde_json::to_value(request(&id)).unwrap();
    assert_eq!(
        route_request(app.clone(), Method::POST, "/agent-access", Some(original.clone()))
            .await
            .0,
        StatusCode::OK
    );
    let mut changed = original.clone();
    changed["ownership_token"] = "22".repeat(32).into();
    assert_eq!(
        route_request(app.clone(), Method::POST, "/agent-access", Some(changed))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut changed = original.clone();
    changed["scope"]["operations"] = serde_json::to_value([capsem_api::AgentOperation::Inspect]).unwrap();
    assert_eq!(
        route_request(app.clone(), Method::POST, "/agent-access", Some(changed))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut changed = original.clone();
    changed["lease_seconds"] = 0.into();
    assert_eq!(
        route_request(app.clone(), Method::POST, "/agent-access", Some(changed))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let oversized = Request::post("/agent-access")
        .header("content-type", "application/json")
        .body(Body::from(vec![b' '; capsem_proto::AGENT_ACCESS_MAX_REQUEST_BYTES + 1]))
        .unwrap();
    assert_eq!(
        app.clone().operator_request(oversized).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let delegated = Request::post("/agent-access")
        .header("authorization", format!("Bearer {id}.{}", "11".repeat(32)))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&original).unwrap()))
        .unwrap();
    assert_eq!(
        app.operator_request(delegated).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let events = state.host_ledger.host_events().await.unwrap();
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        [
            capsem_logger::HostEventKind::GrantMinted,
            capsem_logger::HostEventKind::AccessDenied,
        ]
    );
    assert_eq!(events[1].actor, format!("agent:{id}"));
}

#[tokio::test]
async fn a_new_service_epoch_cannot_reissue_a_historical_access_id() {
    let state = make_test_state();
    let id = capsem_crypto::uuid_v4().to_string();
    let body = serde_json::to_value(request(&id)).unwrap();
    let app = crate::router_runtime::build_service_router(state.clone());
    assert_eq!(
        route_request(app.clone(), Method::POST, "/agent-access", Some(body.clone()))
            .await
            .0,
        StatusCode::OK
    );
    drop(app);
    // A fresh service owns fresh authority but reads the same durable ledger.
    // The old HTTP driver's last Arc is not a service-restart synchronization primitive.
    let mut next = crate::tests::make_test_state_owned();
    next.host_ledger = Arc::clone(&state.host_ledger);
    let state = Arc::new(next);
    assert!(state.caller_authority.authenticate_gateway(&headers(&id)).is_err());
    let app = crate::router_runtime::build_service_router(state.clone());
    assert_eq!(
        route_request(app, Method::POST, "/agent-access", Some(body)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(state.host_ledger.host_events().await.unwrap().len(), 1);
}

#[tokio::test]
async fn audit_failure_never_activates_reserved_authority() {
    let mut state = crate::tests::make_test_state_owned();
    state.host_ledger = Arc::new(capsem_logger::DbHandle::open_external_reader(state.host_ledger.path()).unwrap());
    let state = Arc::new(state);
    let app = crate::router_runtime::build_service_router(state.clone());
    let id = capsem_crypto::uuid_v4().to_string();
    let body = serde_json::to_value(request(&id)).unwrap();
    assert_eq!(
        route_request(app, Method::POST, "/agent-access", Some(body)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(state.caller_authority.authenticate_gateway(&headers(&id)).is_err());
    assert!(state.host_ledger.host_events().await.unwrap().is_empty());
}
