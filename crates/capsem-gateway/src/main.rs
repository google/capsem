mod auth;
mod proxy;
mod status;
mod terminal;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::auth::AuthState;
use crate::status::StatusCache;

#[derive(Parser, Debug)]
#[command(name = "capsem-gateway", about = "TCP-to-UDS gateway for capsem-service")]
struct Args {
    /// TCP port to listen on
    #[arg(long, default_value_t = 19222)]
    port: u16,

    /// Path to capsem-service UDS socket
    #[arg(long)]
    uds_path: Option<PathBuf>,

    /// Run in foreground (default: true, placeholder for daemonization)
    #[arg(long, default_value_t = true)]
    foreground: bool,
}

pub struct AppState {
    pub token: String,
    pub uds_path: PathBuf,
    pub status_cache: StatusCache,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "capsem_gateway=info".into()),
        )
        .init();

    let args = Args::parse();

    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = PathBuf::from(&home).join(".capsem/run");
    let uds_path = args.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    // Check if service socket exists (warning only -- service may start later)
    if !uds_path.exists() {
        tracing::warn!(path = %uds_path.display(), "service socket not found -- requests will return 502 until service starts");
    }

    // Generate auth token and write runtime files
    let token = auth::generate_token();
    let auth_state = AuthState::new(&run_dir, &token, args.port)?;

    let state = Arc::new(AppState {
        token,
        uds_path,
        status_cache: StatusCache::new(),
    });

    let app = Router::new()
        .route("/", get(handle_health))
        .route("/status", get(status::handle_status))
        .route("/terminal/{id}", get(terminal::handle_terminal_ws))
        .fallback(proxy::handle_proxy)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    info!(
        port = args.port,
        token_path = %auth_state.token_path.display(),
        uds_path = %state.uds_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "capsem-gateway listening"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind TCP listener")?;

    // Graceful shutdown on SIGTERM/SIGINT
    let shutdown_auth = auth_state.clone();
    axum::serve(listener, app)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    use crate::status::StatusCache;

    fn health_app(uds_path: &str) -> (axum::Router, Arc<AppState>) {
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: uds_path.into(),
            status_cache: StatusCache::new(),
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
