use axum::http::{Method, StatusCode};
use serde_json::json;

use super::*;

async fn running_proxy(state: &ServiceState, session_id: &str, lease_token: &str) -> StandaloneProxy {
    let session_dir = state.run_dir.join("sessions").join(session_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    StandaloneProxy {
        lease_token: lease_token.to_string(),
        expires_at: Instant::now() + LEASE_TTL,
        worker: crate::proxy_worker::tests::fake("normal").await.unwrap(),
        authority: WorkerAuthority::default(),
        accept_task: tokio::spawn(async {}),
        broker_task: tokio::spawn(async {}),
        session_dir,
        provider_id: "openai".to_string(),
        upstream_policy: crate::upstream_broker::test_policy_publisher(),
    }
}

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

#[tokio::test]
async fn create_cleans_its_session_when_provider_or_worker_startup_fails() {
    for provider in ["missing-provider", "openai"] {
        let state = crate::tests::make_test_state();
        std::fs::create_dir_all(state.run_dir.join("sessions")).unwrap();
        let app = crate::router_runtime::build_service_router(Arc::clone(&state));

        let (status, body) = crate::tests::route_request(
            app,
            Method::POST,
            "/proxies",
            Some(json!({"provider": provider, "bind": "127.0.0.1", "port": 0})),
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("start standalone proxy"),
            "{body}"
        );
        assert!(state.standalone_proxies.lock().await.is_empty());
        let sessions = state.run_dir.join("sessions");
        assert!(
            !sessions.exists() || std::fs::read_dir(sessions).unwrap().next().is_none(),
            "failed startup retained its private session"
        );
    }
}

#[tokio::test]
async fn unknown_proxy_lease_routes_fail_without_mutating_registry() {
    let state = crate::tests::make_test_state();
    let app = crate::router_runtime::build_service_router(Arc::clone(&state));
    for (method, path) in [
        (Method::POST, "/proxies/missing/heartbeat"),
        (Method::POST, "/proxies/missing/stop"),
    ] {
        let (status, body) =
            crate::tests::route_request(app.clone(), method, path, Some(json!({"lease_token": "unknown"}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "proxy session not found");
    }
    assert!(state.standalone_proxies.lock().await.is_empty());
}

#[tokio::test]
async fn empty_proxy_registry_refresh_and_shutdown_are_noops() {
    let state = crate::tests::make_test_state();

    assert_eq!(refresh_policies(&state).await.unwrap(), (0, Vec::new()));
    state.stop_all_standalone_proxies().await;
    assert!(state.standalone_proxies.lock().await.is_empty());
}

#[tokio::test]
async fn heartbeat_and_stop_require_the_exact_lease_and_cleanup_the_worker() {
    let state = crate::tests::make_test_state();
    let id = "proxy-live";
    let proxy = running_proxy(&state, id, "secret").await;
    state.standalone_proxies.lock().await.insert(id.to_string(), proxy);
    let app = crate::router_runtime::build_service_router(Arc::clone(&state));

    let (status, body) = crate::tests::route_request(
        app.clone(),
        Method::POST,
        &format!("/proxies/{id}/heartbeat"),
        Some(json!({"lease_token": "wrong"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = crate::tests::route_request(
        app.clone(),
        Method::POST,
        &format!("/proxies/{id}/heartbeat"),
        Some(json!({"lease_token": "secret"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["session_id"], id);

    let (status, body) = crate::tests::route_request(
        app,
        Method::POST,
        &format!("/proxies/{id}/stop"),
        Some(json!({"lease_token": "secret"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"session_id": id, "stopped": true}));
    assert!(state.standalone_proxies.lock().await.is_empty());
    assert!(!state.run_dir.join("sessions").join(id).exists());
}

#[tokio::test]
async fn expired_lease_revokes_and_cleans_up_its_worker_automatically() {
    let state = crate::tests::make_test_state();
    let id = "proxy-expired";
    let mut proxy = running_proxy(&state, id, "secret").await;
    proxy.expires_at = Instant::now();
    state.standalone_proxies.lock().await.insert(id.to_string(), proxy);

    spawn_lease_monitor(Arc::clone(&state), id.to_string());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.standalone_proxies.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expired lease was not removed");
    assert!(!state.run_dir.join("sessions").join(id).exists());
}

#[test]
fn wire_expiry_is_bounded_by_the_lease_ttl() {
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let expiry = expiry_unix_ms(Instant::now() + LEASE_TTL);
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

    assert!(expiry >= before + LEASE_TTL.as_millis() as u64 - 10);
    assert!(expiry <= after + LEASE_TTL.as_millis() as u64 + 10);
}
