use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn connection_catalog_query_refuses_invalid_input_without_echoing_it() {
    let app = build_service_router(crate::tests::make_test_state());
    for query in [
        "limit=0",
        "limit=33",
        "limit=4294967295",
        "limit=-1",
        "limit=1&limit=2",
        "cursor=private-query",
        "cursor=00000000-0000-0000-0000-000000000000",
        "cursor=12345678-1234-4234-8234-123456789ABC",
        "scope=private-query",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/connections?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "invalid query: {query}");
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let error: api::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.error, "credential request refused");
        assert_eq!(error.code, Some("request_conflict".into()));
        assert!(!String::from_utf8(bytes.to_vec()).unwrap().contains("private-query"));
    }
    for query in ["", "limit=1", "limit=32&cursor=12345678-1234-4234-8234-123456789abc"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/connections?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let result: api::ConnectionListResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(result.connections.is_empty());
        assert!(result.next_cursor.is_none());
        assert_eq!(result.scope_profiles[0].id, "agy-native");
    }
}

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

#[tokio::test]
async fn retained_credential_approvals_use_canonical_registry_owner_without_a_process() {
    let (state, _root) = crate::tests::make_test_state_with_tempdir();
    let id = "fa24d12d-86af-448a-9f99-dd1fcceae68c".to_string();
    let mut entry = crate::tests::test_persistent_entry("retained-box", state.run_dir.join("persistent").join(&id));
    entry.id = id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let app = build_service_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/vms/{id}/credential-grants"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let inventory: api::SessionGrantListResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(inventory.session_id, id);
    assert_eq!(inventory.owner_generation, id);
    assert!(inventory.grants.is_empty());
    assert!(state.instances.lock().unwrap().is_empty());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/vms/{id}/credential-grants/missing"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<api::ErrorResponse>(&bytes).unwrap().code,
        Some("grant_unavailable".into())
    );
    for route_id in [
        "retained-box".into(),
        capsem_crypto::uuid_v4().to_string(),
        uuid::Uuid::nil().to_string(),
        id.to_uppercase(),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/vms/{route_id}/credential-grants"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::OK);
    }
    let request = api::CreateSessionGrantRequest {
        request_id: capsem_crypto::uuid_v4().to_string(),
        expected_owner_generation: id.clone(),
        connection_id: "missing".into(),
        capability_ids: vec!["agy".into()],
        expires_at: None,
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/vms/{id}/credential-grants"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::CREATED);
    assert!(state.instances.lock().unwrap().is_empty());
}
