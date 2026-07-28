use super::*;
use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use tower::ServiceExt;

use crate::status::StatusCache;

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn health_app(uds_path: &str) -> (axum::Router, Arc<AppState>) {
    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: uds_path.into(),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    let app = axum::Router::new()
        .route("/", axum::routing::get(handle_health))
        .with_state(state.clone());
    (app, state)
}

fn service_proxy_app(uds_path: &str) -> axum::Router {
    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: uds_path.into(),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    service_proxy_routes().with_state(state)
}

#[tokio::test]
async fn gateway_unknown_paths_are_not_forwarded_to_service() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/not-a-capsem-api")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_profile_assets_edit_is_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("PATCH")
                .uri("/profiles/code/assets/edit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_profile_lifecycle_writes_are_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    for (method, uri) in [
        ("POST", "/profiles/create"),
        ("PATCH", "/profiles/code/edit"),
        ("DELETE", "/profiles/code/delete"),
        ("POST", "/profiles/code/clone"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_update_status_route_is_get_only() {
    let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
    let get_resp = app
        .clone()
        .oneshot(
            http::Request::builder()
                .method("GET")
                .uri("/update/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), http::StatusCode::BAD_GATEWAY);

    let post_resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/update/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post_resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn gateway_update_action_routes_are_post_only() {
    for uri in ["/update/check", "/update/apply"] {
        let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
        let post_resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), http::StatusCode::BAD_GATEWAY, "{uri}");

        let get_resp = app
            .oneshot(
                http::Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            get_resp.status(),
            http::StatusCode::METHOD_NOT_ALLOWED,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn gateway_fake_vm_mutation_routes_are_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    for (method, uri) in [
        ("PATCH", "/vms/test-vm/edit"),
        ("POST", "/vms/test-vm/restart"),
        ("POST", "/vms/test-vm/reload-profile"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_security_routes_are_explicitly_forwarded() {
    for (method, uri) in [
        ("GET", "/vms/test-vm/security/latest"),
        ("GET", "/vms/test-vm/security/status"),
        ("GET", "/vms/test-vm/detection/latest"),
        ("GET", "/vms/test-vm/detection/status"),
        ("GET", "/vms/test-vm/enforcement/latest"),
        ("GET", "/vms/test-vm/enforcement/status"),
        ("GET", "/security/latest"),
        ("GET", "/security/status"),
        ("GET", "/enforcement/latest"),
        ("GET", "/enforcement/status"),
        ("GET", "/detection/latest"),
        ("GET", "/detection/status"),
        ("GET", "/profiles/list"),
        ("GET", "/profiles/status"),
        ("GET", "/update/status"),
        ("POST", "/update/check"),
        ("POST", "/update/apply"),
        ("POST", "/profiles/reload"),
        ("GET", "/profiles/code/info"),
        ("GET", "/profiles/code/obom"),
        ("POST", "/profiles/code/validate"),
        ("POST", "/vms/create"),
        ("GET", "/vms/list"),
        ("GET", "/vms/test-vm/info"),
        ("GET", "/vms/test-vm/status"),
        ("GET", "/vms/test-vm/snapshots/status"),
        ("GET", "/vms/test-vm/snapshots/list"),
        ("GET", "/vms/test-vm/logs"),
        ("POST", "/vms/test-vm/exec"),
        ("POST", "/vms/test-vm/files/write"),
        ("POST", "/vms/test-vm/files/read"),
        ("GET", "/vms/test-vm/files/list"),
        ("GET", "/vms/test-vm/files/content?path=/root/a.txt"),
        ("POST", "/vms/test-vm/files/content?path=/root/a.txt"),
        ("GET", "/vms/test-vm/history"),
        ("GET", "/vms/test-vm/history/processes"),
        ("GET", "/vms/test-vm/history/counts"),
        ("GET", "/vms/test-vm/history/transcript"),
        ("GET", "/vms/test-vm/stats/detail"),
        ("GET", "/vms/test-vm/timeline"),
        ("POST", "/vms/test-vm/stop"),
        ("POST", "/vms/test-vm/pause"),
        ("DELETE", "/vms/test-vm/delete"),
        ("POST", "/vms/test-vm/start"),
        ("POST", "/vms/test-vm/resume"),
        ("POST", "/vms/test-vm/save"),
        ("GET", "/vms/test-vm/save/status"),
        ("GET", "/vms/test-vm/fork/status"),
        ("POST", "/vms/test-vm/fork"),
        ("POST", "/profiles/code/enforcement/evaluate"),
        ("GET", "/profiles/code/enforcement/info"),
        ("PUT", "/profiles/code/enforcement/rules/eicar_block/edit"),
        (
            "DELETE",
            "/profiles/code/enforcement/rules/eicar_block/delete",
        ),
        ("POST", "/profiles/code/enforcement/reload"),
        ("GET", "/profiles/code/enforcement/rules/list"),
        ("POST", "/profiles/code/detection/evaluate"),
        ("GET", "/profiles/code/detection/info"),
        ("PUT", "/profiles/code/detection/rules/eicar_detect/edit"),
        (
            "DELETE",
            "/profiles/code/detection/rules/eicar_detect/delete",
        ),
        ("POST", "/profiles/code/detection/reload"),
        ("GET", "/profiles/code/detection/rules/list"),
        ("GET", "/profiles/code/assets/status"),
        ("GET", "/profiles/code/assets/info"),
        ("POST", "/profiles/code/assets/ensure"),
        ("GET", "/profiles/code/skills/info"),
        ("GET", "/profiles/code/skills/list"),
        ("POST", "/profiles/code/skills/add"),
        ("PATCH", "/profiles/code/skills/build/edit"),
        ("DELETE", "/profiles/code/skills/build/delete"),
        ("GET", "/profiles/code/plugins/list"),
        ("GET", "/profiles/code/plugins/info"),
        ("GET", "/profiles/code/plugins/dummy_pre_eicar/info"),
        ("PATCH", "/profiles/code/plugins/dummy_pre_eicar/edit"),
        (
            "GET",
            "/profiles/code/plugins/credential_broker/credentials/info",
        ),
        (
            "POST",
            "/profiles/code/plugins/credential_broker/credentials/reload",
        ),
        ("GET", "/profiles/code/mcp/info"),
        ("GET", "/profiles/code/mcp/servers/list"),
        ("GET", "/profiles/code/mcp/default/info"),
        ("PATCH", "/profiles/code/mcp/default/edit"),
        ("PUT", "/profiles/code/mcp/servers/local/edit"),
        ("DELETE", "/profiles/code/mcp/servers/local/delete"),
        ("GET", "/profiles/code/mcp/servers/local/tools/list"),
        ("POST", "/profiles/code/mcp/servers/local/refresh"),
        ("PATCH", "/profiles/code/mcp/servers/local/tools/echo/edit"),
        ("POST", "/profiles/code/mcp/servers/local/tools/echo/call"),
        ("PUT", "/corp/edit"),
        ("GET", "/settings/info"),
        ("PATCH", "/settings/edit"),
        ("POST", "/profiles/code/reload"),
        ("GET", "/corp/info"),
        ("POST", "/corp/validate"),
        ("POST", "/corp/reload"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_GATEWAY,
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_vm_lifecycle_routes() {
    for (method, uri) in [
        ("POST", "/provision"),
        ("GET", "/list"),
        ("GET", "/info/test-vm"),
        ("POST", "/stop/test-vm"),
        ("GET", "/logs/test-vm"),
        ("POST", "/inspect/test-vm"),
        ("POST", "/exec/test-vm"),
        ("POST", "/write_file/test-vm"),
        ("POST", "/read_file/test-vm"),
        ("GET", "/files/test-vm"),
        ("GET", "/files/test-vm/content?path=/root/a.txt"),
        ("POST", "/files/test-vm/content?path=/root/a.txt"),
        ("GET", "/history/test-vm"),
        ("GET", "/history/test-vm/processes"),
        ("GET", "/history/test-vm/counts"),
        ("GET", "/history/test-vm/transcript"),
        ("GET", "/timeline/test-vm"),
        ("POST", "/suspend/test-vm"),
        ("DELETE", "/delete/test-vm"),
        ("POST", "/resume/test-vm"),
        ("POST", "/persist/test-vm"),
        ("POST", "/fork/test-vm"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_plugin_authoring_routes() {
    for (method, uri) in [
        ("GET", "/plugins"),
        ("GET", "/plugins/test-vm"),
        ("GET", "/plugins/test-vm/dummy_pre_eicar"),
        ("POST", "/plugins/test-vm/dummy_pre_eicar"),
        ("GET", "/plugins/global/dummy_pre_eicar"),
        ("POST", "/plugins/global/dummy_pre_eicar"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_profile_credential_routes() {
    for (method, uri) in [
        ("GET", "/profiles/code/credentials/info"),
        ("GET", "/profiles/code/credentials/status"),
        ("GET", "/profiles/code/credentials/list"),
        ("POST", "/profiles/code/credentials/reload"),
        ("GET", "/profiles/code/credentials/openai/info"),
        ("DELETE", "/profiles/code/credentials/openai/delete"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_enforcement_authoring_routes() {
    for (method, uri) in [
        ("POST", "/enforcements/evaluate"),
        ("POST", "/enforcements/rules/eicar_block"),
        ("DELETE", "/enforcements/rules/eicar_block"),
        ("POST", "/enforcements/reload"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_ledger_routes() {
    for (method, uri) in [
        ("GET", "/security/test-vm/latest"),
        ("GET", "/security/test-vm/info"),
        ("GET", "/detections/test-vm/latest"),
        ("GET", "/detections/test-vm/info"),
        ("GET", "/enforcements/test-vm/latest"),
        ("GET", "/enforcements/test-vm/info"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_corp_config_route() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/corp-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_does_not_forward_retired_global_asset_routes() {
    for (method, uri) in [("GET", "/assets/status"), ("POST", "/assets/ensure")] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_magic_settings_route() {
    for (method, uri) in [("GET", "/settings"), ("POST", "/settings")] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_settings_utility_routes() {
    for (method, uri) in [
        ("GET", "/settings/presets"),
        ("POST", "/settings/presets/high"),
        ("POST", "/settings/lint"),
        ("POST", "/settings/validate-key"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_global_reload_route() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/reload-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_does_not_forward_retired_mcp_policy_route() {
    for (method, uri) in [
        ("GET", "/mcp/policy"),
        ("GET", "/mcp/servers"),
        ("GET", "/mcp/tools"),
        ("POST", "/mcp/tools/refresh"),
        ("POST", "/mcp/tools/local__echo/approve"),
        ("POST", "/mcp/tools/local__echo/call"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn health_response_shape() {
    let (app, _) = health_app("/tmp/test.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["version"].is_string());
    assert!(json["service_socket"].is_string());
}

#[tokio::test]
async fn health_version_matches_cargo_pkg() {
    let (app, _) = health_app("/tmp/test.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["version"].as_str().unwrap(), env!("CARGO_PKG_VERSION"));
}

// --- Token endpoint ---

fn token_app() -> (axum::Router, Arc<AppState>) {
    let state = Arc::new(AppState {
        token: "test-secret-token-64chars-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        uds_path: "/tmp/test.sock".into(),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    let app = axum::Router::new()
        .route("/token", axum::routing::get(handle_token))
        .with_state(state.clone());
    (app, state)
}

#[tokio::test]
async fn token_returns_token_from_loopback() {
    let (app, state) = token_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["token"].as_str().unwrap(), state.token);
}

#[tokio::test]
async fn token_rejects_non_loopback_ip() {
    let (app, _) = token_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 100], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "forbidden");
}

#[tokio::test]
async fn token_allows_ipv6_loopback() {
    let (app, _) = token_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
        [0, 0, 0, 0, 0, 0, 0, 1],
        12345,
    ))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
}

// --- CORS restriction (issue #1) ---

fn cors_app() -> axum::Router {
    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: "/tmp/test.sock".into(),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    axum::Router::new()
        .route("/", axum::routing::get(handle_health))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    crate::cors::is_allowed_origin(origin)
                }))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state)
}

#[tokio::test]
async fn cors_allows_localhost_origin() {
    let app = cors_app();
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "http://localhost:4321")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "http://localhost:4321"
    );
}

#[tokio::test]
async fn cors_allows_127_origin() {
    let app = cors_app();
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "http://127.0.0.1:19222")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.headers().get("access-control-allow-origin").is_some());
}

#[tokio::test]
async fn cors_allows_tauri_origin() {
    let app = cors_app();
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "tauri://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.headers().get("access-control-allow-origin").is_some());
}

#[tokio::test]
async fn cors_rejects_external_origin() {
    let app = cors_app();
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "https://evil.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "external origin should not get CORS headers"
    );
}

#[tokio::test]
async fn cors_rejects_localhost_like_origin() {
    let app = cors_app();
    // AB-001: a prefix-based predicate would approve this attacker host.
    // The CORS layer must NOT echo back `Access-Control-Allow-Origin`
    // for it; otherwise a page on `http://localhostevil.com` could read
    // the gateway token via a cross-origin XHR to 127.0.0.1.
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "http://localhostevil.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "localhost-prefix attacker origin must not be approved by CORS"
    );
}

#[tokio::test]
async fn cors_rejects_127_0_0_1_dot_suffix_origin() {
    let app = cors_app();
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("origin", "http://127.0.0.1.evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "127.0.0.1 dotted-suffix attacker origin must not be approved by CORS"
    );
}

// --- Args / CLI parsing ---

#[test]
fn args_have_sensible_defaults() {
    let a = Args::parse_from(["capsem-gateway"]);
    assert_eq!(a.port, 19222);
    assert!(a.foreground);
    assert!(a.uds_path.is_none());
    assert!(a.run_dir.is_none());
}

#[test]
fn args_run_dir_override() {
    let a = Args::parse_from(["capsem-gateway", "--run-dir", "/tmp/capsem-run"]);
    assert_eq!(a.run_dir, Some(PathBuf::from("/tmp/capsem-run")));
}

#[test]
fn explicit_run_dir_decides_runtime_artifacts_even_with_env_override() {
    let _guard = EnvGuard::set("CAPSEM_RUN_DIR", "/tmp/capsem-wrong-run");
    let args = Args::parse_from(["capsem-gateway", "--run-dir", "/tmp/capsem-right-run"]);

    assert_eq!(
        gateway_run_dir(&args),
        PathBuf::from("/tmp/capsem-right-run")
    );
}

#[test]
fn args_port_override() {
    let a = Args::parse_from(["capsem-gateway", "--port", "8080"]);
    assert_eq!(a.port, 8080);
}

#[test]
fn args_uds_path_override() {
    let a = Args::parse_from(["capsem-gateway", "--uds-path", "/tmp/custom.sock"]);
    assert_eq!(a.uds_path, Some(PathBuf::from("/tmp/custom.sock")));
}

#[test]
fn args_rejects_bad_port() {
    let r = Args::try_parse_from(["capsem-gateway", "--port", "abc"]);
    assert!(r.is_err());
}

// --- Health response reflects the configured service socket ---

#[tokio::test]
async fn health_reports_service_socket_path() {
    let (app, _) = health_app("/tmp/unique-socket-path.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["service_socket"].as_str().unwrap(),
        "/tmp/unique-socket-path.sock"
    );
}

// --- Token endpoint: loopback matrix ---

#[tokio::test]
async fn token_rejects_another_external_ipv4() {
    let (app, _) = token_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([8, 8, 8, 8], 443))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_rejects_external_ipv6() {
    let (app, _) = token_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .body(Body::empty())
        .unwrap();
    // 2001:4860:4860::8888 (public Google DNS) -- not loopback.
    req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
        [0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888],
        443,
    ))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

// --- Events WebSocket: verify the route mounts and upgrades ---

#[tokio::test]
async fn events_ws_without_upgrade_header_is_rejected() {
    let state = Arc::new(AppState {
        token: "t".into(),
        uds_path: "/tmp/x.sock".into(),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    let app = axum::Router::new()
        .route("/events", axum::routing::get(handle_events_ws))
        .with_state(state);
    // A plain GET without Upgrade should return 426 Upgrade Required or 400.
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), http::StatusCode::OK);
}
