mod ui_preview;
mod ui_tools;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use capsem_plugin_engine::{
    InstallPluginRequest, InstallRunRequest, PluginError, PluginRegistry, RunPluginRequest,
};
use capsem_ui_catalog::native_deck::{
    artifact_by_id, create_chart, create_diagram, create_sheet, create_slide, create_slide_deck,
    create_table, demo_deck_proof, generate_image, query_demo_sql, ChartRequest, DiagramRequest,
    GenerateImageRequest, NativeArtifact, NativeDeckProof, SheetRequest, SlideDeckRequest,
    SlideRequest, SqliteQueryRequest, SqliteQueryResponse, TableRequest,
};
use capsem_ui_catalog::ui_tools::UiToolProgramResult;
use serde_json::json;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    registry: Arc<PluginRegistry>,
    authored_ui: Arc<RwLock<Option<UiToolProgramResult>>>,
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
        authored_ui: Arc::new(RwLock::new(None)),
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
        .route("/chat", get(ui_preview::preview_html))
        .route("/deck", get(ui_preview::preview_html))
        .route("/ui-preview", get(ui_preview::preview_html))
        .route("/ui/spec/demo", get(ui_preview::demo))
        .route("/ui/spec/validate", post(ui_preview::validate))
        .route("/ui/tools/run", post(ui_tools::run))
        .route("/ui/tools/latest", get(ui_tools::latest))
        .route("/ui/tools/acceptance", get(ui_tools::acceptance))
        .route("/native/deck-proof", get(native_deck_proof))
        .route("/native/artifacts", get(native_artifacts))
        .route("/native/artifacts/:id", get(native_artifact))
        .route("/native/data/sqlite/query", post(native_sqlite_query))
        .route("/native/data/sheet", post(native_create_sheet))
        .route("/native/generate/image", post(native_generate_image))
        .route("/native/ui/table", post(native_create_table))
        .route("/native/ui/chart", post(native_create_chart))
        .route("/native/ui/diagram", post(native_create_diagram))
        .route("/native/ui/slide", post(native_create_slide))
        .route("/native/ui/slide-deck", post(native_create_slide_deck))
        .route("/native/ui/render-artifact", post(native_render_artifact))
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

async fn native_deck_proof() -> Result<Json<NativeDeckProof>, NativeApiError> {
    Ok(Json(
        demo_deck_proof().map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_artifacts() -> Result<Json<Vec<NativeArtifact>>, NativeApiError> {
    Ok(Json(
        demo_deck_proof()
            .map_err(NativeApiError::bad_request)?
            .artifacts,
    ))
}

async fn native_artifact(Path(id): Path<String>) -> Result<Json<NativeArtifact>, NativeApiError> {
    match artifact_by_id(&id).map_err(NativeApiError::bad_request)? {
        Some(artifact) => Ok(Json(artifact)),
        None => Err(NativeApiError::not_found(format!(
            "artifact not found: {id}"
        ))),
    }
}

async fn native_sqlite_query(
    Json(request): Json<SqliteQueryRequest>,
) -> Result<Json<SqliteQueryResponse>, NativeApiError> {
    Ok(Json(
        query_demo_sql(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_sheet(
    Json(request): Json<SheetRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        create_sheet(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_generate_image(
    Json(request): Json<GenerateImageRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        generate_image(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_table(
    Json(request): Json<TableRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        create_table(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_chart(
    Json(request): Json<ChartRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        create_chart(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_diagram(
    Json(request): Json<DiagramRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        create_diagram(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_slide(
    Json(request): Json<SlideRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    Ok(Json(
        create_slide(request).map_err(NativeApiError::bad_request)?,
    ))
}

async fn native_create_slide_deck(
    Json(request): Json<SlideDeckRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let (_, artifact) = create_slide_deck(request).map_err(NativeApiError::bad_request)?;
    Ok(Json(artifact))
}

async fn native_render_artifact(
    Json(request): Json<RenderArtifactRequest>,
) -> Result<Json<RenderArtifactResponse>, NativeApiError> {
    let artifact = artifact_by_id(&request.artifact_id)
        .map_err(NativeApiError::bad_request)?
        .ok_or_else(|| {
            NativeApiError::not_found(format!("artifact not found: {}", request.artifact_id))
        })?;
    Ok(Json(RenderArtifactResponse {
        ok: true,
        component: artifact.spec["component"]
            .as_str()
            .unwrap_or("capsem-elt")
            .to_owned(),
        artifact,
    }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderArtifactRequest {
    artifact_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderArtifactResponse {
    ok: bool,
    component: String,
    artifact: NativeArtifact,
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

struct NativeApiError {
    status: StatusCode,
    message: String,
}

impl NativeApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }
}

impl IntoResponse for NativeApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "ok": false,
            "error": self.message,
        }));
        (self.status, body).into_response()
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
