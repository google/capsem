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
#[command(name = "capsem-gateway", version, about = "TCP-to-UDS gateway for capsem-service")]
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
    let args = Args::parse();
    let run_dir = gateway_run_dir(&args);
    let _ = std::fs::create_dir_all(&run_dir);
    let _telemetry_guard = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-gateway",
        sink: capsem_foundation::telemetry::LogSink::File {
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
    capsem_foundation::telemetry::install_panic_logger("capsem-gateway");

    // Companion guards: refuse to run without a live parent service, and
    // refuse if another gateway already holds the singleton lock for this
    // run_dir. Both conditions are expected (stale launch, double-spawn race)
    // and resolved by exiting 0 -- standalone launches become no-ops.
    let lock_path = args.lock_path.clone().unwrap_or_else(|| run_dir.join("gateway.lock"));
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
    let bound_port = listener.local_addr().context("failed to read bound TCP port")?.port();

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
                .allow_origin(AllowOrigin::predicate(|origin, _| cors::is_allowed_origin(origin)))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .layer(request_trace_layer())
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
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
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

fn gateway_run_dir(args: &Args) -> PathBuf {
    args.run_dir
        .clone()
        .unwrap_or_else(capsem_foundation::paths::capsem_run_dir)
}

fn service_proxy_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/version", get(proxy::handle_proxy))
        .route("/update/status", get(proxy::handle_proxy))
        .route("/system/status", get(proxy::handle_proxy))
        .route("/update/check", post(proxy::handle_proxy))
        .route("/update/apply", post(proxy::handle_proxy))
        .route("/vms/create", post(proxy::handle_proxy))
        .route("/vms/list", get(proxy::handle_proxy))
        .route("/vms/{id}/info", get(proxy::handle_proxy))
        .route("/vms/{id}/status", get(proxy::handle_proxy))
        .route("/vms/{id}/snapshots/status", get(proxy::handle_proxy))
        .route("/vms/{id}/snapshots/list", get(proxy::handle_proxy))
        .route("/vms/{id}/logs", get(proxy::handle_proxy))
        .route("/vms/{id}/exec", post(proxy::handle_proxy))
        .route("/vms/{id}/files/write", post(proxy::handle_proxy))
        .route("/vms/{id}/files/read", post(proxy::handle_proxy))
        .route("/vms/{id}/stop", post(proxy::handle_proxy))
        .route("/vms/{id}/pause", post(proxy::handle_proxy))
        .route("/vms/{id}/delete", delete(proxy::handle_proxy))
        .route("/vms/{id}/start", post(proxy::handle_proxy))
        .route("/vms/{id}/resume", post(proxy::handle_proxy))
        .route("/vms/{id}/save", post(proxy::handle_proxy))
        .route("/vms/{id}/save/status", get(proxy::handle_proxy))
        .route("/vms/{id}/fork/status", get(proxy::handle_proxy))
        .route("/purge", post(proxy::handle_proxy))
        .route("/run", post(proxy::handle_proxy))
        .route("/stats", get(proxy::handle_proxy))
        .route("/vms/{id}/stats/summary", get(proxy::handle_proxy))
        .route("/vms/{id}/stats/detail", get(proxy::handle_proxy))
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
        .route("/security/latest", get(proxy::handle_proxy))
        .route("/security/status", get(proxy::handle_proxy))
        .route("/enforcement/latest", get(proxy::handle_proxy))
        .route("/enforcement/status", get(proxy::handle_proxy))
        .route("/detection/latest", get(proxy::handle_proxy))
        .route("/detection/status", get(proxy::handle_proxy))
        .route("/profiles/list", get(proxy::handle_proxy))
        .route("/profiles/status", get(proxy::handle_proxy))
        .route("/profiles/reload", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/obom", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/validate", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/enforcement/evaluate", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/enforcement/info", get(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/edit",
            put(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route("/profiles/{profile_id}/enforcement/reload", post(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/enforcement/rules/list",
            get(proxy::handle_proxy),
        )
        .route("/profiles/{profile_id}/detection/evaluate", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/detection/info", get(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/edit",
            put(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route("/profiles/{profile_id}/detection/reload", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/detection/rules/list", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/plugins/list", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/plugins/info", get(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/credential_broker/credentials/info",
            get(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/credential_broker/credentials/reload",
            post(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/edit",
            patch(proxy::handle_proxy),
        )
        .route("/profiles/{profile_id}/reload", post(proxy::handle_proxy))
        .route("/vms/{id}/fork", post(proxy::handle_proxy))
        .route("/settings/info", get(proxy::handle_proxy))
        .route("/settings/edit", patch(proxy::handle_proxy))
        .route("/profiles/{profile_id}/assets/status", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/assets/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/assets/ensure", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/skills/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/skills/list", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/skills/add", post(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/edit",
            patch(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/delete",
            delete(proxy::handle_proxy),
        )
        .route("/corp/info", get(proxy::handle_proxy))
        .route("/corp/edit", put(proxy::handle_proxy))
        .route("/corp/validate", post(proxy::handle_proxy))
        .route("/corp/reload", post(proxy::handle_proxy))
        .route("/profiles/{profile_id}/mcp/servers/list", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/mcp/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/mcp/default/info", get(proxy::handle_proxy))
        .route("/profiles/{profile_id}/mcp/default/edit", patch(proxy::handle_proxy))
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/edit",
            put(proxy::handle_proxy),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/delete",
            delete(proxy::handle_proxy),
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
/// Events are broadcast when consecutive authoritative status reads detect VM
/// state transitions.
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

/// Request spans name the method and path, never the query string.
///
/// tower-http's default span records the full URI at debug, and the gateway
/// log runs `tower_http=debug`. The browser WebSocket API cannot set headers,
/// so `/events` and `/terminal/{id}` authenticate with `?token=`; with the
/// default span every such request wrote the bearer token into gateway.log.
fn request_trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    impl tower_http::trace::MakeSpan<axum::body::Body> + Clone,
> {
    TraceLayer::new_for_http().make_span_with(|req: &http::Request<axum::body::Body>| {
        tracing::debug_span!(
            "request",
            method = %req.method(),
            path = req.uri().path(),
            version = ?req.version(),
        )
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
mod tests;
