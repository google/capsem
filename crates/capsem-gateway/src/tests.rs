use super::*;
use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use tower::ServiceExt;

use crate::status::StatusCache;

mod route_forwarding;

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

#[tokio::test]
async fn health_response_shape() {
    let (app, _) = health_app("/tmp/test.sock");
    let resp = app
        .oneshot(http::Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["version"].is_string());
    assert!(json["service_socket"].is_string());
}

#[tokio::test]
async fn health_version_matches_cargo_pkg() {
    let (app, _) = health_app("/tmp/test.sock");
    let resp = app
        .oneshot(http::Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
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
    let mut req = http::Request::builder().uri("/token").body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["token"].as_str().unwrap(), state.token);
}

#[tokio::test]
async fn token_rejects_non_loopback_ip() {
    let (app, _) = token_app();
    let mut req = http::Request::builder().uri("/token").body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 100], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "forbidden");
}

#[tokio::test]
async fn token_allows_ipv6_loopback() {
    let (app, _) = token_app();
    let mut req = http::Request::builder().uri("/token").body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 12345))));
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

    assert_eq!(gateway_run_dir(&args), PathBuf::from("/tmp/capsem-right-run"));
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
        .oneshot(http::Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["service_socket"].as_str().unwrap(), "/tmp/unique-socket-path.sock");
}

// --- Token endpoint: loopback matrix ---

#[tokio::test]
async fn token_rejects_another_external_ipv4() {
    let (app, _) = token_app();
    let mut req = http::Request::builder().uri("/token").body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([8, 8, 8, 8], 443))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_rejects_external_ipv6() {
    let (app, _) = token_app();
    let mut req = http::Request::builder().uri("/token").body(Body::empty()).unwrap();
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
        .oneshot(http::Request::builder().uri("/events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(resp.status(), http::StatusCode::OK);
}

// --- Host header: DNS rebinding ---
//
// `/token` is unauthenticated and gated by the peer being loopback. A page at
// `http://evil.example:19222` whose DNS answer flips to 127.0.0.1 is a
// same-origin caller from a loopback peer: CORS never enters into it, and the
// token came back. The Host header is the one thing a rebinding page cannot
// forge, so every request must name a loopback host.

fn guarded_app() -> (axum::Router, Arc<AppState>) {
    let (_, state) = token_app();
    let app = axum::Router::new()
        .route("/health", axum::routing::get(handle_health))
        .route("/token", axum::routing::get(handle_token))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .with_state(state.clone());
    (app, state)
}

async fn get_with_host(app: axum::Router, path: &str, host: Option<&str>) -> http::Response<Body> {
    let mut builder = http::Request::builder().uri(path);
    if let Some(host) = host {
        builder = builder.header(http::header::HOST, host);
    }
    let mut req = builder.body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    app.oneshot(req).await.unwrap()
}

#[tokio::test]
async fn token_refuses_a_rebound_host_from_a_loopback_peer() {
    let (app, state) = guarded_app();
    let resp = get_with_host(app, "/token", Some("evil.example:19222")).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&body).contains(&state.token),
        "the token must not leave for a foreign host"
    );
}

#[tokio::test]
async fn every_route_refuses_a_rebound_host() {
    for host in [
        "evil.example",
        "evil.example:19222",
        "localhost.evil.example",
        "127.0.0.1.evil.example:80",
    ] {
        let (app, _) = guarded_app();
        let resp = get_with_host(app, "/health", Some(host)).await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN, "host {host}");
    }
}

#[tokio::test]
async fn loopback_hosts_in_every_spelling_are_accepted() {
    for host in [
        "localhost",
        "localhost:19222",
        "LOCALHOST:19222",
        "127.0.0.1",
        "127.0.0.1:19222",
        "[::1]",
        "[::1]:19222",
    ] {
        let (app, state) = guarded_app();
        let resp = get_with_host(app, "/token", Some(host)).await;
        assert_eq!(resp.status(), http::StatusCode::OK, "host {host}");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["token"], state.token, "host {host}");
    }
}

#[tokio::test]
async fn a_request_without_a_host_header_is_not_a_browser_and_passes() {
    // Browsers always send Host, so an absent header cannot be a rebinding
    // page; refusing it would only break raw HTTP/1.0 tooling.
    let (app, _) = guarded_app();
    let resp = get_with_host(app, "/health", None).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
}

// --- Constant-time token comparison ---

#[test]
fn token_comparison_does_not_depend_on_prefix_agreement() {
    assert!(auth::token_matches("abc", "abc"));
    assert!(!auth::token_matches("abd", "abc"));
    assert!(!auth::token_matches("ab", "abc"));
    assert!(!auth::token_matches("abcd", "abc"));
    assert!(!auth::token_matches("", "abc"));
}

// --- Request spans must not record the query string ---
//
// The browser WebSocket API cannot set headers, so `/events` and `/terminal`
// authenticate with `?token=`. tower-http's default span records the full
// URI at debug, and the gateway log runs `tower_http=debug`, so every such
// request wrote the bearer token into gateway.log in clear text.

#[derive(Default)]
struct TraceCapture {
    records: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

struct TraceCaptureVisitor<'a>(&'a mut Vec<String>);

impl tracing::field::Visit for TraceCaptureVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TraceCapture {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut records = self.records.lock().unwrap();
        attrs.record(&mut TraceCaptureVisitor(&mut records));
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut records = self.records.lock().unwrap();
        event.record(&mut TraceCaptureVisitor(&mut records));
    }
}

#[tokio::test]
async fn request_spans_record_the_path_but_never_the_query() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;

    let capture = TraceCapture::default();
    let records = std::sync::Arc::clone(&capture.records);
    let dispatcher = tracing::Dispatch::new(tracing_subscriber::registry().with(capture));

    let (_, state) = health_app("/tmp/test.sock");
    let app = axum::Router::new()
        .route("/health", axum::routing::get(handle_health))
        .layer(request_trace_layer())
        .with_state(state);
    let req = http::Request::builder()
        .uri("/health?token=SECRET-QUERY-TOKEN")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).with_subscriber(dispatcher).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::OK);

    let records = std::mem::take(&mut *records.lock().unwrap());
    assert!(
        records.iter().any(|r| r.contains("/health")),
        "the span must still name the path: {records:?}"
    );
    assert!(
        !records.iter().any(|r| r.contains("SECRET-QUERY-TOKEN")),
        "the query string carries the WebSocket token and must not be logged: {records:?}"
    );
}
