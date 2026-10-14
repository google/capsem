use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn connection_catalog_and_nine_routes_use_typed_owner_failures_without_secret_echo() {
    let state = crate::tests::make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/connections").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let catalog: api::ConnectionListResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(catalog.connections.is_empty());
    assert!(!catalog.scope_profiles[0].enabled);
    assert!(!catalog.scope_profiles[0].capabilities[0].supported);
    let cases = [
        ("GET", "/connections/authorizations/missing"),
        ("DELETE", "/connections/authorizations/missing"),
        ("GET", "/connections/missing"),
        ("DELETE", "/connections/missing"),
        ("GET", "/vms/not-canonical/credential-grants"),
        ("DELETE", "/vms/not-canonical/credential-grants/missing"),
    ];
    for (method, path) in cases {
        let response = app
            .clone()
            .oneshot(Request::builder().method(method).uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.status().is_client_error() || response.status().is_server_error());
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let error: api::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(error.code.is_some());
        assert_eq!(error.error, "credential request refused");
    }
    for path in ["/connections/google/authorize", "/vms/not-canonical/credential-grants"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"raw_secret":"private-secret"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!text.contains("private-secret"));
        assert!(serde_json::from_str::<api::ErrorResponse>(&text)
            .unwrap()
            .code
            .is_some());
    }
}
