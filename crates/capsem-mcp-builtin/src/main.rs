//! Built-in MCP server for Capsem local tools.
//!
//! Runs as a stdio MCP server subprocess, managed by the aggregator.
//! Exposes HTTP tools (fetch_http, grep_http, http_headers) and
//! file/snapshot tools (when CAPSEM_SESSION_DIR is set).
//!
//! Config via environment variables:
//! - CAPSEM_SESSION_DIR: Session directory (parent of workspace). Enables snapshot tools.
//! - CAPSEM_DOMAIN_ALLOW: Comma-separated allowed domain patterns
//! - CAPSEM_DOMAIN_BLOCK: Comma-separated blocked domain patterns
//! - CAPSEM_SESSION_DB: Path to session DB for telemetry (optional)

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::{
    router::Router,
    wrapper::Parameters,
    ServerHandler,
};
use rmcp::model::{
    Implementation, InitializeResult, ServerCapabilities,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_router, ServiceExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::info;

use capsem_core::auto_snapshot::AutoSnapshotScheduler;
use capsem_core::mcp::{builtin_tools, file_tools};
use capsem_core::mcp::types::JsonRpcResponse;
use capsem_core::net::domain_policy::{Action, DomainPolicy};
use capsem_logger::DbWriter;

// -- Tool parameter types --

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct FetchHttpParams {
    /// The URL to fetch. The domain must be allowed by network policy.
    url: String,
    /// Output format: 'markdown' (default), 'content' (plain text), or 'raw'.
    #[serde(default)]
    format: Option<String>,
    /// Character offset to start reading from (default: 0). For pagination.
    #[serde(default)]
    start_index: Option<u64>,
    /// Maximum characters to return (default: 5000).
    #[serde(default)]
    max_length: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GrepHttpParams {
    /// The URL to fetch and search.
    url: String,
    /// Regex pattern to search for.
    pattern: String,
    /// Number of context lines around each match (default: 3).
    #[serde(default)]
    context_lines: Option<u64>,
    /// Maximum number of matches to return (default: 50).
    #[serde(default)]
    max_matches: Option<u64>,
    /// Character offset to start reading from (default: 0).
    #[serde(default)]
    start_index: Option<u64>,
    /// Maximum characters to return (default: 5000).
    #[serde(default)]
    max_length: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct HttpHeadersParams {
    /// The URL to check headers for.
    url: String,
    /// HTTP method to use (default: GET).
    #[serde(default)]
    method: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotPaginationParams {
    /// Character offset to start from (default: 0).
    #[serde(default)]
    start_index: Option<u64>,
    /// Maximum characters to return (default: 5000).
    #[serde(default)]
    max_length: Option<u64>,
    /// Output format: 'text' (default) or 'json'.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotHistoryParams {
    /// File path to show history for (relative to workspace root or absolute).
    path: String,
    /// Character offset to start from (default: 0).
    #[serde(default)]
    start_index: Option<u64>,
    /// Maximum characters to return (default: 5000).
    #[serde(default)]
    max_length: Option<u64>,
    /// Output format: 'text' (default) or 'json'.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotNameParams {
    /// Name for the snapshot.
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotRevertParams {
    /// File path to revert (relative to workspace root or absolute).
    path: String,
    /// Checkpoint ID (e.g. "cp-3"). Optional -- auto-picks the newest
    /// snapshot that contains the file when absent.
    #[serde(default)]
    checkpoint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotDeleteParams {
    /// Checkpoint ID to delete (e.g. "cp-3").
    checkpoint: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SnapshotCompactParams {
    /// List of checkpoint IDs to compact.
    checkpoints: Vec<String>,
    /// Name for the resulting merged snapshot (optional; auto-generated if missing).
    #[serde(default)]
    name: Option<String>,
}

// -- Handler --

#[derive(Clone)]
struct BuiltinHandler {
    http_client: reqwest::Client,
    domain_policy: Arc<DomainPolicy>,
    db: Arc<DbWriter>,
    scheduler: Option<Arc<Mutex<AutoSnapshotScheduler>>>,
    workspace_dir: Option<PathBuf>,
}

impl ServerHandler for BuiltinHandler {
    fn get_info(&self) -> InitializeResult {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .build();
        let mut info = InitializeResult::new(caps);
        info.server_info = Implementation::new("capsem-local", env!("CARGO_PKG_VERSION"));
        info
    }
}

#[tool_router]
impl BuiltinHandler {
    // -- HTTP tools --

    #[tool(
        name = "fetch_http",
        description = "Fetch a URL and return its content. In 'markdown' mode (default), HTML is converted to markdown. In 'content' mode, plain text. In 'raw' mode, unchanged. Use start_index/max_length for pagination.",
        annotations(title = "Fetch URL", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn fetch_http(
        &self,
        Parameters(params): Parameters<FetchHttpParams>,
    ) -> Result<String, String> {
        call_builtin(self, "fetch_http", to_args(&params)).await
    }

    #[tool(
        name = "grep_http",
        description = "Fetch a URL and search its content for a regex pattern. Returns matching lines with context. Use start_index/max_length for pagination.",
        annotations(title = "Grep URL", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn grep_http(
        &self,
        Parameters(params): Parameters<GrepHttpParams>,
    ) -> Result<String, String> {
        call_builtin(self, "grep_http", to_args(&params)).await
    }

    #[tool(
        name = "http_headers",
        description = "Return HTTP status code and response headers for a URL. Optionally specify the HTTP method (default: GET).",
        annotations(title = "HTTP headers", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn http_headers(
        &self,
        Parameters(params): Parameters<HttpHeadersParams>,
    ) -> Result<String, String> {
        call_builtin(self, "http_headers", to_args(&params)).await
    }

    // -- Snapshot tools --

    #[tool(
        name = "snapshots_changes",
        description = "List files changed in the workspace compared to automatic checkpoints. Shows newest changes first, paginated."
    )]
    async fn snapshots_changes(
        &self,
        Parameters(params): Parameters<SnapshotPaginationParams>,
    ) -> Result<String, String> {
        let (sched, ws) = self.snapshot_state()?;
        let sched = sched.lock().await;
        let resp = file_tools::handle_list_changed_files(&to_args(&params), &sched, &ws, None);
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_list",
        description = "List all snapshots (automatic and manual) with metadata and per-snapshot diffs."
    )]
    async fn snapshots_list(
        &self,
        Parameters(params): Parameters<SnapshotPaginationParams>,
    ) -> Result<String, String> {
        let (sched, ws) = self.snapshot_state()?;
        let sched = sched.lock().await;
        let resp = file_tools::handle_list_snapshots(&to_args(&params), &sched, &ws, None);
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_revert",
        description = "Restore a file from a checkpoint to the current workspace."
    )]
    async fn snapshots_revert(
        &self,
        Parameters(params): Parameters<SnapshotRevertParams>,
    ) -> Result<String, String> {
        let (sched, ws) = self.snapshot_state()?;
        let sched = sched.lock().await;
        let resp = file_tools::handle_revert_file(&to_args(&params), &sched, &ws, None, Some(&self.db));
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_create",
        description = "Create a named manual snapshot (checkpoint)."
    )]
    async fn snapshots_create(
        &self,
        Parameters(params): Parameters<SnapshotNameParams>,
    ) -> Result<String, String> {
        let (sched, _ws) = self.snapshot_state()?;
        let mut sched = sched.lock().await;
        let resp = file_tools::handle_snapshot(&to_args(&params), &mut sched, None);
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_delete",
        description = "Delete a manual snapshot by checkpoint ID."
    )]
    async fn snapshots_delete(
        &self,
        Parameters(params): Parameters<SnapshotDeleteParams>,
    ) -> Result<String, String> {
        let (sched, _ws) = self.snapshot_state()?;
        let sched = sched.lock().await;
        let resp = file_tools::handle_delete_snapshot(&to_args(&params), &sched, None);
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_history",
        description = "Show revert history for the session."
    )]
    async fn snapshots_history(
        &self,
        Parameters(params): Parameters<SnapshotHistoryParams>,
    ) -> Result<String, String> {
        let (sched, ws) = self.snapshot_state()?;
        let sched = sched.lock().await;
        let resp = file_tools::handle_snapshots_history(&to_args(&params), &sched, &ws, None);
        extract_text(resp)
    }

    #[tool(
        name = "snapshots_compact",
        description = "Compact snapshot storage by merging specified checkpoints."
    )]
    async fn snapshots_compact(
        &self,
        Parameters(params): Parameters<SnapshotCompactParams>,
    ) -> Result<String, String> {
        let (sched, _ws) = self.snapshot_state()?;
        let mut sched = sched.lock().await;
        let resp = file_tools::handle_snapshots_compact(&to_args(&params), &mut sched, None);
        extract_text(resp)
    }
}

impl BuiltinHandler {
    fn snapshot_state(&self) -> Result<(Arc<Mutex<AutoSnapshotScheduler>>, PathBuf), String> {
        let sched = self.scheduler.as_ref()
            .ok_or("snapshot tools unavailable (no session directory)")?;
        let ws = self.workspace_dir.as_ref()
            .ok_or("snapshot tools unavailable (no workspace directory)")?;
        Ok((Arc::clone(sched), ws.clone()))
    }
}

// -- Helpers --

fn to_args<T: serde::Serialize>(params: &T) -> serde_json::Value {
    serde_json::to_value(params).unwrap_or(serde_json::Value::Object(Default::default()))
}

async fn call_builtin(
    handler: &BuiltinHandler,
    name: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    let resp = builtin_tools::call_builtin_tool(
        name,
        &args,
        &handler.http_client,
        &handler.domain_policy,
        None,
        &handler.db,
    )
    .await;
    extract_text(resp)
}

fn extract_text(resp: JsonRpcResponse) -> Result<String, String> {
    if let Some(err) = resp.error {
        return Err(err.message);
    }
    let result = resp.result.unwrap_or(serde_json::Value::Null);
    let text = if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        content
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        serde_json::to_string_pretty(&result).unwrap_or_default()
    };
    // The underlying ``call_builtin_tool`` signals a logical tool failure via
    // ``isError: true`` on the result (blocked domain, invalid URL, policy
    // refusal). rmcp's ``Result<String, String>`` maps ``Err`` to the wire-
    // level ``isError: true`` response, so we propagate that here -- without
    // this, the client saw a successful result containing error text.
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_error { Err(text) } else { Ok(text) }
}

// -- Main --

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "capsem_mcp_builtin=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    info!("capsem-mcp-builtin starting");

    let parent_pid = std::env::var("CAPSEM_PARENT_PID").ok()
        .and_then(|s| s.parse::<u32>().ok());
    let session_dir = std::env::var("CAPSEM_SESSION_DIR").ok();

    if let (Some(pid), Some(dir)) = (parent_pid, session_dir) {
        let lock_path = std::path::PathBuf::from(dir).join("mcp-builtin.lock");
        match capsem_guard::install(Some(pid), &lock_path) {
            Ok(Some(guards)) => {
                // Keep the guards alive for the process's lifetime.
                Box::leak(Box::new(guards));
            }
            Ok(None) => {
                info!(lock = %lock_path.display(), "another instance holds the lock; exiting 0");
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(error = %e, "refusing to run without live parent; exiting 0");
                return Ok(());
            }
        }
    }

    // Domain policy from env vars.
    let allow: Vec<String> = std::env::var("CAPSEM_DOMAIN_ALLOW")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let block: Vec<String> = std::env::var("CAPSEM_DOMAIN_BLOCK")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let default_action = if allow.is_empty() && block.is_empty() {
        Action::Allow
    } else {
        Action::Deny
    };
    let domain_policy = Arc::new(DomainPolicy::new(&allow, &block, default_action));

    // Session DB writer (optional).
    let db = match std::env::var("CAPSEM_SESSION_DB") {
        Ok(path) => {
            match DbWriter::open(std::path::Path::new(&path), 256) {
                Ok(writer) => Arc::new(writer),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to open session DB, telemetry disabled");
                    Arc::new(DbWriter::open_in_memory(1).expect("in-memory DB"))
                }
            }
        }
        Err(_) => Arc::new(DbWriter::open_in_memory(1).expect("in-memory DB")),
    };

    // Snapshot scheduler (optional, requires CAPSEM_SESSION_DIR).
    let (scheduler, workspace_dir) = match std::env::var("CAPSEM_SESSION_DIR") {
        Ok(session_dir) => {
            let session_path = PathBuf::from(&session_dir);
            let ws = session_path.join("workspace");
            if ws.exists() {
                let sched = AutoSnapshotScheduler::new(
                    session_path,
                    10,  // max auto snapshots
                    12,  // max manual snapshots
                    std::time::Duration::from_secs(300),
                );
                info!(workspace = %ws.display(), "snapshot tools enabled");
                (Some(Arc::new(Mutex::new(sched))), Some(ws))
            } else {
                tracing::warn!(path = %ws.display(), "workspace directory not found, snapshot tools disabled");
                (None, None)
            }
        }
        Err(_) => {
            info!("CAPSEM_SESSION_DIR not set, snapshot tools disabled");
            (None, None)
        }
    };

    let handler = BuiltinHandler {
        http_client: reqwest::Client::new(),
        domain_policy,
        db,
        scheduler,
        workspace_dir,
    };

    let tools = BuiltinHandler::tool_router();
    info!(tool_count = tools.list_all().len(), "registered tools");

    let router = Router::new(handler).with_tools(tools);
    let transport = rmcp::transport::stdio();

    router.serve(transport).await?.waiting().await?;

    info!("capsem-mcp-builtin shutting down");
    Ok(())
}
