mod ui_preview;
mod ui_tools;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use capsem_plugin_engine::{
    InstallPluginRequest, InstallRunRequest, PluginError, PluginRegistry, RunPluginRequest,
};
use serde_json::json;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    registry: Arc<PluginRegistry>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let artifact_dir = env::var_os("CAPSEM_PLUGIN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/plugin-artifacts"));
    let engine_mode = env::var("CAPSEM_PLUGIN_ENGINE").unwrap_or_else(|_| "placeholder".to_owned());
    let state = AppState {
        registry: Arc::new(match engine_mode.as_str() {
            "wasmtime-wat" => PluginRegistry::wasmtime_wat(artifact_dir),
            "wasmtime-wat-fuel" => PluginRegistry::wasmtime_wat_fuel(artifact_dir),
            _ => PluginRegistry::new(artifact_dir),
        }),
    };

    let app = app(state);
    let addr: SocketAddr = env::var("CAPSEM_PLUGIN_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8788".to_owned())
        .parse()
        .context("invalid CAPSEM_PLUGIN_BIND")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("rust plugin server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui_preview::preview_html))
        .route("/ui-preview", get(ui_preview::preview_html))
        .route("/ui/spec/demo", get(ui_preview::demo))
        .route("/ui/spec/validate", post(ui_preview::validate))
        .route("/ui/tools/run", post(ui_tools::run))
        .route("/ui/tools/acceptance", get(ui_tools::acceptance))
        .route("/health", get(health))
        .route("/plugins/install", post(install_plugin))
        .route("/plugins/run", post(run_plugin))
        .route("/plugins/install-run", post(install_run_plugin))
        .nest_service("/assets", ServeDir::new("ui-preview/dist/assets"))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "service": "capsem-plugin-server",
        "engines": ["placeholder", "wasmtime-wat", "wasmtime-wat-fuel"]
    }))
}

async fn install_plugin(
    State(state): State<AppState>,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<capsem_plugin_engine::InstallPluginResponse>, ApiError> {
    Ok(Json(state.registry.install(request)?))
}

async fn run_plugin(
    State(state): State<AppState>,
    Json(request): Json<RunPluginRequest>,
) -> Result<Json<capsem_plugin_engine::RunPluginResponse>, ApiError> {
    Ok(Json(state.registry.run(request)?))
}

async fn install_run_plugin(
    State(state): State<AppState>,
    Json(request): Json<InstallRunRequest>,
) -> Result<Json<capsem_plugin_engine::InstallRunResponse>, ApiError> {
    Ok(Json(state.registry.install_run(request)?))
}

struct ApiError(PluginError);

impl From<PluginError> for ApiError {
    fn from(value: PluginError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            PluginError::Request(_) => StatusCode::BAD_REQUEST,
            PluginError::Runtime(_) => StatusCode::BAD_GATEWAY,
            PluginError::Io(_) | PluginError::Serde(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "ok": false,
            "error": self.0.to_string(),
        }));
        (status, body).into_response()
    }
}
