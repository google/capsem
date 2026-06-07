mod auth;
mod cors;
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
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use clap::Parser;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::auth::{AuthFailureTracker, AuthState};
use crate::status::StatusCache;

#[derive(Parser, Debug)]
#[command(
    name = "capsem-gateway",
    about = "TCP-to-UDS gateway for capsem-service"
)]
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
    let run_dir = capsem_core::paths::capsem_run_dir();
    let _ = std::fs::create_dir_all(&run_dir);
    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-gateway",
        sink: capsem_core::telemetry::LogSink::File {
            path: run_dir.join("gateway.log"),
        },
        // tower_http + hyper at debug so request-level and connection-level
        // failures (parse errors, early RST, malformed headers) land in the
        // gateway log; without these, auth-path flakes surface as curl "000"
        // with nothing on the gateway side to explain it.
        default_filter: "capsem_gateway=info,tower_http=debug,hyper=info",
    })?;

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
    let uds_path = args
        .uds_path
        .unwrap_or_else(|| run_dir.join("service.sock"));

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
        .merge(service_proxy_routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    cors::is_allowed_origin(origin)
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

fn service_proxy_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/version", get(proxy::handle_proxy))
        .route("/vms/create", post(proxy::handle_proxy))
        .route("/vms/list", get(proxy::handle_proxy))
        .route("/vms/{id}/info", get(proxy::handle_proxy))
        .route("/vms/{id}/status", get(proxy::handle_proxy))
        .route("/vms/{id}/edit", patch(proxy::handle_proxy))
        .route("/vms/{id}/logs", get(proxy::handle_proxy))
        .route("/vms/{id}/inspect", post(proxy::handle_proxy))
        .route("/vms/{id}/exec", post(proxy::handle_proxy))
        .route("/vms/{id}/files/write", post(proxy::handle_proxy))
        .route("/vms/{id}/files/read", post(proxy::handle_proxy))
        .route("/vms/{id}/stop", post(proxy::handle_proxy))
        .route("/vms/{id}/pause", post(proxy::handle_proxy))
        .route("/vms/{id}/delete", delete(proxy::handle_proxy))
        .route("/vms/{id}/start", post(proxy::handle_proxy))
        .route("/vms/{id}/resume", post(proxy::handle_proxy))
        .route("/vms/{id}/restart", post(proxy::handle_proxy))
        .route("/vms/{id}/save", post(proxy::handle_proxy))
        .route("/vms/{id}/save/status", get(proxy::handle_proxy))
        .route("/vms/{id}/fork/status", get(proxy::handle_proxy))
        .route("/vms/{id}/reload-profile", post(proxy::handle_proxy))
        .route("/purge", post(proxy::handle_proxy))
        .route("/run", post(proxy::handle_proxy))
        .route("/stats", get(proxy::handle_proxy))
        .route("/service-logs", get(proxy::handle_proxy))
        .route("/triage", get(proxy::handle_proxy))
        .route("/panics", get(proxy::handle_proxy))
        .route("/host-logs/{name}", get(proxy::handle_proxy))
        .route("/vms/{id}/timeline", get(proxy::handle_proxy))
        .route("/vms/{id}/security/latest", get(proxy::handle_proxy))
        .route("/vms/{id}/security/status", get(proxy::handle_proxy))
        .route("/vms/{id}/detection/latest", get(proxy::handle_proxy))
        .route("/vms/{id}/detection/status", get(proxy::handle_proxy))
        .route("/vms/{id}/enforcement/latest", get(proxy::handle_proxy))
        .route("/vms/{id}/enforcement/status", get(proxy::handle_proxy))
        .route("/profiles/list", get(proxy::handle_proxy))
        .route("/profiles/create", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/edit", patch(proxy::handle_proxy))
        .route("/profiles/{profile_id}/delete", delete(proxy::handle_proxy))
        .route("/profiles/{profile_id}/clone", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/validate", post(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/enforcement/evaluate",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/edit",
            put(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/reload",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/evaluate",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/edit",
            put(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/reload",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/rules/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/edit",
            patch(proxy::handle_proxy),
        )
        .route("/profiles/{profile_id}/reload", post(proxy::handle_proxy))
        .route("/vms/{id}/fork", post(proxy::handle_proxy))
        .route("/settings/info", get(proxy::handle_proxy))
        .route("/settings/edit", patch(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/assets/status",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/assets/ensure",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/add",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/edit",
            patch(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/status",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/reload",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/{credential_id}/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/credentials/{credential_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route("/corp/info", get(proxy::handle_proxy))
        .route("/corp/edit", put(proxy::handle_proxy))
        .route("/corp/validate", post(proxy::handle_proxy))
        .route("/corp/reload", post(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/mcp/servers/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/list",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/refresh",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit",
            patch(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call",
            post(proxy::handle_proxy),
        )
        .route("/vms/{id}/history", get(proxy::handle_proxy))
        .route("/vms/{id}/history/processes", get(proxy::handle_proxy))
        .route("/vms/{id}/history/counts", get(proxy::handle_proxy))
        .route("/vms/{id}/history/transcript", get(proxy::handle_proxy))
        .route("/vms/{id}/files/list", get(proxy::handle_proxy))
        .route(
            "/vms/{id}/files/content",
            get(proxy::handle_proxy).post(proxy::handle_proxy),
        )
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
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
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
    async fn gateway_security_routes_are_explicitly_forwarded() {
        for (method, uri) in [
            ("GET", "/vms/test-vm/security/latest"),
            ("GET", "/vms/test-vm/security/status"),
            ("GET", "/vms/test-vm/detection/latest"),
            ("GET", "/vms/test-vm/detection/status"),
            ("GET", "/vms/test-vm/enforcement/latest"),
            ("GET", "/vms/test-vm/enforcement/status"),
            ("GET", "/profiles/list"),
            ("POST", "/profiles/create"),
            ("GET", "/profiles/default/info"),
            ("PATCH", "/profiles/default/edit"),
            ("DELETE", "/profiles/default/delete"),
            ("POST", "/profiles/default/clone"),
            ("POST", "/profiles/default/validate"),
            ("POST", "/vms/create"),
            ("GET", "/vms/list"),
            ("GET", "/vms/test-vm/info"),
            ("GET", "/vms/test-vm/status"),
            ("PATCH", "/vms/test-vm/edit"),
            ("GET", "/vms/test-vm/logs"),
            ("POST", "/vms/test-vm/inspect"),
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
            ("GET", "/vms/test-vm/timeline"),
            ("POST", "/vms/test-vm/stop"),
            ("POST", "/vms/test-vm/pause"),
            ("DELETE", "/vms/test-vm/delete"),
            ("POST", "/vms/test-vm/start"),
            ("POST", "/vms/test-vm/resume"),
            ("POST", "/vms/test-vm/restart"),
            ("POST", "/vms/test-vm/save"),
            ("GET", "/vms/test-vm/save/status"),
            ("GET", "/vms/test-vm/fork/status"),
            ("POST", "/vms/test-vm/fork"),
            ("POST", "/vms/test-vm/reload-profile"),
            ("POST", "/profiles/default/enforcement/evaluate"),
            ("GET", "/profiles/default/enforcement/info"),
            (
                "PUT",
                "/profiles/default/enforcement/rules/eicar_block/edit",
            ),
            (
                "DELETE",
                "/profiles/default/enforcement/rules/eicar_block/delete",
            ),
            ("POST", "/profiles/default/enforcement/reload"),
            ("GET", "/profiles/default/enforcement/rules/list"),
            ("POST", "/profiles/default/detection/evaluate"),
            ("GET", "/profiles/default/detection/info"),
            ("PUT", "/profiles/default/detection/rules/eicar_detect/edit"),
            (
                "DELETE",
                "/profiles/default/detection/rules/eicar_detect/delete",
            ),
            ("POST", "/profiles/default/detection/reload"),
            ("GET", "/profiles/default/detection/rules/list"),
            ("GET", "/profiles/default/assets/status"),
            ("POST", "/profiles/default/assets/ensure"),
            ("GET", "/profiles/default/skills/info"),
            ("GET", "/profiles/default/skills/list"),
            ("POST", "/profiles/default/skills/add"),
            ("PATCH", "/profiles/default/skills/build/edit"),
            ("DELETE", "/profiles/default/skills/build/delete"),
            ("GET", "/profiles/default/credentials/info"),
            ("GET", "/profiles/default/credentials/status"),
            ("GET", "/profiles/default/credentials/list"),
            ("POST", "/profiles/default/credentials/reload"),
            ("GET", "/profiles/default/credentials/openai/info"),
            ("DELETE", "/profiles/default/credentials/openai/delete"),
            ("GET", "/profiles/default/plugins/list"),
            ("GET", "/profiles/default/plugins/dummy_pre_eicar/info"),
            ("PATCH", "/profiles/default/plugins/dummy_pre_eicar/edit"),
            ("GET", "/profiles/default/mcp/servers/list"),
            ("GET", "/profiles/default/mcp/servers/local/tools/list"),
            ("POST", "/profiles/default/mcp/servers/local/refresh"),
            (
                "PATCH",
                "/profiles/default/mcp/servers/local/tools/echo/edit",
            ),
            (
                "POST",
                "/profiles/default/mcp/servers/local/tools/echo/call",
            ),
            ("PUT", "/corp/edit"),
            ("GET", "/settings/info"),
            ("PATCH", "/settings/edit"),
            ("POST", "/profiles/default/reload"),
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
}
