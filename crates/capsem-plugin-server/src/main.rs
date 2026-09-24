mod ui_preview;
mod ui_tools;

use std::{
    collections::BTreeMap,
    env, fs,
    net::SocketAddr,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
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
    create_table, create_timeline, demo_deck_proof, generate_embedding, generate_image,
    generate_text, query_demo_sql, ChartRequest, DiagramRequest, GenerateEmbeddingRequest,
    GenerateImageRequest, GenerateTextRequest, NativeArtifact, NativeArtifactKind, NativeDeckProof,
    SheetRequest, SlideDeckRequest, SlideDeckSpec, SlideRef, SlideRequest, SqliteQueryRequest,
    SqliteQueryResponse, TableRequest, TimelineRequest,
};
use capsem_ui_catalog::ui_tools::UiToolProgramResult;
use capsem_ui_runtime::storage::{SqliteWorkspaceStore, WorkspaceRestore};
use capsem_ui_runtime::workspace::{
    artifact_record, change_request_record, delete_artifact_record, resolve_task_record,
    select_record, style_patch_record, text_patch_record, text_selector_patch_record,
    title_patch_record, ElementProvenance, ElementStylePatch, ElementTextPatch, RenderDelta,
    RenderProjection, RenderTopology, TopologyNode, WorkspaceAnnotationTarget, WorkspaceCheckpoint,
    WorkspaceContent, WorkspaceContentType, WorkspaceFrame, WorkspaceProjector, WorkspaceRole,
    WorkspaceSnapshot, WorkspaceStatus, WorkspaceTaskStatus, WorkspaceVerb,
};
use serde_json::json;
use tokio::sync::{broadcast, Mutex, RwLock};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

const NATIVE_WORKSPACE_ID: &str = "native-artifacts";

#[derive(Clone)]
struct AppState {
    registry: Arc<PluginRegistry>,
    authored_ui: Arc<RwLock<Option<UiToolProgramResult>>>,
    native_workspace: Arc<RwLock<NativeWorkspace>>,
    workspace_store: Option<Arc<Mutex<SqliteWorkspaceStore>>>,
    workspace_tx: broadcast::Sender<WorkspaceStreamMessage>,
    telemetry: Arc<RwLock<Vec<NativeTelemetryEvent>>>,
    generation: Arc<GenerationEngine>,
    export_tool: ExportToolConfig,
}

#[derive(Clone, Debug)]
struct ExportToolConfig {
    python: PathBuf,
    script: PathBuf,
    output_dir: PathBuf,
}

#[derive(Clone, Debug, Default)]
struct NativeWorkspace {
    artifacts: BTreeMap<String, NativeArtifact>,
    deck_id: Option<String>,
    projector: WorkspaceProjector,
    frames: Vec<WorkspaceFrame>,
    next_seq: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WorkspaceStreamMessage {
    Snapshot { snapshot: WorkspaceSnapshot },
    Record { frame: WorkspaceFrame },
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum NativeTelemetryEvent {
    #[serde(rename_all = "camelCase")]
    Artifact {
        operation: String,
        artifact_id: String,
        kind: NativeArtifactKind,
        duration_ms: u128,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<serde_json::Value>,
    },
    #[serde(rename_all = "camelCase")]
    Workspace {
        operation: String,
        seq: u64,
        record_id: String,
        timestamp: String,
        role: WorkspaceRole,
        principal: String,
        title: String,
        content_type: WorkspaceContentType,
        verb: WorkspaceVerb,
        status: WorkspaceStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        duration_ms: u128,
        delta_count: usize,
        deltas: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mutation: Option<serde_json::Value>,
    },
    #[serde(rename_all = "camelCase")]
    RenderError {
        operation: String,
        timestamp: String,
        source: String,
        status: String,
        duration_ms: u128,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        component: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        renderer: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Export {
        operation: String,
        timestamp: String,
        artifact_id: String,
        format: String,
        adapter: String,
        status: String,
        duration_ms: u128,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceInfo {
    workspace_id: String,
    checkpoint_seq: u64,
    seq: u64,
    selected: Option<String>,
    topology: RenderTopology,
    elements: Vec<WorkspaceInfoElement>,
    change_requests: Vec<WorkspaceInfoChangeRequest>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceInfoElement {
    id: String,
    title: String,
    content_type: WorkspaceContentType,
    status: WorkspaceStatus,
    component: String,
    node: Option<TopologyNode>,
    provenance: ElementProvenance,
    artifact_kind: Option<NativeArtifactKind>,
    artifact_handle: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceInfoChangeRequest {
    seq: u64,
    id: String,
    timestamp: String,
    principal: String,
    target: String,
    instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotation: Option<WorkspaceAnnotationTarget>,
    status: WorkspaceStatus,
}

impl NativeWorkspace {
    fn from_restore(restore: WorkspaceRestore) -> Self {
        let projection = restore.projector.projection().clone();
        let artifacts: BTreeMap<String, NativeArtifact> = projection
            .elements
            .values()
            .filter_map(|element| {
                element
                    .artifact
                    .as_ref()
                    .map(|artifact| (artifact.id.clone(), artifact.clone()))
            })
            .collect();
        let deck_id = artifacts
            .values()
            .find(|artifact| artifact.kind == NativeArtifactKind::SlideDeck)
            .map(|artifact| artifact.id.clone());
        Self {
            artifacts,
            deck_id,
            projector: restore.projector,
            frames: restore.frames,
            next_seq: projection.seq,
        }
    }

    fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    fn insert(
        &mut self,
        artifact: NativeArtifact,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if artifact.kind == NativeArtifactKind::SlideDeck {
            self.deck_id = Some(artifact.id.clone());
        }
        let verb = if self.artifacts.contains_key(&artifact.id) {
            WorkspaceVerb::Replace
        } else {
            WorkspaceVerb::Create
        };
        self.artifacts.insert(artifact.id.clone(), artifact.clone());
        self.next_seq += 1;
        let record = artifact_record(self.next_seq, now_timestamp(), principal, verb, artifact);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn delete(&mut self, target: &str, principal: &str) -> Result<WorkspaceFrame, String> {
        let artifact = self
            .artifacts
            .remove(target)
            .ok_or_else(|| format!("artifact not found: {target}"))?;
        if self.deck_id.as_deref() == Some(target) {
            self.deck_id = None;
        }
        self.next_seq += 1;
        let record = delete_artifact_record(self.next_seq, now_timestamp(), principal, artifact);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn select(
        &mut self,
        target: Option<String>,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        self.next_seq += 1;
        let record = select_record(self.next_seq, now_timestamp(), principal, target);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn update_title(
        &mut self,
        target: String,
        title: String,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if title.trim().is_empty() {
            return Err("title must not be empty".to_owned());
        }
        self.next_seq += 1;
        let record = title_patch_record(self.next_seq, now_timestamp(), principal, target, title);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn update_text(
        &mut self,
        target: String,
        text: String,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if text.trim().is_empty() {
            return Err("text must not be empty".to_owned());
        }
        self.next_seq += 1;
        let record = text_patch_record(self.next_seq, now_timestamp(), principal, target, text);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn patch_text(
        &mut self,
        target: String,
        text_patch: ElementTextPatch,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if !self.projector.projection().elements.contains_key(&target) {
            return Err(format!("cannot patch missing element: {target}"));
        }
        if text_patch
            .selector
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
            && text_patch
                .shadow_selector
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("text patch selector or shadowSelector must not be empty".to_owned());
        }
        if text_patch.text.trim().is_empty() {
            return Err("text patch text must not be empty".to_owned());
        }
        self.next_seq += 1;
        let record = text_selector_patch_record(
            self.next_seq,
            now_timestamp(),
            principal,
            target,
            text_patch,
        );
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn patch_style(
        &mut self,
        target: String,
        style_patch: ElementStylePatch,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if !self.projector.projection().elements.contains_key(&target) {
            return Err(format!("cannot patch missing element: {target}"));
        }
        if style_patch
            .selector
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
            && style_patch
                .shadow_selector
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("style patch selector or shadowSelector must not be empty".to_owned());
        }
        if style_patch.styles.is_empty() {
            return Err("style patch styles must not be empty".to_owned());
        }
        self.next_seq += 1;
        let record = style_patch_record(
            self.next_seq,
            now_timestamp(),
            principal,
            target,
            style_patch,
        );
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn request_change(
        &mut self,
        target: String,
        instruction: String,
        annotation: Option<WorkspaceAnnotationTarget>,
        principal: &str,
    ) -> Result<WorkspaceFrame, String> {
        if !self.projector.projection().elements.contains_key(&target) {
            return Err(format!(
                "cannot request change for missing element: {target}"
            ));
        }
        if instruction.trim().is_empty() {
            return Err("change request instruction must not be empty".to_owned());
        }
        self.next_seq += 1;
        let record = change_request_record(
            self.next_seq,
            now_timestamp(),
            principal,
            target,
            instruction.trim().to_owned(),
            annotation,
        );
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
    }

    fn resolve_task(&mut self, task_id: String, principal: &str) -> Result<WorkspaceFrame, String> {
        if !self.projector.projection().tasks.contains_key(&task_id) {
            return Err(format!("cannot resolve missing task: {task_id}"));
        }
        self.next_seq += 1;
        let record = resolve_task_record(self.next_seq, now_timestamp(), principal, task_id);
        let deltas = self.projector.apply(&record)?;
        let frame = WorkspaceFrame { record, deltas };
        self.frames.push(frame.clone());
        Ok(frame)
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

    fn snapshot(&self) -> WorkspaceSnapshot {
        let projection = self.projector.projection().clone();
        let checkpoint = self.checkpoint();
        WorkspaceSnapshot {
            workspace_id: NATIVE_WORKSPACE_ID.to_owned(),
            checkpoint,
            projection,
            tail: Vec::new(),
        }
    }

    fn checkpoint(&self) -> WorkspaceCheckpoint {
        self.projector
            .checkpoint(NATIVE_WORKSPACE_ID, now_timestamp())
    }

    fn projection(&self) -> RenderProjection {
        self.projector.projection().clone()
    }

    fn info(&self) -> WorkspaceInfo {
        let projection = self.projector.projection();
        let elements = projection
            .elements
            .values()
            .map(|element| WorkspaceInfoElement {
                id: element.id.clone(),
                title: element.title.clone(),
                content_type: element.content_type,
                status: element.status,
                component: element.component.clone(),
                node: projection.topology.nodes.get(&element.id).cloned(),
                provenance: element.provenance.clone(),
                artifact_kind: element.artifact.as_ref().map(|artifact| artifact.kind),
                artifact_handle: element
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.handle.clone()),
            })
            .collect();
        WorkspaceInfo {
            workspace_id: NATIVE_WORKSPACE_ID.to_owned(),
            checkpoint_seq: projection.seq,
            seq: projection.seq,
            selected: projection.selected.clone(),
            topology: projection.topology.clone(),
            elements,
            change_requests: projection
                .tasks
                .values()
                .map(|task| WorkspaceInfoChangeRequest {
                    seq: task.created_seq,
                    id: task.id.clone(),
                    timestamp: task.created_at.clone(),
                    principal: task.created_by.clone(),
                    target: task.target.clone(),
                    instruction: task.instruction.clone(),
                    annotation: task.annotation.clone(),
                    status: match task.status {
                        WorkspaceTaskStatus::Open => WorkspaceStatus::Pending,
                        WorkspaceTaskStatus::Resolved => WorkspaceStatus::Complete,
                    },
                })
                .collect(),
        }
    }

    fn frames_after(&self, seq: u64) -> Vec<WorkspaceFrame> {
        self.frames
            .iter()
            .filter(|frame| frame.record.seq > seq)
            .cloned()
            .collect()
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
    let export_output_dir = env::var_os("CAPSEM_NATIVE_EXPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/native-exports"));
    fs::create_dir_all(&export_output_dir).context("failed to create native export directory")?;
    let export_tool = ExportToolConfig {
        python: env::var_os("CAPSEM_EXPORT_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python3")),
        script: env::var_os("CAPSEM_EXPORT_SCRIPT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("scripts/export_artifact.py")),
        output_dir: export_output_dir,
    };
    let engine_mode = env::var("CAPSEM_PLUGIN_ENGINE").unwrap_or_else(|_| "placeholder".to_owned());
    let workspace_store = env::var_os("CAPSEM_NATIVE_WORKSPACE_DB")
        .map(|path| -> anyhow::Result<_> {
            let store = SqliteWorkspaceStore::open(PathBuf::from(path))
                .context("failed to open native workspace DB")?;
            let workspace = NativeWorkspace::from_restore(
                store
                    .restore(NATIVE_WORKSPACE_ID)
                    .context("failed to restore native workspace")?,
            );
            Ok((workspace, Some(Arc::new(Mutex::new(store)))))
        })
        .transpose()?
        .unwrap_or_else(|| (NativeWorkspace::default(), None));
    let (workspace_tx, _) = broadcast::channel(256);
    let state = AppState {
        registry: Arc::new(match engine_mode.as_str() {
            "wasmtime-wat" => PluginRegistry::wasmtime_wat(artifact_dir),
            "wasmtime-wat-fuel" => PluginRegistry::wasmtime_wat_fuel(artifact_dir),
            _ => PluginRegistry::new(artifact_dir),
        }),
        authored_ui: Arc::new(RwLock::new(None)),
        native_workspace: Arc::new(RwLock::new(workspace_store.0)),
        workspace_store: workspace_store.1,
        workspace_tx,
        telemetry: Arc::new(RwLock::new(Vec::new())),
        generation: Arc::new(
            GenerationEngine::from_capsem_environment()
                .context("failed to load Capsem AI settings")?,
        ),
        export_tool,
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
    let export_dir = state.export_tool.output_dir.clone();
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
        .route("/native/workspace/delete", post(native_workspace_delete))
        .route("/native/workspace/select", post(native_workspace_select))
        .route("/native/workspace/title", post(native_workspace_title))
        .route("/native/workspace/style", post(native_workspace_style))
        .route("/native/workspace/mutate", post(native_workspace_mutate))
        .route(
            "/native/workspace/checkpoint",
            post(native_workspace_checkpoint),
        )
        .route(
            "/native/workspace/change-request",
            post(native_workspace_change_request),
        )
        .route("/native/workspace/resolve", post(native_workspace_resolve))
        .route("/native/workspace/info", get(native_workspace_info))
        .route("/native/workspace/snapshot", get(native_workspace_snapshot))
        .route(
            "/native/workspace/projection",
            get(native_workspace_projection),
        )
        .route("/native/workspace/stream", get(native_workspace_stream))
        .route("/native/telemetry", get(native_telemetry))
        .route("/native/mcp/tools", get(native_mcp_tools))
        .route("/native/artifacts", get(native_artifacts))
        .route("/native/artifacts/:id", get(native_artifact))
        .route("/native/data/sqlite/query", post(native_sqlite_query))
        .route("/native/data/sheet", post(native_create_sheet))
        .route("/native/generate/text", post(native_generate_text))
        .route("/native/generate/image", post(native_generate_image))
        .route(
            "/native/generate/embedding",
            post(native_generate_embedding),
        )
        .route("/native/ui/table", post(native_create_table))
        .route("/native/ui/chart", post(native_create_chart))
        .route("/native/ui/diagram", post(native_create_diagram))
        .route("/native/ui/timeline", post(native_create_timeline))
        .route("/native/ui/slide", post(native_create_slide))
        .route("/native/ui/slide-deck", post(native_create_slide_deck))
        .route("/native/ui/render-artifact", post(native_render_artifact))
        .route("/native/ui/render-error", post(native_render_error))
        .route(
            "/native/export/spreadsheet",
            post(native_export_spreadsheet),
        )
        .route("/native/export/slide-deck", post(native_export_slide_deck))
        .route("/health", get(health))
        .route("/plugins/install", post(install_plugin))
        .route("/plugins/run", post(run_plugin))
        .route("/plugins/install-run", post(install_run_plugin))
        .nest_service("/assets", ServeDir::new("ui-preview/dist/assets"))
        .nest_service("/native/export/files", ServeDir::new(export_dir))
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

async fn native_workspace_reset(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, NativeApiError> {
    *state.native_workspace.write().await = NativeWorkspace::default();
    if let Some(store) = &state.workspace_store {
        store
            .lock()
            .await
            .clear_workspace(NATIVE_WORKSPACE_ID)
            .map_err(|err| NativeApiError::internal(err.to_string()))?;
    }
    state.telemetry.write().await.clear();
    Ok(Json(json!({
        "ok": true,
        "workspace": NATIVE_WORKSPACE_ID,
        "artifacts": 0
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTargetRequest {
    target: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSelectRequest {
    target: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTitleRequest {
    target: String,
    title: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceChangeRequestBody {
    target: String,
    instruction: String,
    #[serde(default)]
    annotation: Option<WorkspaceAnnotationTarget>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceResolveRequest {
    task_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceStyleRequest {
    target: String,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default, rename = "hostSelector")]
    host_selector: Option<String>,
    #[serde(default, rename = "shadowSelector")]
    shadow_selector: Option<String>,
    styles: BTreeMap<String, String>,
    #[serde(default, rename = "sourceRequestSeq")]
    source_request_seq: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WorkspaceMutateRequest {
    Title {
        target: String,
        title: String,
    },
    Text {
        target: String,
        text: String,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default, rename = "hostSelector")]
        host_selector: Option<String>,
        #[serde(default, rename = "shadowSelector")]
        shadow_selector: Option<String>,
        #[serde(default, rename = "sourceRequestSeq")]
        source_request_seq: Option<u64>,
    },
    Style {
        target: String,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default, rename = "hostSelector")]
        host_selector: Option<String>,
        #[serde(default, rename = "shadowSelector")]
        shadow_selector: Option<String>,
        styles: BTreeMap<String, String>,
        #[serde(default, rename = "sourceRequestSeq")]
        source_request_seq: Option<u64>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    artifact_id: String,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportJobResponse {
    ok: bool,
    artifact_id: String,
    format: String,
    adapter: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

async fn native_workspace_delete(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceTargetRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .delete(&request.target, "local.workspace.delete")
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_select(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceSelectRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .select(request.target, "local.workspace.select")
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_title(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceTitleRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .update_title(request.target, request.title, "local.workspace.title")
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_style(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceStyleRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .patch_style(
            request.target,
            ElementStylePatch {
                selector: request.selector,
                host_selector: request.host_selector,
                shadow_selector: request.shadow_selector,
                styles: request.styles,
                source_request_seq: request.source_request_seq,
            },
            "local.workspace.style",
        )
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_mutate(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceMutateRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = match request {
        WorkspaceMutateRequest::Title { target, title } => state
            .native_workspace
            .write()
            .await
            .update_title(target, title, "local.ui.mutate")
            .map_err(NativeApiError::bad_request)?,
        WorkspaceMutateRequest::Text {
            target,
            text,
            selector,
            host_selector,
            shadow_selector,
            source_request_seq,
        } => {
            if selector.is_some() || shadow_selector.is_some() {
                state
                    .native_workspace
                    .write()
                    .await
                    .patch_text(
                        target,
                        ElementTextPatch {
                            selector,
                            host_selector,
                            shadow_selector,
                            text,
                            source_request_seq,
                        },
                        "local.ui.mutate",
                    )
                    .map_err(NativeApiError::bad_request)?
            } else {
                state
                    .native_workspace
                    .write()
                    .await
                    .update_text(target, text, "local.ui.mutate")
                    .map_err(NativeApiError::bad_request)?
            }
        }
        WorkspaceMutateRequest::Style {
            target,
            selector,
            host_selector,
            shadow_selector,
            styles,
            source_request_seq,
        } => state
            .native_workspace
            .write()
            .await
            .patch_style(
                target,
                ElementStylePatch {
                    selector,
                    host_selector,
                    shadow_selector,
                    styles,
                    source_request_seq,
                },
                "local.ui.mutate",
            )
            .map_err(NativeApiError::bad_request)?,
    };
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_checkpoint(
    State(state): State<AppState>,
) -> Result<Json<WorkspaceCheckpoint>, NativeApiError> {
    let checkpoint = state.native_workspace.read().await.checkpoint();
    persist_workspace_checkpoint(&state, &checkpoint).await?;
    Ok(Json(checkpoint))
}

async fn native_workspace_change_request(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceChangeRequestBody>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .request_change(
            request.target,
            request.instruction,
            request.annotation,
            "chat.ui",
        )
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_resolve(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<Json<WorkspaceFrame>, NativeApiError> {
    let started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .resolve_task(request.task_id, "assistant.ui")
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(&state, &frame, started).await?;
    Ok(Json(frame))
}

async fn native_workspace_info(State(state): State<AppState>) -> Json<WorkspaceInfo> {
    Json(state.native_workspace.read().await.info())
}

async fn native_workspace_snapshot(State(state): State<AppState>) -> Json<WorkspaceSnapshot> {
    Json(state.native_workspace.read().await.snapshot())
}

async fn native_workspace_projection(State(state): State<AppState>) -> Json<RenderProjection> {
    Json(state.native_workspace.read().await.projection())
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceStreamQuery {
    #[serde(default)]
    last_seq: u64,
}

async fn native_workspace_stream(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceStreamQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| native_workspace_socket(state, query.last_seq, socket))
}

async fn native_workspace_socket(state: AppState, last_seq: u64, mut socket: WebSocket) {
    let (snapshot, replay) = {
        let workspace = state.native_workspace.read().await;
        (workspace.snapshot(), workspace.frames_after(last_seq))
    };
    let snapshot_message = WorkspaceStreamMessage::Snapshot { snapshot };
    if send_workspace_message(&mut socket, &snapshot_message)
        .await
        .is_err()
    {
        return;
    }
    for frame in replay {
        if send_workspace_message(&mut socket, &WorkspaceStreamMessage::Record { frame })
            .await
            .is_err()
        {
            return;
        }
    }

    let mut rx = state.workspace_tx.subscribe();
    while let Ok(message) = rx.recv().await {
        if send_workspace_message(&mut socket, &message).await.is_err() {
            break;
        }
    }
}

async fn send_workspace_message(
    socket: &mut WebSocket,
    message: &WorkspaceStreamMessage,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).expect("workspace stream message serializes");
    socket.send(Message::Text(text)).await
}

async fn native_telemetry(State(state): State<AppState>) -> Json<Vec<NativeTelemetryEvent>> {
    Json(state.telemetry.read().await.clone())
}

async fn native_mcp_tools() -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "namespace": "local",
        "tools": [
            tool_descriptor("local.workspace.reset", "local__workspace_reset", "POST", "/native/workspace/reset", "Reset the prototype workspace.", empty_schema()),
            tool_descriptor("local.workspace.snapshot", "local__workspace_snapshot", "GET", "/native/workspace/snapshot", "Return compact workspace state.", empty_schema()),
            tool_descriptor("local.workspace.stream", "local__workspace_stream", "GET", "/native/workspace/stream", "Subscribe to or replay workspace records.", json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {"lastSeq": {"type": "integer", "minimum": 0}}
            })),
            tool_descriptor("local.workspace.checkpoint", "local__workspace_checkpoint", "POST", "/native/workspace/checkpoint", "Compact records into a replay-safe checkpoint.", empty_schema()),
            tool_descriptor("local.ui.info", "local__ui_info", "GET", "/native/workspace/info", "Return topology, selected element, render elements, and tasks.", empty_schema()),
            tool_descriptor("local.ui.comment", "local__ui_comment", "POST", "/native/workspace/change-request", "Attach feedback to a topology target.", json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["target", "instruction"],
                "properties": {
                    "target": {"type": "string", "minLength": 1},
                    "instruction": {"type": "string", "minLength": 1},
                    "annotation": {"type": "object"}
                }
            })),
            tool_descriptor("local.ui.resolve", "local__ui_resolve", "POST", "/native/workspace/resolve", "Resolve a comment or task.", json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["taskId"],
                "properties": {"taskId": {"type": "string", "minLength": 1}}
            })),
            tool_descriptor("local.ui.mutate", "local__ui_mutate", "POST", "/native/workspace/mutate", "Apply typed UI/artifact mutations through the Rust allowlist.", json!({
                "oneOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["type", "target", "title"],
                        "properties": {
                            "type": {"const": "title"},
                            "target": {"type": "string", "minLength": 1},
                            "title": {"type": "string", "minLength": 1}
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["type", "target", "text"],
                        "properties": {
                            "type": {"const": "text"},
                            "target": {"type": "string", "minLength": 1},
                            "text": {"type": "string", "minLength": 1},
                            "selector": {"type": "string", "pattern": "data-capsem-(node|topology-id)="},
                            "hostSelector": {"type": "string"},
                            "shadowSelector": {"type": "string", "pattern": "data-capsem-(node|topology-id)="},
                            "sourceRequestSeq": {"type": "integer", "minimum": 0}
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["type", "target", "styles"],
                        "properties": {
                            "type": {"const": "style"},
                            "target": {"type": "string", "minLength": 1},
                            "selector": {"type": "string"},
                            "hostSelector": {"type": "string"},
                            "shadowSelector": {"type": "string"},
                            "styles": {
                                "type": "object",
                                "propertyNames": {
                                    "enum": ["backgroundColor", "color", "fontStyle", "fontWeight", "opacity", "textDecoration"]
                                },
                                "additionalProperties": {"type": "string", "minLength": 1, "maxLength": 120}
                            },
                            "sourceRequestSeq": {"type": "integer", "minimum": 0}
                        }
                    }
                ]
            })),
            tool_descriptor("local.data.sqlite", "local__data_sqlite", "POST", "/native/data/sqlite/query", "Use the per-instance SQLite workbench.", json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["sql"],
                "properties": {"sql": {"type": "string", "minLength": 1}}
            })),
            tool_descriptor("local.data.sheet", "local__data_sheet", "POST", "/native/data/sheet", "Create or update sheet artifacts.", sheet_schema()),
            tool_descriptor("local.data.spreadsheet", "local__data_spreadsheet", "POST", "/native/data/sheet", "Create or update spreadsheet artifacts.", sheet_schema()),
            tool_descriptor("local.export.spreadsheet", "local__export_spreadsheet", "POST", "/native/export/spreadsheet", "Export spreadsheet/sheet artifacts through the configured VM document toolchain.", export_schema(&["xlsx"])),
            tool_descriptor("local.export.slideDeck", "local__export_slide_deck", "POST", "/native/export/slide-deck", "Export slide deck artifacts through the configured VM document toolchain.", export_schema(&["pptx", "pdf", "html"])),
            deferred_tool_descriptor("local.export.chart", "Export charts through the authoritative Plotly browser renderer path."),
            deferred_tool_descriptor("local.export.diagram", "Export diagrams through the authoritative Mermaid browser renderer path."),
            deferred_tool_descriptor("local.export.pdf", "Export decks/reports to PDF after the office/PDF toolchain spike is selected."),
            tool_descriptor("local.generate.text", "local__generate_text", "POST", "/native/generate/text", "Generate text through capsem-ai.", generated_text_schema()),
            tool_descriptor("local.generate.image", "local__generate_image", "POST", "/native/generate/image", "Generate image through capsem-ai.", generated_prompt_schema()),
            deferred_tool_descriptor("local.generate.audio", "Generate audio when provider/config support is wired."),
            deferred_tool_descriptor("local.generate.video", "Generate video when provider/config support is wired."),
            tool_descriptor("local.generate.embedding", "local__generate_embedding", "POST", "/native/generate/embedding", "Generate embeddings through capsem-ai.", generated_embedding_schema()),
            tool_descriptor("local.ui.table", "local__ui_table", "POST", "/native/ui/table", "Create a table artifact.", table_schema()),
            tool_descriptor("local.ui.chart", "local__ui_chart", "POST", "/native/ui/chart", "Create a chart artifact.", chart_schema()),
            tool_descriptor("local.diagram.render", "local__diagram_render", "POST", "/native/ui/diagram", "Create and render diagram artifacts.", diagram_schema()),
            tool_descriptor("local.ui.timeline", "local__ui_timeline", "POST", "/native/ui/timeline", "Create structured timeline artifacts.", timeline_schema()),
            tool_descriptor("local.ui.slide", "local__ui_slide", "POST", "/native/ui/slide", "Create a slide artifact.", slide_schema()),
            tool_descriptor("local.ui.slideDeck", "local__ui_slide_deck", "POST", "/native/ui/slide-deck", "Create a slide deck artifact.", slide_deck_schema()),
            tool_descriptor("local.ui.render", "local__ui_render", "POST", "/native/ui/render-artifact", "Render checked artifact state into a component spec.", json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["artifactId"],
                "properties": {"artifactId": {"type": "string", "minLength": 1}}
            })),
            tool_descriptor("local.ui.renderError", "local__ui_render_error", "POST", "/native/ui/render-error", "Report browser or renderer failures for audit telemetry.", json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["message"],
                "properties": {
                    "message": {"type": "string", "minLength": 1},
                    "source": {"type": "string"},
                    "status": {"type": "string"},
                    "artifactId": {"type": "string"},
                    "component": {"type": "string"},
                    "renderer": {"type": "string"},
                    "phase": {"type": "string"}
                }
            })),
            deferred_tool_descriptor("local.website.render", "Render website/page artifacts."),
            deferred_tool_descriptor("local.website.form", "Create or update forms and validation."),
            deferred_tool_descriptor("local.web.preview", "Preview full websites/pages in a browser surface."),
            tool_descriptor("local.native.telemetry", "local__native_telemetry", "GET", "/native/telemetry", "Return prototype native telemetry.", empty_schema())
        ]
    }))
}

fn tool_descriptor(
    name: &str,
    adapter: &str,
    method: &str,
    route: &str,
    description: &str,
    input_schema: serde_json::Value,
) -> serde_json::Value {
    json!({
        "name": name,
        "adapter": adapter,
        "method": method,
        "route": route,
        "status": "implemented",
        "description": description,
        "inputSchema": input_schema
    })
}

fn deferred_tool_descriptor(name: &str, description: &str) -> serde_json::Value {
    json!({
        "name": name,
        "status": "deferred",
        "description": description,
        "inputSchema": empty_schema()
    })
}

fn empty_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn string_field() -> serde_json::Value {
    json!({"type": "string", "minLength": 1})
}

fn nullable_string_field() -> serde_json::Value {
    json!({"type": ["string", "null"]})
}

fn string_array_field() -> serde_json::Value {
    json!({
        "type": "array",
        "minItems": 1,
        "items": string_field()
    })
}

fn export_schema(formats: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["artifactId"],
        "properties": {
            "artifactId": string_field(),
            "format": {"enum": formats}
        }
    })
}

fn row_array_field() -> serde_json::Value {
    json!({
        "type": "array",
        "items": {"type": "object"}
    })
}

fn generated_prompt_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "prompt"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "prompt": string_field(),
            "caption": nullable_string_field(),
            "provider": string_field(),
            "model": nullable_string_field()
        }
    })
}

fn generated_text_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "prompt"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "prompt": string_field(),
            "system": nullable_string_field(),
            "provider": string_field(),
            "model": nullable_string_field()
        }
    })
}

fn generated_embedding_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "input"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "input": string_array_field(),
            "provider": string_field(),
            "model": nullable_string_field()
        }
    })
}

fn sheet_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "columns", "rows"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "columns": string_array_field(),
            "rows": row_array_field(),
            "source": {}
        }
    })
}

fn table_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "sourceArtifact", "columns", "rows"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "sourceArtifact": string_field(),
            "columns": string_array_field(),
            "rows": row_array_field(),
            "searchable": {"type": "boolean"},
            "filterable": {"type": "boolean"},
            "pageSize": {"type": "integer", "minimum": 1}
        }
    })
}

fn chart_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "chart", "sourceArtifact", "data", "x", "series", "xLabel", "yLabel", "yUnit"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "chart": {"enum": ["barChart", "lineChart", "heatmapChart", "boxPlot", "scatterPlot"]},
            "sourceArtifact": string_field(),
            "data": row_array_field(),
            "x": string_field(),
            "series": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "field"],
                    "properties": {
                        "name": string_field(),
                        "field": string_field(),
                        "axis": {"enum": ["left", "right"]}
                    }
                }
            },
            "xLabel": string_field(),
            "xUnit": nullable_string_field(),
            "yLabel": string_field(),
            "yUnit": string_field(),
            "stack": {"enum": ["none", "stacked", "grouped"]},
            "direction": {"enum": ["vertical", "horizontal"]},
            "legend": {"enum": ["top", "right", "bottom", "left", "none", null]},
            "secondAxis": {
                "type": ["object", "null"],
                "additionalProperties": false,
                "required": ["label", "unit"],
                "properties": {
                    "label": string_field(),
                    "unit": string_field()
                }
            },
            "fit": {
                "type": ["object", "null"],
                "additionalProperties": false,
                "required": ["method"],
                "properties": {
                    "method": {"enum": ["linear", "logarithmic", "movingAverage"]},
                    "display": {"type": "boolean"}
                }
            },
            "export": {
                "type": "array",
                "items": {"enum": ["png", "svg"]}
            }
        }
    })
}

fn diagram_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "source"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "kind": {"const": "mermaid"},
            "source": string_field(),
            "export": {"type": "array", "items": {"enum": ["svg", "png"]}}
        }
    })
}

fn timeline_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "lanes", "events"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "lanes": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "title"],
                    "properties": {
                        "id": string_field(),
                        "title": string_field()
                    }
                }
            },
            "events": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "title", "lane", "start"],
                    "properties": {
                        "id": string_field(),
                        "title": string_field(),
                        "lane": string_field(),
                        "start": string_field(),
                        "end": nullable_string_field(),
                        "description": nullable_string_field()
                    }
                }
            },
            "export": {"type": "array", "items": {"enum": ["html", "png", "svg"]}}
        }
    })
}

fn slide_block_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "title", "body"],
                "properties": {
                    "kind": {"const": "text"},
                    "title": string_field(),
                    "body": string_field()
                }
            },
            artifact_ref_block_schema("image"),
            artifact_ref_block_schema("diagram"),
            artifact_ref_block_schema("table"),
            artifact_ref_block_schema("sheet"),
            artifact_ref_block_schema("chart")
        ]
    })
}

fn artifact_ref_block_schema(kind: &str) -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "artifactId"],
        "properties": {
            "kind": {"const": kind},
            "artifactId": string_field()
        }
    })
}

fn slide_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "blocks"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "blocks": {
                "type": "array",
                "minItems": 1,
                "items": slide_block_schema()
            }
        }
    })
}

fn slide_ref_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["artifactId", "title"],
        "properties": {
            "artifactId": string_field(),
            "title": string_field()
        }
    })
}

fn slide_deck_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "slides"],
        "properties": {
            "id": string_field(),
            "title": string_field(),
            "slides": {
                "type": "array",
                "minItems": 1,
                "items": slide_ref_schema()
            },
            "export": {"type": "array", "items": {"enum": ["html", "pdf"]}}
        }
    })
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

async fn native_generate_embedding(
    State(state): State<AppState>,
    Json(request): Json<GenerateEmbeddingRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        generate_embedding_provider(request, &state.generation)
            .await
            .map_err(NativeApiError::bad_request)?,
        "generate.embedding",
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

async fn native_create_timeline(
    State(state): State<AppState>,
    Json(request): Json<TimelineRequest>,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let started = Instant::now();
    store_artifact(
        &state,
        create_timeline(request).map_err(NativeApiError::bad_request)?,
        "ui.timeline",
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
    let started = Instant::now();
    let workspace = state.native_workspace.read().await;
    let artifact = match workspace
        .artifact(&request.artifact_id)
        .cloned()
        .map(Some)
        .unwrap_or_else(|| artifact_by_id(&request.artifact_id).ok().flatten())
    {
        Some(artifact) => artifact,
        None => {
            let message = format!("artifact not found: {}", request.artifact_id);
            audit_render_error(
                &state,
                RenderErrorTelemetry {
                    source: "server".to_owned(),
                    status: "notFound".to_owned(),
                    message: message.clone(),
                    artifact_id: Some(request.artifact_id),
                    component: None,
                    renderer: None,
                    phase: Some("renderArtifact".to_owned()),
                    duration_ms: started.elapsed().as_millis(),
                },
            )
            .await;
            return Err(NativeApiError::not_found(message));
        }
    };
    Ok(Json(RenderArtifactResponse {
        ok: true,
        component: artifact.spec["component"]
            .as_str()
            .unwrap_or("capsem-elt")
            .to_owned(),
        artifact,
    }))
}

async fn native_render_error(
    State(state): State<AppState>,
    Json(request): Json<RenderErrorRequest>,
) -> Result<Json<serde_json::Value>, NativeApiError> {
    let message = request.message.trim().to_owned();
    if message.is_empty() {
        return Err(NativeApiError::bad_request(
            "render error message must not be empty".to_owned(),
        ));
    }
    audit_render_error(
        &state,
        RenderErrorTelemetry {
            source: request.source.unwrap_or_else(|| "browser".to_owned()),
            status: request.status.unwrap_or_else(|| "failed".to_owned()),
            message,
            artifact_id: request.artifact_id,
            component: request.component,
            renderer: request.renderer,
            phase: request.phase,
            duration_ms: 0,
        },
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn native_export_spreadsheet(
    State(state): State<AppState>,
    Json(request): Json<ExportRequest>,
) -> Result<Json<ExportJobResponse>, NativeApiError> {
    export_workspace_artifact(
        &state,
        request,
        "xlsx",
        &[NativeArtifactKind::Sheet, NativeArtifactKind::Table],
        "spreadsheet",
        "export.spreadsheet",
    )
    .await
}

async fn native_export_slide_deck(
    State(state): State<AppState>,
    Json(request): Json<ExportRequest>,
) -> Result<Json<ExportJobResponse>, NativeApiError> {
    export_workspace_artifact(
        &state,
        request,
        "pptx",
        &[NativeArtifactKind::SlideDeck],
        "slideDeck",
        "export.slideDeck",
    )
    .await
}

async fn export_workspace_artifact(
    state: &AppState,
    request: ExportRequest,
    default_format: &str,
    allowed_kinds: &[NativeArtifactKind],
    export_kind: &str,
    operation: &str,
) -> Result<Json<ExportJobResponse>, NativeApiError> {
    let started = Instant::now();
    let artifact_id = request.artifact_id.trim().to_owned();
    if artifact_id.is_empty() {
        return Err(NativeApiError::bad_request(
            "artifactId must not be empty".to_owned(),
        ));
    }
    let format = request
        .format
        .unwrap_or_else(|| default_format.to_owned())
        .trim()
        .to_owned();
    if format.is_empty() {
        return Err(NativeApiError::bad_request(
            "format must not be empty".to_owned(),
        ));
    }
    let (artifact, artifacts) = {
        let workspace = state.native_workspace.read().await;
        let artifact = workspace.artifact(&artifact_id).cloned().ok_or_else(|| {
            NativeApiError::not_found(format!("artifact not found: {artifact_id}"))
        })?;
        if !allowed_kinds.contains(&artifact.kind) {
            return Err(NativeApiError::bad_request(format!(
                "{operation} cannot export {:?} artifacts",
                artifact.kind
            )));
        }
        (artifact, workspace.artifacts())
    };
    let result = run_export_tool(
        &state.export_tool,
        export_kind,
        &format,
        artifact,
        artifacts,
    )?;
    audit_export(state, operation, &result, started).await;
    Ok(Json(result))
}

fn run_export_tool(
    config: &ExportToolConfig,
    export_kind: &str,
    format: &str,
    artifact: NativeArtifact,
    artifacts: Vec<NativeArtifact>,
) -> Result<ExportJobResponse, NativeApiError> {
    fs::create_dir_all(&config.output_dir)
        .map_err(|err| NativeApiError::internal(err.to_string()))?;
    if !config.script.exists() {
        return Ok(export_tool_unavailable(
            &artifact.id,
            format,
            &format!(
                "export adapter script not found: {}",
                config.script.display()
            ),
        ));
    }
    let job_id = format!(
        "{}-{}-{}",
        sanitize_file_component(&artifact.id),
        sanitize_file_component(format),
        now_millis()
    );
    let input_path = config.output_dir.join(format!("{job_id}.input.json"));
    let input = json!({
        "kind": export_kind,
        "format": format,
        "artifact": artifact,
        "artifacts": artifacts,
        "outputDir": config.output_dir.to_string_lossy().to_string(),
        "jobId": job_id
    });
    let input_bytes = serde_json::to_vec_pretty(&input)
        .map_err(|err| NativeApiError::internal(err.to_string()))?;
    fs::write(&input_path, input_bytes).map_err(|err| NativeApiError::internal(err.to_string()))?;
    let output = Command::new(&config.python)
        .arg(&config.script)
        .arg(&input_path)
        .output()
        .map_err(|err| {
            NativeApiError::internal(format!(
                "failed to launch export adapter {}: {err}",
                config.python.display()
            ))
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result: ExportJobResponse = serde_json::from_str(stdout.trim()).map_err(|err| {
        NativeApiError::internal(format!(
            "export adapter returned invalid JSON: {err}; stderr={stderr}"
        ))
    })?;
    if !output.status.success() && result.status == "complete" {
        result.ok = false;
        result.status = "failed".to_owned();
        result.message = Some(stderr.trim().to_owned());
    }
    if let Some(path) = result.file_path.as_deref() {
        result.file_url = export_file_url(&config.output_dir, path);
    }
    Ok(result)
}

fn export_tool_unavailable(artifact_id: &str, format: &str, message: &str) -> ExportJobResponse {
    ExportJobResponse {
        ok: false,
        artifact_id: artifact_id.to_owned(),
        format: format.to_owned(),
        adapter: "python.export_artifact".to_owned(),
        status: "toolUnavailable".to_owned(),
        file_path: None,
        file_url: None,
        mime_type: None,
        bytes: None,
        message: Some(message.to_owned()),
    }
}

async fn audit_export(
    state: &AppState,
    operation: &str,
    result: &ExportJobResponse,
    started: Instant,
) {
    state
        .telemetry
        .write()
        .await
        .push(NativeTelemetryEvent::Export {
            operation: operation.to_owned(),
            timestamp: now_timestamp(),
            artifact_id: result.artifact_id.clone(),
            format: result.format.clone(),
            adapter: result.adapter.clone(),
            status: result.status.clone(),
            duration_ms: started.elapsed().as_millis(),
            file_path: result.file_path.clone(),
            bytes: result.bytes,
            message: result.message.clone(),
        });
}

fn export_file_url(output_dir: &std::path::Path, file_path: &str) -> Option<String> {
    let path = PathBuf::from(file_path);
    let name = path
        .strip_prefix(output_dir)
        .ok()
        .and_then(|relative| relative.file_name())
        .or_else(|| path.file_name())?
        .to_string_lossy();
    Some(format!("/native/export/files/{name}"))
}

fn sanitize_file_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').chars().take(80).collect()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

async fn store_artifact(
    state: &AppState,
    artifact: NativeArtifact,
    operation: &str,
    started: Instant,
) -> Result<Json<NativeArtifact>, NativeApiError> {
    let workspace_started = Instant::now();
    let frame = state
        .native_workspace
        .write()
        .await
        .insert(artifact.clone(), &format!("local.{operation}"))
        .map_err(NativeApiError::bad_request)?;
    publish_audited_workspace_frame(state, &frame, workspace_started).await?;
    state
        .telemetry
        .write()
        .await
        .push(NativeTelemetryEvent::Artifact {
            operation: operation.to_owned(),
            artifact_id: artifact.id.clone(),
            kind: artifact.kind,
            duration_ms: started.elapsed().as_millis(),
            status: artifact.spec["status"].as_str().unwrap_or("ok").to_owned(),
            usage: artifact.spec.get("usage").cloned(),
            cost: artifact.spec.get("cost").cloned(),
        });
    Ok(Json(artifact))
}

async fn publish_audited_workspace_frame(
    state: &AppState,
    frame: &WorkspaceFrame,
    started: Instant,
) -> Result<(), NativeApiError> {
    persist_workspace_frame(state, frame).await?;
    publish_workspace_frame(state, frame.clone());
    state
        .telemetry
        .write()
        .await
        .push(workspace_telemetry_event(
            frame,
            started.elapsed().as_millis(),
        ));
    Ok(())
}

fn publish_workspace_frame(state: &AppState, frame: WorkspaceFrame) {
    let _ = state
        .workspace_tx
        .send(WorkspaceStreamMessage::Record { frame });
}

fn workspace_telemetry_event(frame: &WorkspaceFrame, duration_ms: u128) -> NativeTelemetryEvent {
    let record = &frame.record;
    NativeTelemetryEvent::Workspace {
        operation: workspace_operation(record.verb).to_owned(),
        seq: record.seq,
        record_id: record.id.clone(),
        timestamp: record.timestamp.clone(),
        role: record.role,
        principal: record.principal.clone(),
        title: record.title.clone(),
        content_type: record.content_type,
        verb: record.verb,
        status: record.status,
        target: record.target.clone(),
        duration_ms,
        delta_count: frame.deltas.len(),
        deltas: frame.deltas.iter().map(workspace_delta_name).collect(),
        task_id: workspace_task_id(frame),
        mutation: workspace_mutation_summary(&record.content),
    }
}

fn workspace_operation(verb: WorkspaceVerb) -> &'static str {
    match verb {
        WorkspaceVerb::Create => "workspace.create",
        WorkspaceVerb::Append => "workspace.append",
        WorkspaceVerb::Replace => "workspace.replace",
        WorkspaceVerb::Patch => "workspace.patch",
        WorkspaceVerb::Delete => "workspace.delete",
        WorkspaceVerb::Select => "workspace.select",
        WorkspaceVerb::Request => "workspace.request",
        WorkspaceVerb::Respond => "workspace.respond",
    }
}

fn workspace_delta_name(delta: &RenderDelta) -> String {
    match delta {
        RenderDelta::UpsertElement { .. } => "upsertElement",
        RenderDelta::DeleteElement { .. } => "deleteElement",
        RenderDelta::UpsertTopologyNode { .. } => "upsertTopologyNode",
        RenderDelta::DeleteTopologyNode { .. } => "deleteTopologyNode",
        RenderDelta::UpsertTask { .. } => "upsertTask",
        RenderDelta::Select { .. } => "select",
    }
    .to_owned()
}

fn workspace_task_id(frame: &WorkspaceFrame) -> Option<String> {
    frame.deltas.iter().find_map(|delta| match delta {
        RenderDelta::UpsertTask { id, .. } => Some(id.clone()),
        _ => None,
    })
}

fn workspace_mutation_summary(content: &WorkspaceContent) -> Option<serde_json::Value> {
    match content {
        WorkspaceContent::Artifact { artifact } => Some(json!({
            "artifactId": artifact.id,
            "artifactKind": artifact.kind,
        })),
        WorkspaceContent::Text { text } => Some(json!({
            "textBytes": text.len(),
        })),
        WorkspaceContent::Ui { spec } => Some(json!({
            "component": spec.get("component").cloned(),
        })),
        WorkspaceContent::ToolCall {
            name,
            input,
            output,
        } => Some(json!({
            "name": name,
            "inputKeys": input.as_object().map(|object| object.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "hasOutput": output.is_some(),
        })),
        WorkspaceContent::Action { name, payload } => Some(json!({
            "name": name,
            "payload": payload,
        })),
        WorkspaceContent::ElementPatch {
            title,
            text,
            text_patches,
            style_patches,
        } => Some(json!({
            "title": title.as_ref().map(|value| !value.is_empty()).unwrap_or(false),
            "text": text.as_ref().map(|value| !value.is_empty()).unwrap_or(false),
            "textPatchCount": text_patches.len(),
            "stylePatchCount": style_patches.len(),
        })),
        WorkspaceContent::Status { message } => Some(json!({
            "messageBytes": message.len(),
        })),
        WorkspaceContent::Error { message } => Some(json!({
            "messageBytes": message.len(),
        })),
    }
}

struct RenderErrorTelemetry {
    source: String,
    status: String,
    message: String,
    artifact_id: Option<String>,
    component: Option<String>,
    renderer: Option<String>,
    phase: Option<String>,
    duration_ms: u128,
}

async fn audit_render_error(state: &AppState, event: RenderErrorTelemetry) {
    state
        .telemetry
        .write()
        .await
        .push(NativeTelemetryEvent::RenderError {
            operation: "render.error".to_owned(),
            timestamp: now_timestamp(),
            source: event.source,
            status: event.status,
            duration_ms: event.duration_ms,
            message: event.message,
            artifact_id: event.artifact_id,
            component: event.component,
            renderer: event.renderer,
            phase: event.phase,
        });
}

async fn persist_workspace_frame(
    state: &AppState,
    frame: &WorkspaceFrame,
) -> Result<(), NativeApiError> {
    if let Some(store) = &state.workspace_store {
        store
            .lock()
            .await
            .append_record(NATIVE_WORKSPACE_ID, &frame.record)
            .map_err(|err| NativeApiError::internal(err.to_string()))?;
    }
    Ok(())
}

async fn persist_workspace_checkpoint(
    state: &AppState,
    checkpoint: &WorkspaceCheckpoint,
) -> Result<(), NativeApiError> {
    if let Some(store) = &state.workspace_store {
        store
            .lock()
            .await
            .save_checkpoint(NATIVE_WORKSPACE_ID, checkpoint)
            .map_err(|err| NativeApiError::internal(err.to_string()))?;
    }
    Ok(())
}

fn now_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{millis}")
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
            artifact.spec["model"] = json!(output.model);
            artifact.spec["providerModel"] = json!(output.model);
            artifact.spec["text"] = json!(output.text);
            artifact.spec["usage"] = json!(output.usage);
            artifact.spec["cost"] = json!(output.cost);
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
            artifact.spec["model"] = json!(output.model);
            artifact.spec["providerModel"] = json!(output.model);
            artifact.spec["usage"] = json!(output.usage);
            artifact.spec["cost"] = json!(output.cost);
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

async fn generate_embedding_provider(
    request: GenerateEmbeddingRequest,
    engine: &GenerationEngine<impl ModelProvider>,
) -> Result<NativeArtifact, String> {
    let mut artifact = generate_embedding(request.clone())?;
    match engine
        .generate_embedding(capsem_ai::GenerateEmbeddingRequest {
            provider: request.provider,
            model: request.model,
            input: request.input,
        })
        .await
    {
        Ok(output) => {
            artifact.spec["status"] = json!("generated");
            artifact.spec["provider"] = json!(output.provider);
            artifact.spec["model"] = json!(output.model);
            artifact.spec["providerModel"] = json!(output.model);
            artifact.spec["dimensions"] = json!(output.dimensions);
            artifact.spec["vectors"] = json!(output.vectors);
            artifact.spec["usage"] = json!(output.usage);
            artifact.spec["cost"] = json!(output.cost);
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderErrorRequest {
    message: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    artifact_id: Option<String>,
    #[serde(default)]
    component: Option<String>,
    #[serde(default)]
    renderer: Option<String>,
    #[serde(default)]
    phase: Option<String>,
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

#[derive(Debug)]
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

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
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
    use std::collections::BTreeSet;

    use capsem_ai::{CapsemHttpModelProvider, FakeModelProvider, GenerationSettings};
    use capsem_ui_catalog::contract_matrix::{Coverage, LOCAL_TOOL_COVERAGE};

    #[tokio::test]
    async fn text_generation_artifact_uses_capsem_ai_engine() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty()
                .with_credential("google-api-key", "AIza-test")
                .with_model("google", "text", "gemini-3.5-flash"),
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
        assert_eq!(artifact.spec["usage"]["totalTokens"], 6);
        assert_eq!(artifact.spec["cost"]["currency"], "USD");
    }

    fn test_state() -> AppState {
        let (workspace_tx, _) = broadcast::channel(16);
        AppState {
            registry: Arc::new(PluginRegistry::new(std::env::temp_dir())),
            authored_ui: Arc::new(RwLock::new(None)),
            native_workspace: Arc::new(RwLock::new(NativeWorkspace::default())),
            workspace_store: None,
            workspace_tx,
            telemetry: Arc::new(RwLock::new(Vec::new())),
            generation: Arc::new(GenerationEngine::new(
                GenerationSettings::empty(),
                CapsemHttpModelProvider::default(),
            )),
            export_tool: ExportToolConfig {
                python: PathBuf::from("python3"),
                script: PathBuf::from("scripts/export_artifact.py"),
                output_dir: std::env::temp_dir().join("capsem-plugin-server-test-exports"),
            },
        }
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf()
    }

    #[test]
    fn workspace_mutate_style_deserializes_camel_case_selector_fields() {
        let request: WorkspaceMutateRequest = serde_json::from_value(json!({
            "type": "style",
            "target": "house-table",
            "selector": "capsem-sheet[data-capsem-artifact-id=\"house-table\"] >>> [data-capsem-node=\"13\"]",
            "hostSelector": "capsem-sheet[data-capsem-artifact-id=\"house-table\"]",
            "shadowSelector": "[data-capsem-node=\"13\"]",
            "styles": {
                "fontWeight": "700"
            },
            "sourceRequestSeq": 4
        }))
        .expect("style mutate request");

        match request {
            WorkspaceMutateRequest::Style {
                target,
                selector,
                host_selector,
                shadow_selector,
                styles,
                source_request_seq,
            } => {
                assert_eq!(target, "house-table");
                assert_eq!(
                    selector.as_deref(),
                    Some("capsem-sheet[data-capsem-artifact-id=\"house-table\"] >>> [data-capsem-node=\"13\"]")
                );
                assert_eq!(
                    host_selector.as_deref(),
                    Some("capsem-sheet[data-capsem-artifact-id=\"house-table\"]")
                );
                assert_eq!(
                    shadow_selector.as_deref(),
                    Some("[data-capsem-node=\"13\"]")
                );
                assert_eq!(styles["fontWeight"], "700");
                assert_eq!(source_request_seq, Some(4));
            }
            other => panic!("expected style request, got {other:?}"),
        }
    }

    #[test]
    fn workspace_mutate_text_deserializes_camel_case_selector_fields() {
        let request: WorkspaceMutateRequest = serde_json::from_value(json!({
            "type": "text",
            "target": "generated-image",
            "selector": "capsem-media[data-capsem-artifact-id=\"generated-image\"] >>> [data-capsem-node=\"generated-image::caption\"]",
            "hostSelector": "capsem-media[data-capsem-artifact-id=\"generated-image\"]",
            "shadowSelector": "[data-capsem-node=\"generated-image::caption\"]",
            "text": "Sharper caption",
            "sourceRequestSeq": 9
        }))
        .expect("text mutate request");

        match request {
            WorkspaceMutateRequest::Text {
                target,
                text,
                selector,
                host_selector,
                shadow_selector,
                source_request_seq,
            } => {
                assert_eq!(target, "generated-image");
                assert_eq!(text, "Sharper caption");
                assert_eq!(
                    selector.as_deref(),
                    Some("capsem-media[data-capsem-artifact-id=\"generated-image\"] >>> [data-capsem-node=\"generated-image::caption\"]")
                );
                assert_eq!(
                    host_selector.as_deref(),
                    Some("capsem-media[data-capsem-artifact-id=\"generated-image\"]")
                );
                assert_eq!(
                    shadow_selector.as_deref(),
                    Some("[data-capsem-node=\"generated-image::caption\"]")
                );
                assert_eq!(source_request_seq, Some(9));
            }
            other => panic!("expected text request, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn embedding_generation_artifact_uses_capsem_ai_engine() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty()
                .with_credential("openai-api-key", "sk-test")
                .with_model("openai", "embedding", "text-embedding-3-small"),
            FakeModelProvider,
        );
        let artifact = generate_embedding_provider(
            GenerateEmbeddingRequest {
                id: "embedding-test".to_owned(),
                title: "Embedding Test".to_owned(),
                input: vec!["review this".to_owned()],
                provider: "openai".to_owned(),
                model: None,
            },
            &engine,
        )
        .await
        .unwrap();

        assert_eq!(artifact.kind, NativeArtifactKind::GeneratedEmbedding);
        assert_eq!(artifact.spec["status"], "generated");
        assert_eq!(artifact.spec["provider"], "openai");
        assert_eq!(artifact.spec["dimensions"], 3);
        assert_eq!(artifact.spec["vectors"].as_array().unwrap().len(), 1);
        assert_eq!(artifact.spec["usage"]["promptTokens"], 4);
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

    #[test]
    fn workspace_info_reports_topology_and_provenance() {
        let mut workspace = NativeWorkspace::default();
        let artifact = create_table(TableRequest {
            id: "table-info-test".to_owned(),
            title: "Original Table Title".to_owned(),
            source_artifact: "manual-proof".to_owned(),
            columns: vec!["name".to_owned(), "value".to_owned()],
            rows: vec![BTreeMap::from([
                ("name".to_owned(), json!("topology")),
                ("value".to_owned(), json!("roots")),
            ])],
            searchable: true,
            filterable: true,
            page_size: 10,
        })
        .expect("table artifact");
        workspace
            .insert(artifact, "local.ui.table")
            .expect("insert artifact");
        workspace
            .update_title(
                "table-info-test".to_owned(),
                "Edited Table Title".to_owned(),
                "local.workspace.title",
            )
            .expect("title patch");
        workspace
            .request_change(
                "table-info-test".to_owned(),
                "add a summary row".to_owned(),
                Some(WorkspaceAnnotationTarget {
                    kind: "tableRow".to_owned(),
                    label: "Row 1".to_owned(),
                    path: vec!["table".to_owned(), "row:0".to_owned()],
                    topology_id: Some("table-info-test::row::0".to_owned()),
                    topology_role: Some("row".to_owned()),
                    selector: Some(
                        "capsem-sheet >>> table[part~=\"table\"] > tbody > tr:nth-child(1)"
                            .to_owned(),
                    ),
                    host_selector: Some("capsem-sheet".to_owned()),
                    shadow_selector: Some(
                        "table[part~=\"table\"] > tbody > tr:nth-child(1)".to_owned(),
                    ),
                    selector_verified: Some(true),
                    metadata: Some(json!({
                        "tag": "tr",
                        "capsemNode": "6"
                    })),
                }),
                "chat.ui",
            )
            .expect("change request");

        let info = workspace.info();
        let element = info
            .elements
            .iter()
            .find(|element| element.id == "table-info-test")
            .expect("info element");

        assert_eq!(info.topology.roots, vec!["table-info-test"]);
        assert_eq!(element.title, "Edited Table Title");
        assert_eq!(element.artifact_kind, Some(NativeArtifactKind::Table));
        assert!(element
            .artifact_handle
            .as_deref()
            .unwrap()
            .starts_with("capsem://artifact/"));
        assert_eq!(element.provenance.created_by, "local.ui.table");
        assert_eq!(element.provenance.updated_by, "local.workspace.title");
        assert_eq!(info.change_requests.len(), 1);
        assert_eq!(info.change_requests[0].target, "table-info-test");
        assert_eq!(info.change_requests[0].instruction, "add a summary row");
        assert_eq!(
            info.change_requests[0]
                .annotation
                .as_ref()
                .expect("annotation")
                .kind,
            "tableRow"
        );
        assert_eq!(
            info.change_requests[0]
                .annotation
                .as_ref()
                .expect("annotation")
                .topology_id
                .as_deref(),
            Some("table-info-test::row::0")
        );
        assert_eq!(
            info.change_requests[0]
                .annotation
                .as_ref()
                .expect("annotation")
                .topology_role
                .as_deref(),
            Some("row")
        );
        assert_eq!(info.change_requests[0].principal, "chat.ui");
    }

    #[test]
    fn native_workspace_restores_from_runtime_sqlite_store() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let db_path = tempdir.path().join("native-workspace.sqlite");
        let store = SqliteWorkspaceStore::open(&db_path).expect("store");
        let mut workspace = NativeWorkspace::default();
        let artifact = create_table(TableRequest {
            id: "table-restore-test".to_owned(),
            title: "Restore Table".to_owned(),
            source_artifact: "manual-proof".to_owned(),
            columns: vec!["name".to_owned(), "value".to_owned()],
            rows: vec![BTreeMap::from([
                ("name".to_owned(), json!("restart")),
                ("value".to_owned(), json!("durable")),
            ])],
            searchable: true,
            filterable: true,
            page_size: 10,
        })
        .expect("table artifact");
        let frame = workspace
            .insert(artifact, "local.ui.table")
            .expect("insert artifact");
        store
            .append_record(NATIVE_WORKSPACE_ID, &frame.record)
            .expect("insert persists");
        let frame = workspace
            .request_change(
                "table-restore-test".to_owned(),
                "tighten the copy".to_owned(),
                None,
                "chat.ui",
            )
            .expect("change request");
        store
            .append_record(NATIVE_WORKSPACE_ID, &frame.record)
            .expect("request persists");
        store
            .save_checkpoint(NATIVE_WORKSPACE_ID, &workspace.checkpoint())
            .expect("checkpoint persists");

        let restored = NativeWorkspace::from_restore(
            SqliteWorkspaceStore::open(&db_path)
                .expect("reopen")
                .restore(NATIVE_WORKSPACE_ID)
                .expect("restore"),
        );
        let info = restored.info();

        assert_eq!(restored.next_seq, 2);
        assert!(restored.artifact("table-restore-test").is_some());
        assert_eq!(info.elements.len(), 1);
        assert_eq!(info.change_requests.len(), 1);
        assert_eq!(info.change_requests[0].instruction, "tighten the copy");
        assert_eq!(info.change_requests[0].status, WorkspaceStatus::Pending);
    }

    #[test]
    fn native_workspace_resolve_updates_task_info() {
        let mut workspace = NativeWorkspace::default();
        let artifact = create_table(TableRequest {
            id: "table-resolve-test".to_owned(),
            title: "Resolve Table".to_owned(),
            source_artifact: "manual-proof".to_owned(),
            columns: vec!["name".to_owned(), "value".to_owned()],
            rows: vec![BTreeMap::from([
                ("name".to_owned(), json!("comment")),
                ("value".to_owned(), json!("resolve")),
            ])],
            searchable: true,
            filterable: true,
            page_size: 10,
        })
        .expect("table artifact");
        workspace
            .insert(artifact, "local.ui.table")
            .expect("insert artifact");
        workspace
            .request_change(
                "table-resolve-test".to_owned(),
                "make the header shorter".to_owned(),
                None,
                "chat.ui",
            )
            .expect("change request");
        let task_id = workspace.info().change_requests[0].id.clone();

        let frame = workspace
            .resolve_task(task_id.clone(), "assistant.ui")
            .expect("resolve task");
        let info = workspace.info();

        assert_eq!(frame.record.verb, WorkspaceVerb::Respond);
        assert_eq!(info.change_requests.len(), 1);
        assert_eq!(info.change_requests[0].id, task_id);
        assert_eq!(info.change_requests[0].status, WorkspaceStatus::Complete);
    }

    #[test]
    fn workspace_telemetry_event_captures_task_record_contract() {
        let mut workspace = NativeWorkspace::default();
        let artifact = create_table(TableRequest {
            id: "table-telemetry-test".to_owned(),
            title: "Telemetry Table".to_owned(),
            source_artifact: "manual-proof".to_owned(),
            columns: vec!["name".to_owned(), "value".to_owned()],
            rows: vec![BTreeMap::from([
                ("name".to_owned(), json!("audit")),
                ("value".to_owned(), json!("workspace")),
            ])],
            searchable: true,
            filterable: true,
            page_size: 10,
        })
        .expect("table artifact");
        workspace
            .insert(artifact, "local.ui.table")
            .expect("insert artifact");

        let frame = workspace
            .request_change(
                "table-telemetry-test".to_owned(),
                "tighten the heading".to_owned(),
                None,
                "chat.ui",
            )
            .expect("change request");
        let task_id = workspace.info().change_requests[0].id.clone();
        let value = serde_json::to_value(workspace_telemetry_event(&frame, 7))
            .expect("telemetry serializes");

        assert_eq!(value["type"], "workspace");
        assert_eq!(value["operation"], "workspace.request");
        assert_eq!(value["seq"], 2);
        assert_eq!(value["recordId"], frame.record.id);
        assert_eq!(value["role"], "user");
        assert_eq!(value["principal"], "chat.ui");
        assert_eq!(value["verb"], "request");
        assert_eq!(value["contentType"], "action");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["target"], "table-telemetry-test");
        assert_eq!(value["durationMs"], 7);
        assert_eq!(value["deltaCount"], 1);
        assert_eq!(value["deltas"], json!(["upsertTask"]));
        assert_eq!(value["taskId"], task_id);
        assert_eq!(value["mutation"]["name"], "ui.change");
        assert_eq!(
            value["mutation"]["payload"]["instruction"],
            "tighten the heading"
        );
    }

    #[tokio::test]
    async fn publish_audited_workspace_frame_records_workspace_telemetry() {
        let state = test_state();
        let artifact = create_table(TableRequest {
            id: "table-publish-test".to_owned(),
            title: "Publish Table".to_owned(),
            source_artifact: "manual-proof".to_owned(),
            columns: vec!["name".to_owned(), "value".to_owned()],
            rows: vec![BTreeMap::from([
                ("name".to_owned(), json!("publish")),
                ("value".to_owned(), json!("audit")),
            ])],
            searchable: true,
            filterable: true,
            page_size: 10,
        })
        .expect("table artifact");
        let frame = state
            .native_workspace
            .write()
            .await
            .insert(artifact, "local.ui.table")
            .expect("insert artifact");

        publish_audited_workspace_frame(&state, &frame, Instant::now())
            .await
            .expect("publish audited frame");
        let events = state.telemetry.read().await.clone();
        let value = serde_json::to_value(&events[0]).expect("telemetry serializes");

        assert_eq!(events.len(), 1);
        assert_eq!(value["type"], "workspace");
        assert_eq!(value["operation"], "workspace.create");
        assert_eq!(value["seq"], 1);
        assert_eq!(value["principal"], "local.ui.table");
        assert_eq!(value["contentType"], "artifact");
        assert_eq!(value["mutation"]["artifactId"], "table-publish-test");
        assert_eq!(value["mutation"]["artifactKind"], "table");
    }

    #[tokio::test]
    async fn native_render_error_records_render_telemetry() {
        let state = test_state();

        let _ = native_render_error(
            State(state.clone()),
            Json(RenderErrorRequest {
                message: "plotly failed to render".to_owned(),
                source: Some("browser".to_owned()),
                status: Some("failed".to_owned()),
                artifact_id: Some("chart-render-error".to_owned()),
                component: Some("capsem-chart".to_owned()),
                renderer: Some("plotly".to_owned()),
                phase: Some("hydrate".to_owned()),
            }),
        )
        .await
        .expect("render error accepted");

        let events = state.telemetry.read().await.clone();
        let value = serde_json::to_value(&events[0]).expect("telemetry serializes");

        assert_eq!(events.len(), 1);
        assert_eq!(value["type"], "renderError");
        assert_eq!(value["operation"], "render.error");
        assert_eq!(value["source"], "browser");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["artifactId"], "chart-render-error");
        assert_eq!(value["component"], "capsem-chart");
        assert_eq!(value["renderer"], "plotly");
        assert_eq!(value["phase"], "hydrate");
        assert_eq!(value["message"], "plotly failed to render");
    }

    #[tokio::test]
    async fn native_render_artifact_missing_records_render_error() {
        let state = test_state();
        let error = native_render_artifact(
            State(state.clone()),
            Json(RenderArtifactRequest {
                artifact_id: "missing-artifact".to_owned(),
            }),
        )
        .await
        .expect_err("missing artifact should fail");
        let events = state.telemetry.read().await.clone();
        let value = serde_json::to_value(&events[0]).expect("telemetry serializes");

        assert_eq!(error.status, StatusCode::NOT_FOUND);
        assert_eq!(events.len(), 1);
        assert_eq!(value["type"], "renderError");
        assert_eq!(value["source"], "server");
        assert_eq!(value["status"], "notFound");
        assert_eq!(value["artifactId"], "missing-artifact");
        assert_eq!(value["phase"], "renderArtifact");
        assert_eq!(value["message"], "artifact not found: missing-artifact");
    }

    #[tokio::test]
    async fn native_export_spreadsheet_uses_configured_adapter_and_records_telemetry() {
        let export_dir = tempfile::tempdir().expect("temp export dir");
        let script = export_dir.path().join("fake_export.py");
        fs::write(
            &script,
            r#"
import json
import pathlib
import sys
request = json.loads(pathlib.Path(sys.argv[1]).read_text())
output = pathlib.Path(request["outputDir"]) / f'{request["jobId"]}.xlsx'
output.write_bytes(b"fake-xlsx")
print(json.dumps({
    "ok": True,
    "artifactId": request["artifact"]["id"],
    "format": request["format"],
    "adapter": "test.export",
    "status": "complete",
    "filePath": str(output),
    "fileUrl": None,
    "mimeType": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "bytes": output.stat().st_size,
    "message": None,
}))
"#,
        )
        .expect("write fake adapter");
        let mut state = test_state();
        state.export_tool = ExportToolConfig {
            python: PathBuf::from("python3"),
            script,
            output_dir: export_dir.path().to_path_buf(),
        };
        let artifact = create_sheet(SheetRequest {
            id: "sheet-export-test".to_owned(),
            title: "Export Sheet".to_owned(),
            columns: vec!["house".to_owned(), "score".to_owned()],
            rows: vec![BTreeMap::from([
                ("house".to_owned(), json!("Lannister")),
                ("score".to_owned(), json!(10)),
            ])],
            source: None,
        })
        .expect("sheet artifact");
        state
            .native_workspace
            .write()
            .await
            .insert(artifact, "test")
            .expect("insert artifact");

        let Json(result) = native_export_spreadsheet(
            State(state.clone()),
            Json(ExportRequest {
                artifact_id: "sheet-export-test".to_owned(),
                format: Some("xlsx".to_owned()),
            }),
        )
        .await
        .expect("export succeeds");

        assert!(result.ok);
        assert_eq!(result.status, "complete");
        assert_eq!(result.adapter, "test.export");
        assert!(result
            .file_url
            .as_deref()
            .expect("file url")
            .starts_with("/native/export/files/"));
        assert_eq!(result.bytes, Some(9));

        let events = state.telemetry.read().await.clone();
        let value = serde_json::to_value(events.last().expect("export telemetry"))
            .expect("telemetry serializes");
        assert_eq!(value["type"], "export");
        assert_eq!(value["operation"], "export.spreadsheet");
        assert_eq!(value["artifactId"], "sheet-export-test");
        assert_eq!(value["format"], "xlsx");
        assert_eq!(value["status"], "complete");
        assert_eq!(value["bytes"], 9);
    }

    #[tokio::test]
    async fn native_export_slide_deck_reports_missing_office_toolchain() {
        let mut state = test_state();
        let export_dir = tempfile::tempdir().expect("temp export dir");
        state.export_tool.script = repo_root().join("scripts/export_artifact.py");
        state.export_tool.output_dir = export_dir.path().to_path_buf();
        let (_, artifact) = create_slide_deck(SlideDeckRequest {
            id: "deck-export-test".to_owned(),
            title: "Export Deck".to_owned(),
            slides: vec![SlideRef {
                artifact_id: "slide-1".to_owned(),
                title: "Slide 1".to_owned(),
            }],
            export: vec!["html".to_owned(), "pdf".to_owned()],
        })
        .expect("deck artifact");
        state
            .native_workspace
            .write()
            .await
            .insert(artifact, "test")
            .expect("insert artifact");

        let Json(result) = native_export_slide_deck(
            State(state),
            Json(ExportRequest {
                artifact_id: "deck-export-test".to_owned(),
                format: Some("pptx".to_owned()),
            }),
        )
        .await
        .expect("tool-unavailable is a typed result");

        assert!(!result.ok);
        assert_eq!(result.status, "toolUnavailable");
        assert_eq!(result.format, "pptx");
        assert!(result
            .message
            .as_deref()
            .expect("message")
            .contains("office toolchain"));
    }

    #[tokio::test]
    async fn native_mcp_tools_expose_canonical_workspace_contracts() {
        let Json(payload) = native_mcp_tools().await;
        let tools = payload["tools"].as_array().expect("tools array");
        let expected_names: BTreeSet<_> =
            LOCAL_TOOL_COVERAGE.iter().map(|entry| entry.tool).collect();
        let mut actual_names = BTreeSet::new();
        for tool in tools {
            let name = tool["name"].as_str().expect("tool name");
            assert!(
                actual_names.insert(name),
                "duplicate tool descriptor: {name}"
            );
        }
        for expected in expected_names {
            assert!(
                actual_names.contains(expected),
                "missing matrix tool descriptor: {expected}"
            );
        }

        let by_name = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
        };

        for entry in LOCAL_TOOL_COVERAGE {
            let tool = by_name(entry.tool);
            match entry.status {
                Coverage::Explicit => {
                    assert_eq!(
                        tool["status"], "implemented",
                        "tool {} status must match matrix",
                        entry.tool
                    );
                    assert!(
                        tool.get("adapter").is_some(),
                        "implemented tool {} must declare adapter",
                        entry.tool
                    );
                    assert!(
                        tool.get("method").is_some(),
                        "implemented tool {} must declare method",
                        entry.tool
                    );
                    assert!(
                        tool.get("route").is_some(),
                        "implemented tool {} must declare route",
                        entry.tool
                    );
                }
                Coverage::Deferred => {
                    assert_eq!(
                        tool["status"], "deferred",
                        "tool {} status must match matrix",
                        entry.tool
                    );
                    assert!(
                        tool.get("adapter").is_none(),
                        "deferred tool {} must not claim an adapter",
                        entry.tool
                    );
                    assert!(
                        tool.get("route").is_none(),
                        "deferred tool {} must not claim a route",
                        entry.tool
                    );
                }
                Coverage::Generic | Coverage::NotApplicable => {
                    panic!("local tool {} has unsupported coverage status", entry.tool)
                }
            }
        }

        let comment = by_name("local.ui.comment");
        assert_eq!(comment["adapter"], "local__ui_comment");
        assert_eq!(comment["method"], "POST");
        assert_eq!(comment["route"], "/native/workspace/change-request");
        assert_eq!(
            comment["inputSchema"]["required"],
            json!(["target", "instruction"])
        );

        let resolve = by_name("local.ui.resolve");
        assert_eq!(resolve["status"], "implemented");
        assert_eq!(resolve["route"], "/native/workspace/resolve");
        assert_eq!(resolve["inputSchema"]["required"], json!(["taskId"]));

        let mutate = by_name("local.ui.mutate");
        assert_eq!(mutate["route"], "/native/workspace/mutate");
        assert_eq!(
            mutate["inputSchema"]["oneOf"][0]["properties"]["type"]["const"],
            "title"
        );
        assert_eq!(
            mutate["inputSchema"]["oneOf"][1]["properties"]["type"]["const"],
            "text"
        );
        assert_eq!(
            mutate["inputSchema"]["oneOf"][1]["properties"]["shadowSelector"],
            json!({"type": "string", "pattern": "data-capsem-(node|topology-id)="})
        );
        assert_eq!(
            mutate["inputSchema"]["oneOf"][2]["properties"]["type"]["const"],
            "style"
        );
        assert_eq!(
            mutate["inputSchema"]["oneOf"][2]["properties"]["styles"]["propertyNames"]["enum"],
            json!([
                "backgroundColor",
                "color",
                "fontStyle",
                "fontWeight",
                "opacity",
                "textDecoration"
            ])
        );

        let snapshot = by_name("local.workspace.snapshot");
        assert_eq!(snapshot["status"], "implemented");
        assert_eq!(snapshot["method"], "GET");
        assert_eq!(snapshot["route"], "/native/workspace/snapshot");

        let render_error = by_name("local.ui.renderError");
        assert_eq!(render_error["route"], "/native/ui/render-error");
        assert_eq!(render_error["inputSchema"]["required"], json!(["message"]));

        let chart = by_name("local.ui.chart");
        assert_eq!(
            chart["inputSchema"]["required"],
            json!([
                "id",
                "title",
                "chart",
                "sourceArtifact",
                "data",
                "x",
                "series",
                "xLabel",
                "yLabel",
                "yUnit"
            ])
        );
        assert_eq!(
            chart["inputSchema"]["properties"]["chart"]["enum"],
            json!([
                "barChart",
                "lineChart",
                "heatmapChart",
                "boxPlot",
                "scatterPlot"
            ])
        );
        assert_eq!(
            chart["inputSchema"]["properties"]["stack"]["enum"],
            json!(["none", "stacked", "grouped"])
        );
        assert_eq!(
            chart["inputSchema"]["properties"]["series"]["items"]["required"],
            json!(["name", "field"])
        );
        assert_eq!(
            chart["inputSchema"]["properties"]["secondAxis"]["properties"]["label"],
            json!({"type": "string", "minLength": 1})
        );
        assert_eq!(
            chart["inputSchema"]["properties"]["fit"]["properties"]["method"]["enum"],
            json!(["linear", "logarithmic", "movingAverage"])
        );

        let timeline = by_name("local.ui.timeline");
        assert_eq!(
            timeline["inputSchema"]["required"],
            json!(["id", "title", "lanes", "events"])
        );
        assert_eq!(
            timeline["inputSchema"]["properties"]["lanes"]["items"]["required"],
            json!(["id", "title"])
        );
        assert_eq!(
            timeline["inputSchema"]["properties"]["events"]["items"]["required"],
            json!(["id", "title", "lane", "start"])
        );
        assert_eq!(
            timeline["inputSchema"]["properties"]["export"]["items"]["enum"],
            json!(["html", "png", "svg"])
        );

        let slide = by_name("local.ui.slide");
        assert_eq!(
            slide["inputSchema"]["required"],
            json!(["id", "title", "blocks"])
        );
        assert_eq!(
            slide["inputSchema"]["properties"]["blocks"]["items"]["oneOf"][0]["properties"]["kind"]
                ["const"],
            "text"
        );
        assert_eq!(
            slide["inputSchema"]["properties"]["blocks"]["items"]["oneOf"][1]["properties"]
                ["artifactId"],
            json!({"type": "string", "minLength": 1})
        );

        let slide_deck = by_name("local.ui.slideDeck");
        assert_eq!(
            slide_deck["inputSchema"]["required"],
            json!(["id", "title", "slides"])
        );
        assert_eq!(
            slide_deck["inputSchema"]["properties"]["slides"]["items"]["required"],
            json!(["artifactId", "title"])
        );
        assert_eq!(
            slide_deck["inputSchema"]["properties"]["export"]["items"]["enum"],
            json!(["html", "pdf"])
        );

        let export_spreadsheet = by_name("local.export.spreadsheet");
        assert_eq!(export_spreadsheet["status"], "implemented");
        assert_eq!(export_spreadsheet["route"], "/native/export/spreadsheet");
        assert_eq!(
            export_spreadsheet["inputSchema"]["properties"]["format"]["enum"],
            json!(["xlsx"])
        );

        let export_slide_deck = by_name("local.export.slideDeck");
        assert_eq!(export_slide_deck["status"], "implemented");
        assert_eq!(export_slide_deck["route"], "/native/export/slide-deck");
        assert_eq!(
            export_slide_deck["inputSchema"]["properties"]["format"]["enum"],
            json!(["pptx", "pdf", "html"])
        );

        let export_chart = by_name("local.export.chart");
        assert_eq!(export_chart["status"], "deferred");
        assert!(export_chart.get("route").is_none());

        let audio = by_name("local.generate.audio");
        assert_eq!(audio["status"], "deferred");
        assert!(audio.get("route").is_none());

        let web_preview = by_name("local.web.preview");
        assert_eq!(web_preview["status"], "deferred");
        assert!(web_preview.get("adapter").is_none());
    }

    #[tokio::test]
    async fn native_mcp_tool_schemas_accept_and_reject_representative_payloads() {
        let Json(payload) = native_mcp_tools().await;
        let tools = payload["tools"].as_array().expect("tools array");
        let by_name = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
        };

        let chart_schema = &by_name("local.ui.chart")["inputSchema"];
        assert_schema_accepts(
            chart_schema,
            &json!({
                "id": "chart-1",
                "title": "Chart",
                "chart": "scatterPlot",
                "sourceArtifact": "sheet-1",
                "data": [{"x": 1, "y": 2}],
                "x": "x",
                "series": [{"name": "y", "field": "y", "axis": "right"}],
                "xLabel": "X",
                "yLabel": "Y",
                "yUnit": "points",
                "fit": {"method": "linear", "display": true},
                "secondAxis": {"label": "Risk", "unit": "points"},
                "export": ["png", "svg"]
            }),
        );
        assert_schema_rejects(
            chart_schema,
            &json!({
                "id": "chart-1",
                "title": "Chart",
                "chart": "pieChart",
                "sourceArtifact": "sheet-1",
                "data": [],
                "x": "x",
                "series": [{"name": "y", "field": "y"}],
                "xLabel": "X",
                "yLabel": "Y",
                "yUnit": "points"
            }),
        );

        let timeline_schema = &by_name("local.ui.timeline")["inputSchema"];
        assert_schema_accepts(
            timeline_schema,
            &json!({
                "id": "timeline-1",
                "title": "Timeline",
                "lanes": [{"id": "build", "title": "Build"}],
                "events": [{
                    "id": "compile",
                    "title": "Compile",
                    "lane": "build",
                    "start": "2026-06-06",
                    "description": "Rust accepted the request."
                }],
                "export": ["html"]
            }),
        );
        assert_schema_rejects(
            timeline_schema,
            &json!({
                "id": "timeline-1",
                "title": "Timeline",
                "lanes": [{"id": "build", "title": "Build"}],
                "events": [{"id": "compile", "title": "Compile", "start": "2026-06-06"}]
            }),
        );

        let slide_schema = &by_name("local.ui.slide")["inputSchema"];
        assert_schema_accepts(
            slide_schema,
            &json!({
                "id": "slide-1",
                "title": "Slide",
                "blocks": [
                    {"kind": "text", "title": "Plan", "body": "Hold the line."},
                    {"kind": "chart", "artifactId": "chart-1"}
                ]
            }),
        );
        assert_schema_rejects(
            slide_schema,
            &json!({
                "id": "slide-1",
                "title": "Slide",
                "blocks": [{"kind": "chart"}]
            }),
        );

        let slide_deck_schema = &by_name("local.ui.slideDeck")["inputSchema"];
        assert_schema_accepts(
            slide_deck_schema,
            &json!({
                "id": "deck-1",
                "title": "Deck",
                "slides": [{"artifactId": "slide-1", "title": "Slide"}],
                "export": ["html", "pdf"]
            }),
        );
        assert_schema_rejects(
            slide_deck_schema,
            &json!({
                "id": "deck-1",
                "title": "Deck",
                "slides": [{"artifactId": "slide-1"}]
            }),
        );

        let export_spreadsheet_schema = &by_name("local.export.spreadsheet")["inputSchema"];
        assert_schema_accepts(
            export_spreadsheet_schema,
            &json!({"artifactId": "sheet-1", "format": "xlsx"}),
        );
        assert_schema_rejects(
            export_spreadsheet_schema,
            &json!({"artifactId": "sheet-1", "format": "csv"}),
        );

        let export_slide_deck_schema = &by_name("local.export.slideDeck")["inputSchema"];
        assert_schema_accepts(
            export_slide_deck_schema,
            &json!({"artifactId": "deck-1", "format": "pptx"}),
        );
        assert_schema_rejects(
            export_slide_deck_schema,
            &json!({"artifactId": "deck-1", "format": "xlsx"}),
        );

        let mutate_schema = &by_name("local.ui.mutate")["inputSchema"];
        assert_schema_accepts(
            mutate_schema,
            &json!({
                "type": "text",
                "target": "generated-image",
                "text": "Sharper caption",
                "selector": "capsem-media[data-capsem-artifact-id=\"generated-image\"] >>> [data-capsem-node=\"generated-image::caption\"]",
                "hostSelector": "capsem-media[data-capsem-artifact-id=\"generated-image\"]",
                "shadowSelector": "[data-capsem-node=\"generated-image::caption\"]",
                "sourceRequestSeq": 9
            }),
        );
        assert_schema_rejects(
            mutate_schema,
            &json!({
                "type": "text",
                "target": "generated-image",
                "text": "Sharper caption",
                "dangerouslySetInnerHTML": "<b>no</b>"
            }),
        );
        assert_schema_rejects(
            mutate_schema,
            &json!({
                "type": "text",
                "target": "generated-image",
                "text": "Sharper caption",
                "shadowSelector": "[part~=\"body\"]"
            }),
        );
    }

    fn assert_schema_accepts(schema: &serde_json::Value, value: &serde_json::Value) {
        if let Err(error) = validate_test_schema(schema, value) {
            panic!("schema should accept value: {error}; value={value}");
        }
    }

    fn assert_schema_rejects(schema: &serde_json::Value, value: &serde_json::Value) {
        assert!(
            validate_test_schema(schema, value).is_err(),
            "schema should reject value={value}"
        );
    }

    fn validate_test_schema(
        schema: &serde_json::Value,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        if let Some(one_of) = schema.get("oneOf").and_then(|value| value.as_array()) {
            let matches = one_of
                .iter()
                .filter(|candidate| validate_test_schema(candidate, value).is_ok())
                .count();
            return if matches == 1 {
                Ok(())
            } else {
                Err(format!("oneOf matched {matches} schemas"))
            };
        }

        if let Some(constant) = schema.get("const") {
            if value != constant {
                return Err(format!("expected const {constant}, got {value}"));
            }
        }
        if let Some(allowed) = schema.get("enum").and_then(|value| value.as_array()) {
            if !allowed.iter().any(|allowed| allowed == value) {
                return Err(format!("{value} not in enum {allowed:?}"));
            }
        }
        if let Some(types) = schema.get("type") {
            validate_test_type(types, value)?;
        }
        if let Some(min_length) = schema.get("minLength").and_then(|value| value.as_u64()) {
            let length = value
                .as_str()
                .ok_or_else(|| "minLength requires string value".to_owned())?
                .len() as u64;
            if length < min_length {
                return Err(format!(
                    "string length {length} below minLength {min_length}"
                ));
            }
        }
        if let Some(pattern) = schema.get("pattern").and_then(|value| value.as_str()) {
            validate_test_pattern(pattern, value)?;
        }
        if let Some(minimum) = schema.get("minimum").and_then(|value| value.as_f64()) {
            let number = value
                .as_f64()
                .ok_or_else(|| "minimum requires numeric value".to_owned())?;
            if number < minimum {
                return Err(format!("number {number} below minimum {minimum}"));
            }
        }
        if let Some(required) = schema.get("required").and_then(|value| value.as_array()) {
            let object = value
                .as_object()
                .ok_or_else(|| "required applies to non-object value".to_owned())?;
            for field in required {
                let field = field
                    .as_str()
                    .ok_or_else(|| "required field name must be string".to_owned())?;
                if !object.contains_key(field) {
                    return Err(format!("missing required field {field}"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&json!(false)) {
            if let Some(object) = value.as_object() {
                let properties = schema
                    .get("properties")
                    .and_then(|value| value.as_object())
                    .cloned()
                    .unwrap_or_default();
                for key in object.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!("unexpected property {key}"));
                    }
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) {
            if let Some(object) = value.as_object() {
                for (field, field_schema) in properties {
                    if let Some(field_value) = object.get(field) {
                        validate_test_schema(field_schema, field_value)
                            .map_err(|error| format!("{field}: {error}"))?;
                    }
                }
            }
        }
        if let Some(items_schema) = schema.get("items") {
            if let Some(items) = value.as_array() {
                for (index, item) in items.iter().enumerate() {
                    validate_test_schema(items_schema, item)
                        .map_err(|error| format!("item {index}: {error}"))?;
                }
            }
        }
        if let Some(min_items) = schema.get("minItems").and_then(|value| value.as_u64()) {
            let length = value
                .as_array()
                .ok_or_else(|| "minItems requires array value".to_owned())?
                .len() as u64;
            if length < min_items {
                return Err(format!("array length {length} below minItems {min_items}"));
            }
        }
        Ok(())
    }

    fn validate_test_pattern(pattern: &str, value: &serde_json::Value) -> Result<(), String> {
        let value = value
            .as_str()
            .ok_or_else(|| "pattern requires string value".to_owned())?;
        match pattern {
            "data-capsem-(node|topology-id)=" => {
                if value.contains("data-capsem-node=") || value.contains("data-capsem-topology-id=")
                {
                    Ok(())
                } else {
                    Err(format!(
                        "{value} does not target a stable Capsem topology node"
                    ))
                }
            }
            other => Err(format!("unsupported test pattern {other}")),
        }
    }

    fn validate_test_type(
        schema_type: &serde_json::Value,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let types: Vec<&str> = match schema_type {
            serde_json::Value::String(kind) => vec![kind.as_str()],
            serde_json::Value::Array(kinds) => kinds
                .iter()
                .map(|kind| {
                    kind.as_str()
                        .ok_or_else(|| "type array must contain strings".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
            other => return Err(format!("unsupported type declaration {other}")),
        };
        if types
            .iter()
            .any(|kind| value_matches_test_type(kind, value))
        {
            Ok(())
        } else {
            Err(format!("{value} does not match type {types:?}"))
        }
    }

    fn value_matches_test_type(kind: &str, value: &serde_json::Value) -> bool {
        match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        }
    }
}
