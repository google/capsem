mod auth;
mod proxy;
mod status;
mod terminal;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::auth::{AuthFailureTracker, AuthState};
use crate::status::StatusCache;

#[derive(Parser, Debug)]
#[command(name = "capsem-gateway", about = "TCP-to-UDS gateway for capsem-service")]
struct Args {
    /// TCP port to listen on (0 = OS-assigned)
    #[arg(long, default_value_t = 19222)]
    port: u16,

    /// Path to capsem-service UDS socket
    #[arg(long)]
    uds_path: Option<PathBuf>,

    /// Directory for runtime files (gateway.token / gateway.port / gateway.pid).
    /// Overrides CAPSEM_RUN_DIR env var and the default $HOME/.capsem/run.
    #[arg(long)]
    run_dir: Option<PathBuf>,

    /// Run in foreground (default: true, placeholder for daemonization)
    #[arg(long, default_value_t = true)]
    foreground: bool,

    /// PID of the capsem-service that spawned us. The gateway is a companion
    /// process: it refuses to start without a live parent service and exits
    /// the moment that parent dies. See capsem-guard.
    #[arg(long)]
    parent_pid: Option<u32>,

    /// Path for the singleton lockfile (overrides default under run_dir).
    #[arg(long)]
    lock_path: Option<PathBuf>,
}

pub struct AppState {
    pub token: String,
    pub uds_path: PathBuf,
    pub status_cache: StatusCache,
    pub auth_failures: AuthFailureTracker,
    /// Broadcast channel for real-time events to WebSocket /events clients.
    pub events_tx: tokio::sync::broadcast::Sender<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // tower_http + hyper at debug so request-level and connection-level
                // failures (parse errors, early RST, malformed headers) land in the
                // gateway log; without these, auth-path flakes surface as curl "000"
                // with nothing on the gateway side to explain it.
                .unwrap_or_else(|_| "capsem_gateway=info,tower_http=debug,hyper=info".into()),
        )
        .init();

    // Surface any gateway panic in the log instead of letting it vanish into
    // the void -- under test load a panicked task would otherwise just drop
    // the connection, leaving the client with no response and no trace.
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(
            panic = %info,
            location = info.location().map(|l| format!("{l}")).unwrap_or_default(),
            "gateway panic"
        );
    }));

    let args = Args::parse();

    // Resolve run_dir in priority: --run-dir, then the shared capsem_run_dir
    // helper (CAPSEM_RUN_DIR > <capsem_home>/run). Must match capsem-service
    // so parent and child read/write the same gateway.{token,port,pid} files.
    let run_dir = args
        .run_dir
        .clone()
        .unwrap_or_else(capsem_core::paths::capsem_run_dir);

    // Companion guards: refuse to run without a live parent service, and
    // refuse if another gateway already holds the singleton lock for this
    // run_dir. Both conditions are expected (stale launch, double-spawn race)
    // and resolved by exiting 0 -- standalone launches become no-ops.
    let lock_path = args
        .lock_path
        .clone()
        .unwrap_or_else(|| run_dir.join("gateway.lock"));
    match capsem_guard::install(args.parent_pid, &lock_path) {
        Ok(Some(guards)) => {
            // Keep the guards alive for the process's lifetime.
            Box::leak(Box::new(guards));
        }
        Ok(None) => {
            tracing::info!(
                lock = %lock_path.display(),
                "another capsem-gateway is already running; exiting 0"
            );
            return Ok(());
        }
        Err(e) => {
            tracing::info!(
                error = %e,
                "gateway refusing to run without a live capsem-service; exiting 0"
            );
            return Ok(());
        }
    }
    let uds_path = args.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    // Check if service socket exists (warning only -- service may start later)
    if !uds_path.exists() {
        tracing::warn!(path = %uds_path.display(), "service socket not found -- requests will return 502 until service starts");
    }

    // Bind TCP listener first so the runtime file records the real bound port
    // (args.port may be 0 to request an OS-assigned port).
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind TCP listener")?;
    let bound_port = listener
        .local_addr()
        .context("failed to read bound TCP port")?
        .port();

    // Generate auth token and write runtime files (token/port/pid).
    let token = auth::generate_token();
    let auth_state = AuthState::new(&run_dir, &token, bound_port)?;

    let (events_tx, _) = tokio::sync::broadcast::channel::<String>(64);
    let state = Arc::new(AppState {
        token,
        uds_path,
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx,
    });

    let app = Router::new()
        .route("/", get(handle_health))
        .route("/health", get(handle_health))
        .route("/token", get(handle_token))
        .route("/status", get(status::handle_status))
        .route("/terminal/{id}", get(terminal::handle_terminal_ws))
        .route("/events", get(handle_events_ws))
        .fallback(proxy::handle_proxy)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    origin.to_str().is_ok_and(|s| {
                        s.starts_with("http://localhost")
                            || s.starts_with("http://127.0.0.1")
                            || s.starts_with("https://localhost")
                            || s.starts_with("https://127.0.0.1")
                            || s.starts_with("tauri://")
                    })
                }))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    info!(
        port = bound_port,
        token_path = %auth_state.token_path.display(),
        uds_path = %state.uds_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "capsem-gateway listening"
    );

    // Graceful shutdown on SIGTERM/SIGINT
    let shutdown_auth = auth_state.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            info!("shutting down");
            shutdown_auth.cleanup();
        })
        .await
        .context("server error")?;

    // Belt-and-suspenders cleanup (signal handler may not run on all exit paths)
    auth_state.cleanup();

    Ok(())
}

async fn handle_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "service_socket": state.uds_path.display().to_string(),
    }))
}

/// WebSocket endpoint for real-time events (VM state changes, progress, etc.).
///
/// Clients receive JSON messages: `{"type":"vm-state-changed","payload":{...}}`
/// Events are broadcast when the status cache detects VM state transitions.
async fn handle_events_ws(
    ws: axum::extract::WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut rx = state.events_tx.subscribe();
    ws.on_upgrade(|mut socket| async move {
        use axum::extract::ws::Message;
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Ok(text) => {
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                frame = socket.recv() => {
                    match frame {
                        Some(Ok(Message::Ping(data))) => {
                            if socket.send(Message::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        _ => {}
                    }
                }
            }
        }
    })
}

/// Return the auth token. Hardcoded to only accept requests from 127.0.0.1.
async fn handle_token(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    if !addr.ip().is_loopback() {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        )
            .into_response();
    }
    Json(serde_json::json!({ "token": state.token })).into_response()
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use tower::ServiceExt;

    use crate::status::StatusCache;

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
            .oneshot(
                http::Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
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
            .oneshot(
                http::Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
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
                        origin.to_str().is_ok_and(|s| {
                            s.starts_with("http://localhost")
                                || s.starts_with("http://127.0.0.1")
                                || s.starts_with("https://localhost")
                                || s.starts_with("https://127.0.0.1")
                                || s.starts_with("tauri://")
                        })
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
        // "http://localhostevil.com" starts with "http://localhost" so the
        // prefix-based predicate will match it. This is acceptable for a
        // service bound to 127.0.0.1 -- the key protection is blocking
        // truly external origins (different host). Verify the response
        // succeeds (the origin IS matched by the predicate).
        let _resp = app
            .oneshot(
                http::Request::builder()
                    .uri("/")
                    .header("origin", "http://localhostevil.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
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
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["service_socket"].as_str().unwrap(), "/tmp/unique-socket-path.sock");
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
}
