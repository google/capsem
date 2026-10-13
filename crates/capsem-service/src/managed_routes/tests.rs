use super::super::*;
use axum::body::Body;
use tower::ServiceExt;

const REQUEST: &str = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";

mod convergence;

#[tokio::test]
async fn failed_managed_http_create_remains_owned_until_awaited_close_and_cannot_replay() {
    let state = crate::tests::make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let token = "af".repeat(32);
    // The actual service adapter has no installed assets/process in this
    // fixture. Its failure must retire under ownership rather than fall back
    // to ordinary provisioning or lose the close handle.
    let create = json!({"managed": {"request_id": REQUEST, "ownership_token": token}});
    let (status, body) = post(&app, "/vms/create", create.clone()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "creation_failed");
    assert!(!body.to_string().contains(&token));
    let close = format!("/vms/managed/{REQUEST}/close");
    let (status, body) = post(&app, &close, json!({"ownership_token": token, "outcome": "failed"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "closed");
    let (status, body) = post(&app, "/vms/create", create).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "duplicate_request");
    assert!(state.instances.lock().unwrap().is_empty());
}

async fn post(app: &Router, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    request_json(app, path, "POST", body).await
}

async fn request_json(
    app: &Router,
    path: &str,
    method: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8192).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn managed_http_close_before_create_authenticates_tombstones_and_never_falls_back() {
    let state = crate::tests::make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let token = "af".repeat(32);
    let claim = format!("/vms/managed/{REQUEST}/claim");
    let close = format!("/vms/managed/{REQUEST}/close");
    let (status, body) = post(&app, &claim, json!({"ownership_token": token})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    let (status, body) = post(&app, &close, json!({"ownership_token": token, "outcome": "cancelled"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "closed");
    assert_eq!(body["evidence_retained"], false);
    let generation = body["generation"].clone();
    let (status, body) = post(&app, &claim, json!({"ownership_token": "ab".repeat(32)})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "forbidden");
    let (status, body) = post(&app, &claim, json!({"ownership_token": token})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "closed");
    assert_eq!(body["ownership"]["generation"], generation);
    assert_eq!(body["ownership"]["lease_seconds"], 300);
    let (status, body) = post(
        &app,
        "/vms/create",
        json!({"managed": {"request_id": REQUEST, "ownership_token": token}}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "duplicate_request");
    assert!(state.instances.lock().unwrap().is_empty());
    let stored =
        std::fs::read_to_string(state.run_dir.join("managed-ownership").join(format!("{REQUEST}.json"))).unwrap();
    assert!(!stored.contains(&token));
}

#[tokio::test]
async fn managed_http_invalid_secrets_and_non_ephemeral_requests_fail_before_effects() {
    let state = crate::tests::make_test_state();
    let app = build_service_router(Arc::clone(&state));
    for (path, body) in [
        (
            "/vms/create".into(),
            json!({"managed": {"request_id": REQUEST, "ownership_token": "invalid-private-token"}}),
        ),
        (
            format!("/vms/managed/{REQUEST}/claim"),
            json!({"ownership_token": "invalid-private-token"}),
        ),
        (
            "/vms/managed/not-a-uuid/close".into(),
            json!({"ownership_token": "af".repeat(32), "outcome": "cancelled"}),
        ),
        (
            "/vms/create".into(),
            json!({"name": "keep", "managed": {"request_id": REQUEST, "ownership_token": "af".repeat(32)}}),
        ),
        (
            "/vms/create".into(),
            json!({"persistent": true, "managed": {"request_id": REQUEST, "ownership_token": "af".repeat(32)}}),
        ),
    ] {
        let (status, response) = post(&app, &path, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["code"], "invalid_request");
        assert!(!response.to_string().contains("private-token"));
        assert!(!response.to_string().contains(&"af".repeat(32)));
    }
    assert!(state.instances.lock().unwrap().is_empty());
}
