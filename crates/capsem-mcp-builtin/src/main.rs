//! Built-in MCP server for Capsem local tools.
//!
//! Runs as a stdio MCP server subprocess, managed by the aggregator.
//! Exposes HTTP tools (fetch_http, grep_http, http_headers) and
//! the echo transport probe.
//!
//! Config via environment variables:
//! - CAPSEM_ACTIVE_PROFILE: Session active profile whose security rules/plugins govern tools.
//! - CAPSEM_SESSION_DIR: Session directory; holds the per-peer singleton lock.
//!
//! It writes no ledger. capsem-process is the one writer of a session's
//! ledger, so what these tools do that only this process can see -- the HTTP
//! requests it makes -- goes back to it as records on each tool result (see `capsem_proto::mcp_contracts::builtin_ledger`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rmcp::handler::server::{router::Router, wrapper::Parameters, ServerHandler};
use rmcp::model::{CallToolResult, Content, Implementation, InitializeResult, Meta, ServerCapabilities};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_router, ServiceExt};
use serde::{Deserialize, Serialize};
use tracing::info;

use capsem_core::mcp::builtin_tools;
use capsem_core::mcp::builtin_tools::BuiltinHttpClient;
use capsem_core::net::policy_config::{ActiveProfileFile, SecurityPluginConfig, SecurityRuleSet};
use capsem_proto::mcp_contracts::builtin_ledger::{self, BuiltinLedgerRecord, BUILTIN_LEDGER_META_KEY};
use capsem_proto::mcp_contracts::JsonRpcResponse;

const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// -- Tool parameter types --

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct EchoParams {
    /// Text to echo back. Returned verbatim in the tool result. Use a
    /// short string to isolate MCP transport overhead; longer strings
    /// to characterize per-byte cost.
    text: String,
}

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

// -- Handler --

#[derive(Clone)]
struct BuiltinHandler {
    http_client: BuiltinHttpClient,
    security_rules: Arc<SecurityRuleSet>,
    plugin_policy: Arc<BTreeMap<String, SecurityPluginConfig>>,
}

impl ServerHandler for BuiltinHandler {
    fn get_info(&self) -> InitializeResult {
        let caps = ServerCapabilities::builder().enable_tools().build();
        let mut info = InitializeResult::new(caps);
        info.server_info = Implementation::new("capsem-local", env!("CARGO_PKG_VERSION"));
        info
    }
}

#[tool_router]
impl BuiltinHandler {
    // -- Diagnostic tool --

    #[tool(
        name = "echo",
        description = "Return the input text verbatim. Zero I/O, zero policy, no upstream -- exists to benchmark MCP transport overhead (gateway -> aggregator -> server -> response) without any other variable. Use the `text` parameter; the result is the same text wrapped in a tool-call response.",
        annotations(
            title = "Echo",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn echo(&self, Parameters(params): Parameters<EchoParams>) -> Result<String, String> {
        Ok(params.text)
    }

    // -- HTTP tools --

    #[tool(
        name = "fetch_http",
        description = "Fetch a URL and return its content. In 'markdown' mode (default), HTML is converted to markdown. In 'content' mode, plain text. In 'raw' mode, unchanged. Use start_index/max_length for pagination.",
        annotations(
            title = "Fetch URL",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn fetch_http(&self, Parameters(params): Parameters<FetchHttpParams>) -> CallToolResult {
        call_builtin(self, "fetch_http", to_args(&params)).await
    }

    #[tool(
        name = "grep_http",
        description = "Fetch a URL and search its content for a regex pattern. Returns matching lines with context. Use start_index/max_length for pagination.",
        annotations(
            title = "Grep URL",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn grep_http(&self, Parameters(params): Parameters<GrepHttpParams>) -> CallToolResult {
        call_builtin(self, "grep_http", to_args(&params)).await
    }

    #[tool(
        name = "http_headers",
        description = "Return HTTP status code and response headers for a URL. Optionally specify the HTTP method (default: GET).",
        annotations(
            title = "HTTP headers",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn http_headers(&self, Parameters(params): Parameters<HttpHeadersParams>) -> CallToolResult {
        call_builtin(self, "http_headers", to_args(&params)).await
    }
}

// -- Helpers --

fn to_args<T: serde::Serialize>(params: &T) -> serde_json::Value {
    serde_json::to_value(params).unwrap_or(serde_json::Value::Object(Default::default()))
}

async fn call_builtin(handler: &BuiltinHandler, name: &str, args: serde_json::Value) -> CallToolResult {
    let mut ledger = Vec::new();
    let resp = builtin_tools::call_builtin_tool(
        name,
        &args,
        &handler.http_client,
        &handler.security_rules,
        &handler.plugin_policy,
        None,
        &mut ledger,
    )
    .await;
    tool_result(extract_text(resp), &ledger)
}

/// The tool result, carrying the ledger records capsem-process writes.
///
/// A refusal is recorded as surely as a success -- a denied request is the
/// row an investigator most wants -- so records ride on error results too.
fn tool_result(outcome: Result<String, String>, records: &[BuiltinLedgerRecord]) -> CallToolResult {
    let mut result = match outcome {
        Ok(text) => CallToolResult::success(vec![Content::text(text)]),
        Err(text) => CallToolResult::error(vec![Content::text(text)]),
    };
    if !records.is_empty() {
        let mut meta = Meta::new();
        meta.insert(BUILTIN_LEDGER_META_KEY.to_string(), builtin_ledger::encode(records));
        result.meta = Some(meta);
    }
    result
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
    let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    if is_error {
        Err(text)
    } else {
        Ok(text)
    }
}

// -- Main --

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().skip(1).any(|arg| arg == "--version" || arg == "-V") {
        println!("capsem-mcp-builtin {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let _telemetry_guard = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-mcp-builtin",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "capsem_mcp_builtin=info",
    })?;

    info!("capsem-mcp-builtin starting");

    let parent_pid = std::env::var("CAPSEM_PARENT_PID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let session_dir = std::env::var("CAPSEM_SESSION_DIR").ok();

    // Per-peer index for pool members (set by the aggregator's
    // connect_stdio when spawning peer 1..N of a pooled server). Each
    // peer gets its own lockfile so they don't fight over the singleton.
    let peer_index: u32 = std::env::var("CAPSEM_BUILTIN_PEER_INDEX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if let (Some(pid), Some(dir)) = (parent_pid, session_dir) {
        let lock_name = if peer_index == 0 {
            "mcp-builtin.lock".to_string()
        } else {
            format!("mcp-builtin-{peer_index}.lock")
        };
        let lock_path = std::path::PathBuf::from(dir).join(&lock_name);
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

    let active_profile_path =
        std::env::var("CAPSEM_ACTIVE_PROFILE").map_err(|_| anyhow::anyhow!("CAPSEM_ACTIVE_PROFILE is required"))?;
    let active_profile_text = std::fs::read_to_string(&active_profile_path)
        .map_err(anyhow::Error::new)
        .with_context(|| format!("read active profile {active_profile_path}"))?;
    let active_profile: ActiveProfileFile = toml::from_str(&active_profile_text)
        .map_err(anyhow::Error::new)
        .with_context(|| format!("parse active profile {active_profile_path}"))?;
    active_profile
        .validate()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("validate active profile {active_profile_path}"))?;
    let security_rules = Arc::new(active_profile.compile_security_rule_set().map_err(anyhow::Error::msg)?);
    let plugin_policy = Arc::new(active_profile.plugins.clone());

    let handler = BuiltinHandler {
        http_client: BuiltinHttpClient::new(HTTP_REQUEST_TIMEOUT, HTTP_CONNECT_TIMEOUT),
        security_rules,
        plugin_policy,
    };

    let tools = BuiltinHandler::tool_router();
    info!(tool_count = tools.list_all().len(), "registered tools");

    let router = Router::new(handler).with_tools(tools);
    let transport = rmcp::transport::stdio();

    router.serve(transport).await?.waiting().await?;

    info!("capsem-mcp-builtin shutting down");
    Ok(())
}

#[cfg(test)]
mod tests;
