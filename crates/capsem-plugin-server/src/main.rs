mod ui_preview;
mod ui_tools;

use std::{collections::BTreeMap, env, net::SocketAddr, path::PathBuf, sync::Arc, time::Instant};

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use capsem_ai::{GenerationEngine, GenerationError, ModelProvider};
use capsem_plugin_engine::{
    InstallPluginRequest, InstallRunRequest, PluginError, PluginRegistry, RunPluginRequest,
};
use capsem_ui_catalog::native_deck::{
    artifact_by_id, create_chart, create_diagram, create_sheet, create_slide, create_slide_deck,
    create_table, demo_deck_proof, generate_image, generate_text, query_demo_sql, ChartRequest,
    DiagramRequest, GenerateImageRequest, GenerateTextRequest, NativeArtifact, NativeArtifactKind,
    NativeDeckProof, SheetRequest, SlideDeckRequest, SlideDeckSpec, SlideRef, SlideRequest,
    SqliteQueryRequest, SqliteQueryResponse, TableRequest,
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
    native_workspace: Arc<RwLock<NativeWorkspace>>,
    telemetry: Arc<RwLock<Vec<NativeTelemetryEvent>>>,
    generation: Arc<GenerationEngine>,
}

#[derive(Clone, Debug, Default)]
struct NativeWorkspace {
    artifacts: BTreeMap<String, NativeArtifact>,
    deck_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTelemetryEvent {
    operation: String,
    artifact_id: String,
    kind: NativeArtifactKind,
    duration_ms: u128,
    status: String,
}

impl NativeWorkspace {
    fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    fn insert(&mut self, artifact: NativeArtifact) {
        if artifact.kind == NativeArtifactKind::SlideDeck {
            self.deck_id = Some(artifact.id.clone());
        }
        self.artifacts.insert(artifact.id.clone(), artifact);
    }

    fn artifact(&self, id: &str) -> Option<&NativeArtifact> {
        self.artifacts.get(id)
    }

    fn artifacts(&self) -> Vec<NativeArtifact> {
        self.artifacts.values().cloned().collect()
    }

    fn to_proof(&self) -> NativeDeckProof {
        let artifacts = self.artifacts();
        let deck_artifact = self
            .deck_id
            .as_deref()
            .and_then(|id| self.artifacts.get(id))
            .or_else(|| {
                self.artifacts
                    .values()
                    .find(|artifact| artifact.kind == NativeArtifactKind::SlideDeck)
            });
        let deck = deck_artifact
            .and_then(deck_from_artifact)
            .unwrap_or_else(|| self.draft_deck());
        let chart_count = artifacts
            .iter()
            .filter(|artifact| artifact.kind == NativeArtifactKind::Chart)
            .count();
        let sqlite_rows = artifacts
            .iter()
            .filter(|artifact| {
                matches!(
                    artifact.kind,
                    NativeArtifactKind::Sheet | NativeArtifactKind::Table
                )
            })
            .filter_map(|artifact| artifact.spec["rows"].as_array().map(Vec::len))
            .max()
            .unwrap_or(0);
        let required_artifacts_present = [
            NativeArtifactKind::GeneratedImage,
            NativeArtifactKind::Sheet,
            NativeArtifactKind::Table,
            NativeArtifactKind::Chart,
            NativeArtifactKind::Diagram,
            NativeArtifactKind::Slide,
            NativeArtifactKind::SlideDeck,
        ]
        .iter()
        .all(|kind| artifacts.iter().any(|artifact| artifact.kind == *kind));
        NativeDeckProof {
            ok: !artifacts.is_empty(),
            title: deck.title.clone(),
            summary: capsem_ui_catalog::native_deck::NativeDeckSummary {
                sqlite_rows,
                chart_count,
                individual_artifact_count: artifacts.len(),
                required_artifacts_present,
            },
            artifacts,
            deck,
        }
    }

    fn draft_deck(&self) -> SlideDeckSpec {
        SlideDeckSpec {
            id: "workspace-draft".to_owned(),
            title: "Native Artifact Workspace".to_owned(),
            slides: self
                .artifacts
                .values()
                .filter(|artifact| artifact.kind == NativeArtifactKind::Slide)
                .map(|artifact| SlideRef {
                    artifact_id: artifact.id.clone(),
                    title: artifact.title.clone(),
                })
                .collect(),
        }
    }
}

fn deck_from_artifact(artifact: &NativeArtifact) -> Option<SlideDeckSpec> {
    let slides = serde_json::from_value(artifact.spec["slides"].clone()).ok()?;
    Some(SlideDeckSpec {
        id: artifact.id.clone(),
        title: artifact.title.clone(),
        slides,
    })
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
        native_workspace: Arc::new(RwLock::new(NativeWorkspace::default())),
        telemetry: Arc::new(RwLock::new(Vec::new())),
        generation: Arc::new(
            GenerationEngine::from_capsem_environment()
                .context("failed to load Capsem AI settings")?,
        ),
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
        .route("/native/workspace/reset", post(native_workspace_reset))
        .route("/native/telemetry", get(native_telemetry))
        .route("/native/mcp/tools", get(native_mcp_tools))
        .route("/native/artifacts", get(native_artifacts))
        .route("/native/artifacts/:id", get(native_artifact))
        .route("/native/data/sqlite/query", post(native_sqlite_query))
        .route("/native/data/sheet", post(native_create_sheet))
        .route("/native/generate/text", post(native_generate_text))
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

async fn native_deck_proof(
    State(state): State<AppState>,
) -> Result<Json<NativeDeckProof>, NativeApiError> {
    let workspace = state.native_workspace.read().await;
    if workspace.is_empty() {
        Ok(Json(
            demo_deck_proof().map_err(NativeApiError::bad_request)?,
        ))
    } else {
        Ok(Json(workspace.to_proof()))
    }
}

async fn native_workspace_reset(State(state): State<AppState>) -> Json<serde_json::Value> {
    *state.native_workspace.write().await = NativeWorkspace::default();
    state.telemetry.write().await.clear();
    Json(json!({
        "ok": true,
        "workspace": "native-artifacts",
        "artifacts": 0
    }))
}

async fn native_telemetry(State(state): State<AppState>) -> Json<Vec<NativeTelemetryEvent>> {
    Json(state.telemetry.read().await.clone())
}

async fn native_mcp_tools() -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "namespace": "local",
        "tools": [
            {"name": "local__native_workspace_reset", "route": "POST /native/workspace/reset"},
            {"name": "local__data_sqlite_query", "route": "POST /native/data/sqlite/query"},
            {"name": "local__data_sheet", "route": "POST /native/data/sheet"},
            {"name": "local__generate_text", "route": "POST /native/generate/text"},
            {"name": "local__generate_image", "route": "POST /native/generate/image"},
            {"name": "local__ui_table", "route": "POST /native/ui/table"},
            {"name": "local__ui_chart", "route": "POST /native/ui/chart"},
            {"name": "local__ui_diagram", "route": "POST /native/ui/diagram"},
            {"name": "local__ui_slide", "route": "POST /native/ui/slide"},
            {"name": "local__ui_slide_deck", "route": "POST /native/ui/slide-deck"},
            {"name": "local__native_render_artifact", "route": "POST /native/ui/render-artifact"},
            {"name": "local__native_telemetry", "route": "GET /native/telemetry"}
        ]
    }))
}

async fn native_artifacts(
    State(state): State<AppState>,
) -> Result<Json<Vec<NativeArtifact>>, NativeApiError> {
    let workspace = state.native_workspace.read().await;
    if workspace.is_empty() {
        Ok(Json(
            demo_deck_proof()
                .map_err(NativeApiError::bad_request)?
                .artifacts,
        ))
    } else {
        Ok(Json(workspace.artifacts()))
    }
}

async fn native_artifact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let workspace = state.native_workspace.read().await;
    let found = workspace
        .artifact(&id)
        .cloned()
        .map(Some)
        .unwrap_or_else(|| artifact_by_id(&id).ok().flatten());
    match found {
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
    State(state): State<AppState>,
    Json(request): Json<SheetRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_sheet(request).map_err(NativeApiError::bad_request)?,
        "data.sheet",
        started,
    )
    .await
}

async fn native_generate_text(
    State(state): State<AppState>,
    Json(request): Json<GenerateTextRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        generate_text_provider(request, &state.generation)
            .await
            .map_err(NativeApiError::bad_request)?,
        "generate.text",
        started,
    )
    .await
}

async fn native_generate_image(
    State(state): State<AppState>,
    Json(request): Json<GenerateImageRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        generate_image_provider(request, &state.generation)
            .await
            .map_err(NativeApiError::bad_request)?,
        "generate.image",
        started,
    )
    .await
}

async fn native_create_table(
    State(state): State<AppState>,
    Json(request): Json<TableRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_table(request).map_err(NativeApiError::bad_request)?,
        "ui.table",
        started,
    )
    .await
}

async fn native_create_chart(
    State(state): State<AppState>,
    Json(request): Json<ChartRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_chart(request).map_err(NativeApiError::bad_request)?,
        "ui.chart",
        started,
    )
    .await
}

async fn native_create_diagram(
    State(state): State<AppState>,
    Json(request): Json<DiagramRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_diagram(request).map_err(NativeApiError::bad_request)?,
        "ui.diagram",
        started,
    )
    .await
}

async fn native_create_slide(
    State(state): State<AppState>,
    Json(request): Json<SlideRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_slide(request).map_err(NativeApiError::bad_request)?,
        "ui.slide",
        started,
    )
    .await
}

async fn native_create_slide_deck(
    State(state): State<AppState>,
    Json(request): Json<SlideDeckRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    let (_, artifact) = create_slide_deck(request).map_err(NativeApiError::bad_request)?;
    store_artifact(&state, artifact, "ui.slideDeck", started).await
}

async fn native_render_artifact(
    State(state): State<AppState>,
    Json(request): Json<RenderArtifactRequest>,
) -> Result<Json<RenderArtifactResponse>, NativeApiError> {
    let workspace = state.native_workspace.read().await;
    let artifact = workspace
        .artifact(&request.artifact_id)
        .cloned()
        .map(Some)
        .unwrap_or_else(|| artifact_by_id(&request.artifact_id).ok().flatten())
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

async fn store_artifact(
    state: &AppState,
    artifact: NativeArtifact,
    operation: &str,
    started: Instant,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    state
        .native_workspace
        .write()
        .await
        .insert(artifact.clone());
    state.telemetry.write().await.push(NativeTelemetryEvent {
        operation: operation.to_owned(),
        artifact_id: artifact.id.clone(),
        kind: artifact.kind,
        duration_ms: started.elapsed().as_millis(),
        status: artifact.spec["status"].as_str().unwrap_or("ok").to_owned(),
    });
    Ok(Json(artifact))
}

async fn generate_text_provider(
    request: GenerateTextRequest,
    engine: &GenerationEngine<impl ModelProvider>,
) -> Result<NativeArtifact, String> {
    let mut artifact = generate_text(request.clone())?;
    match engine
        .generate_text(capsem_ai::GenerateTextRequest {
            provider: request.provider,
            model: request.model,
            system: request.system,
            prompt: request.prompt,
        })
        .await
    {
        Ok(output) => {
            artifact.spec["status"] = json!("generated");
            artifact.spec["provider"] = json!(output.provider);
            artifact.spec["providerModel"] = json!(output.model);
            artifact.spec["text"] = json!(output.text);
        }
        Err(error) => generation_error_to_artifact(&mut artifact, error),
    }
    Ok(artifact)
}

async fn generate_image_provider(
    request: GenerateImageRequest,
    engine: &GenerationEngine<impl ModelProvider>,
) -> Result<NativeArtifact, String> {
    let mut artifact = generate_image(request.clone())?;
    match engine
        .generate_image(capsem_ai::GenerateImageRequest {
            provider: request.provider,
            model: request.model,
            prompt: request.prompt,
        })
        .await
    {
        Ok(output) => {
            artifact.spec["status"] = json!("generated");
            artifact.spec["provider"] = json!(output.provider);
            artifact.spec["providerModel"] = json!(output.model);
            if let Some(mime_type) = output.mime_type {
                artifact.spec["mimeType"] = json!(mime_type);
            }
            if let Some(data_url) = output.data_url {
                artifact.spec["dataUrl"] = json!(data_url);
            }
            if let Some(url) = output.url {
                artifact.spec["url"] = json!(url);
            }
            if let Some(revised_prompt) = output.revised_prompt {
                artifact.spec["revisedPrompt"] = json!(revised_prompt);
            }
        }
        Err(error) => generation_error_to_artifact(&mut artifact, error),
    }
    Ok(artifact)
}

fn generation_error_to_artifact(artifact: &mut NativeArtifact, error: GenerationError) {
    match error {
        GenerationError::MissingCredential { checked, .. } => {
            artifact.spec["status"] = json!("configMissing");
            artifact.spec["error"] = json!("missing provider credential");
            artifact.spec["checkedCredentials"] = json!(checked);
        }
        other => {
            artifact.spec["status"] = json!("providerError");
            artifact.spec["error"] = json!(other.to_string());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use capsem_ai::{FakeModelProvider, GenerationSettings};

    #[tokio::test]
    async fn text_generation_artifact_uses_capsem_ai_engine() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty().with_credential("google-api-key", "AIza-test"),
            FakeModelProvider,
        );
        let artifact = generate_text_provider(
            GenerateTextRequest {
                id: "text-test".to_owned(),
                title: "Text Test".to_owned(),
                prompt: "review this".to_owned(),
                system: Some("be strict".to_owned()),
                provider: "gemini".to_owned(),
                model: None,
            },
            &engine,
        )
        .await
        .unwrap();

        assert_eq!(artifact.kind, NativeArtifactKind::GeneratedText);
        assert_eq!(artifact.spec["status"], "generated");
        assert_eq!(artifact.spec["provider"], "google");
        assert_eq!(artifact.spec["text"], "fake: review this");
    }

    #[tokio::test]
    async fn missing_capsem_ai_credential_stays_typed_artifact_state() {
        let engine = GenerationEngine::new(GenerationSettings::empty(), FakeModelProvider);
        let artifact = generate_text_provider(
            GenerateTextRequest {
                id: "text-test".to_owned(),
                title: "Text Test".to_owned(),
                prompt: "review this".to_owned(),
                system: None,
                provider: "gemini".to_owned(),
                model: None,
            },
            &engine,
        )
        .await
        .unwrap();

        assert_eq!(artifact.spec["status"], "configMissing");
        assert_eq!(artifact.spec["error"], "missing provider credential");
        assert!(artifact.spec["checkedCredentials"].is_array());
    }
}
