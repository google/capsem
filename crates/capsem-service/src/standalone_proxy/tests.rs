use axum::http::{Method, StatusCode};
use serde_json::json;

use super::*;

#[tokio::test]
async fn create_rejects_a_non_ip_bind_before_starting_any_worker() {
    let state = crate::tests::make_test_state();
    let app = crate::router_runtime::build_service_router(Arc::clone(&state));

    let (status, body) = crate::tests::route_request(
        app,
        Method::POST,
        "/proxies",
        Some(json!({"provider": "openai", "bind": "localhost", "port": 0})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not an IP address"));
    assert!(state.standalone_proxies.lock().await.is_empty());
}

#[tokio::test]
async fn create_rejects_an_empty_provider_before_starting_any_worker() {
    let state = crate::tests::make_test_state();
    let app = crate::router_runtime::build_service_router(Arc::clone(&state));

    let (status, body) = crate::tests::route_request(
        app,
        Method::POST,
        "/proxies",
        Some(json!({"provider": "", "bind": "127.0.0.1", "port": 0})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("1 to 128 bytes"));
    assert!(state.standalone_proxies.lock().await.is_empty());
}

#[test]
fn wire_expiry_is_bounded_by_the_lease_ttl() {
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let expiry = expiry_unix_ms(Instant::now() + LEASE_TTL);
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

    assert!(expiry >= before + LEASE_TTL.as_millis() as u64 - 10);
    assert!(expiry <= after + LEASE_TTL.as_millis() as u64 + 10);
}
