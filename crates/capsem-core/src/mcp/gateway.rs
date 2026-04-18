//! MCP gateway: handles vsock:5003 connections from the guest.
//!
//! Each connection is served by a `tokio::spawn()` task (same pattern as
//! the MITM proxy on vsock:5002). The guest sends raw NDJSON (one JSON-RPC
//! message per line) over the vsock, with a `\0CAPSEM_META:process_name\n`
//! metadata line first (same convention as capsem-net-proxy). No msgpack
//! framing -- bytes pass through with minimal overhead.

use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use capsem_logger::{DbWriter, McpCall, WriteOp};

use crate::net::domain_policy::DomainPolicy;

use super::aggregator::AggregatorClient;
use super::policy::{McpPolicy, ToolDecision};
use super::types::*;

/// Maximum NDJSON line length (1MB). Reject lines larger than this.
const MAX_LINE_LEN: usize = 1_048_576;

/// Shared configuration for the MCP gateway.
///
/// Two construction patterns:
/// - **capsem-process** (service mode): uses `aggregator` to route tool calls
///   through a subprocess. `server_manager` / `http_client` are `None`.
/// - **capsem-app** (desktop mode): manages MCP servers directly via
///   `server_manager` + `http_client`. `aggregator` is a no-op stub.
pub struct McpGatewayConfig {
    /// Client handle for the MCP aggregator subprocess.
    /// Routes all tool calls (local + external) through the aggregator.
    pub aggregator: AggregatorClient,
    pub db: Arc<DbWriter>,
    /// Double-Arc for atomic policy swap: outer RwLock protects inner Arc.
    /// New sessions clone the inner Arc for a consistent snapshot.
    pub policy: RwLock<Arc<McpPolicy>>,
    /// Domain policy (retained for hot-reload, passed to aggregator on refresh).
    pub domain_policy: std::sync::RwLock<Arc<DomainPolicy>>,
    /// Direct MCP server manager (desktop app mode).
    pub server_manager: tokio::sync::Mutex<super::server_manager::McpServerManager>,
    /// HTTP client for MCP server communication (desktop app mode).
    pub http_client: reqwest::Client,
    /// Snapshot scheduler (set after boot in VirtioFS mode).
    pub auto_snapshots: Option<Arc<tokio::sync::Mutex<crate::auto_snapshot::AutoSnapshotScheduler>>>,
    /// Workspace directory for the current session.
    pub workspace_dir: Option<std::path::PathBuf>,
}

/// Serve a single MCP session over a vsock connection.
///
/// Called inside `tokio::spawn()` for each vsock:5003 connection.
/// The guest sends NDJSON lines (one JSON-RPC message per line), prefixed
/// with a `\0CAPSEM_META:process_name\n` metadata line.
pub async fn serve_mcp_session(fd: RawFd, config: Arc<McpGatewayConfig>) {
    if let Err(e) = serve_mcp_session_inner(fd, &config).await {
        debug!(error = %e, "MCP session ended");
    }
}

async fn serve_mcp_session_inner(fd: RawFd, config: &McpGatewayConfig) -> Result<()> {
    use std::os::unix::io::FromRawFd;

    // Wrap the vsock fd for async I/O via tokio.
    // Safety: fd is a valid vsock socket from the accept loop.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    // Safely duplicate the file descriptor so we don't take ownership of the original,
    // preventing a double-close IO safety violation when the framework drops the connection.
    let vsock_stream = std::mem::ManuallyDrop::new(unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) });
    let std_stream = vsock_stream.try_clone()
        .context("failed to dup vsock fd")?;

    let tokio_stream = tokio::net::UnixStream::from_std(std_stream)
        .context("failed to create async vsock stream")?;
    let (read_half, mut write_half) = tokio_stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Read the metadata line: \0CAPSEM_META:process_name\n
    let mut meta_line = String::new();
    reader
        .read_line(&mut meta_line)
        .await
        .context("failed to read MCP metadata line")?;

    let process_name = meta_line
        .strip_prefix('\0')
        .and_then(|s| s.strip_prefix("CAPSEM_META:"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    info!(process = %process_name, "MCP session started");

    // Snapshot policy for this session
    let policy: Arc<McpPolicy> = config.policy.read().await.clone();

    // Main request loop: read NDJSON lines
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .context("failed to read NDJSON line")?;

        if n == 0 {
            debug!(process = %process_name, "MCP session closed (EOF)");
            return Ok(());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.len() > MAX_LINE_LEN {
            let err_resp = JsonRpcResponse::err(None, -32600, "request too large");
            let mut resp_line = serde_json::to_vec(&err_resp)?;
            resp_line.push(b'\n');
            write_half.write_all(&resp_line).await?;
            continue;
        }

        // Parse the JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "invalid JSON-RPC in MCP request");
                let err_resp = JsonRpcResponse::err(None, -32700, "parse error");
                let mut resp_line = serde_json::to_vec(&err_resp)?;
                resp_line.push(b'\n');
                write_half.write_all(&resp_line).await?;
                continue;
            }
        };

        let start = Instant::now();

        // Handle the request
        let response = handle_json_rpc(&request, config, &policy, &process_name).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Notifications (no id) get no response per JSON-RPC spec.
        let Some(response) = response else {
            continue;
        };

        // Log the call
        log_mcp_call(config, &request, &response, &process_name, duration_ms).await;

        // Send NDJSON response line
        let mut resp_line = serde_json::to_vec(&response)
            .context("failed to serialize JSON-RPC response")?;
        resp_line.push(b'\n');
        write_half.write_all(&resp_line).await?;
    }
}

/// Handle a single JSON-RPC request.
/// Returns `None` for notifications (no response expected).
async fn handle_json_rpc(
    req: &JsonRpcRequest,
    config: &McpGatewayConfig,
    policy: &McpPolicy,
    _process_name: &str,
) -> Option<JsonRpcResponse> {
    match req.method.as_str() {
        "initialize" => {
            Some(JsonRpcResponse::ok(
                req.id.clone(),
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {},
                        "resources": {},
                        "prompts": {}
                    },
                    "serverInfo": {
                        "name": "capsem-mcp-gateway",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ))
        }

        // Notifications have no id and expect no response.
        "notifications/initialized" => None,

        "tools/list" => {
            // All tools (local + external) come from the aggregator.
            let all_tools = config.aggregator.list_tools().await.unwrap_or_default();
            let tools: Vec<serde_json::Value> = all_tools
                .iter()
                .map(|t| {
                    let mut tool = serde_json::json!({
                        "name": t.namespaced_name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    });
                    if let Some(ref ann) = t.annotations {
                        tool["annotations"] = ann.to_mcp_json();
                    }
                    tool
                })
                .collect();
            Some(JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"tools": tools})))
        }

        "tools/call" => {
            let params = req.params.as_ref();
            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");

            if tool_name.is_empty() {
                return Some(JsonRpcResponse::err(req.id.clone(), -32602, "missing tool name"));
            }

            let arguments = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::json!({}));

            // Policy check: parse namespace to get server name.
            let (server_name, _local_name) = parse_namespaced(tool_name)
                .unwrap_or(("", tool_name));

            let decision = policy.evaluate(server_name, Some(tool_name));
            match decision {
                ToolDecision::Block => {
                    return Some(JsonRpcResponse::err(
                        req.id.clone(),
                        -32600,
                        format!("tool blocked by policy: {tool_name}"),
                    ));
                }
                ToolDecision::Warn => {
                    debug!(tool = tool_name, "MCP tool call warned by policy");
                }
                ToolDecision::Allow => {}
            }

            // All tools route through the aggregator (local + external).
            match config.aggregator.call_tool(tool_name, arguments).await {
                Ok(result) => Some(JsonRpcResponse::ok(req.id.clone(), result)),
                Err(e) => Some(JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("tool call failed: {e}"),
                )),
            }
        }

        "resources/list" => {
            let resources: Vec<serde_json::Value> = config
                .aggregator
                .list_resources()
                .await
                .unwrap_or_default()
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "uri": r.namespaced_uri,
                        "name": r.name,
                        "description": r.description,
                        "mimeType": r.mime_type,
                    })
                })
                .collect();
            Some(JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"resources": resources})))
        }

        "resources/read" => {
            let uri = req
                .params
                .as_ref()
                .and_then(|p| p.get("uri"))
                .and_then(|u| u.as_str())
                .unwrap_or("");

            if uri.is_empty() {
                return Some(JsonRpcResponse::err(req.id.clone(), -32602, "missing resource URI"));
            }

            match config.aggregator.read_resource(uri).await {
                Ok(result) => Some(JsonRpcResponse::ok(req.id.clone(), result)),
                Err(e) => Some(JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("resource read failed: {e}"),
                )),
            }
        }

        "prompts/list" => {
            let prompts: Vec<serde_json::Value> = config
                .aggregator
                .list_prompts()
                .await
                .unwrap_or_default()
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "name": p.namespaced_name,
                        "description": p.description,
                        "arguments": p.arguments,
                    })
                })
                .collect();
            Some(JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"prompts": prompts})))
        }

        "prompts/get" => {
            let params = req.params.as_ref();
            let prompt_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");

            if prompt_name.is_empty() {
                return Some(JsonRpcResponse::err(req.id.clone(), -32602, "missing prompt name"));
            }

            let arguments = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::json!({}));

            match config.aggregator.get_prompt(prompt_name, arguments).await {
                Ok(result) => Some(JsonRpcResponse::ok(req.id.clone(), result)),
                Err(e) => Some(JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("prompt get failed: {e}"),
                )),
            }
        }

        _ => Some(JsonRpcResponse::err(req.id.clone(), -32601, format!("method not found: {}", req.method))),
    }
}

/// Log an MCP call to the session database.
async fn log_mcp_call(
    config: &McpGatewayConfig,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
) {
    let tool_name = req
        .params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str());

    let server_name = match tool_name {
        Some(t) => parse_namespaced(t).map(|(s, _)| s).unwrap_or("gateway"),
        None => "gateway",
    };

    let decision = if resp.error.is_some() {
        if resp
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("blocked by policy"))
        {
            "denied"
        } else {
            "error"
        }
    } else {
        "allowed"
    };

    let error_message = resp.error.as_ref().map(|e| e.message.clone());

    // Full preview -- the writer's cap_field() at 256KB is the safety net.
    let req_preview = req
        .params
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok());

    let resp_preview = resp
        .result
        .as_ref()
        .and_then(|r| serde_json::to_string(r).ok());

    let bytes_sent = req
        .params
        .as_ref()
        .and_then(|p| serde_json::to_vec(p).ok())
        .map(|v| v.len() as u64)
        .unwrap_or(0);

    let bytes_received = resp
        .result
        .as_ref()
        .and_then(|r| serde_json::to_vec(r).ok())
        .map(|v| v.len() as u64)
        .unwrap_or(0);

    let call = McpCall {
        timestamp: SystemTime::now(),
        server_name: server_name.to_string(),
        method: req.method.clone(),
        tool_name: tool_name.map(String::from),
        request_id: req.id.as_ref().and_then(|v| v.as_u64()).map(|n| n.to_string()),
        request_preview: req_preview,
        response_preview: resp_preview,
        decision: decision.to_string(),
        duration_ms,
        error_message,
        process_name: Some(process_name.to_string()),
        bytes_sent,
        bytes_received,
    };

    config.db.write(WriteOp::McpCall(call)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::aggregator::*;
    use crate::net::domain_policy::DomainPolicy;

    /// Create a test AggregatorClient with a background driver that returns
    /// empty results (no external servers connected).
    fn test_aggregator_client(rt: &tokio::runtime::Runtime) -> AggregatorClient {
        let (client, mut rx) = AggregatorClient::channel(16);
        rt.spawn(async move {
            while let Some((req, resp_tx)) = rx.recv().await {
                let body = match req.method {
                    AggregatorMethod::ListServers => {
                        AggregatorResult::Servers { servers: vec![] }
                    }
                    AggregatorMethod::ListTools => {
                        AggregatorResult::Tools { tools: vec![] }
                    }
                    AggregatorMethod::ListResources => {
                        AggregatorResult::Resources { resources: vec![] }
                    }
                    AggregatorMethod::ListPrompts => {
                        AggregatorResult::Prompts { prompts: vec![] }
                    }
                    AggregatorMethod::CallTool { name, .. } => {
                        AggregatorResult::Error {
                            error: format!("no server for tool: {name}"),
                        }
                    }
                    AggregatorMethod::ReadResource { uri, .. } => {
                        AggregatorResult::Error {
                            error: format!("no server for resource: {uri}"),
                        }
                    }
                    AggregatorMethod::GetPrompt { name, .. } => {
                        AggregatorResult::Error {
                            error: format!("no server for prompt: {name}"),
                        }
                    }
                    AggregatorMethod::Refresh { .. } => {
                        AggregatorResult::Ok { ok: true }
                    }
                    AggregatorMethod::Shutdown => {
                        AggregatorResult::Ok { ok: true }
                    }
                };
                let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
            }
        });
        client
    }

    fn test_config(rt: &tokio::runtime::Runtime) -> McpGatewayConfig {
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        McpGatewayConfig {
            aggregator: test_aggregator_client(rt),
            db,
            policy: RwLock::new(Arc::new(McpPolicy::new())),
            domain_policy: std::sync::RwLock::new(Arc::new(DomainPolicy::default_dev())),
            server_manager: tokio::sync::Mutex::new(
                crate::mcp::server_manager::McpServerManager::new(vec![], reqwest::Client::new()),
            ),
            http_client: reqwest::Client::new(),
            auto_snapshots: None,
            workspace_dir: None,
        }
    }

    #[test]
    fn handle_unknown_method() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "unknown/method".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    }

    #[test]
    fn handle_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "initialize".into(),
            params: Some(serde_json::json!({})),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "capsem-mcp-gateway");
    }

    #[test]
    fn handle_notifications_initialized_returns_none() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        assert!(resp.is_none(), "notifications should not produce a response");
    }

    // Tests for direct builtin/file tool dispatch removed -- those tools
    // now route through the aggregator subprocess (capsem-mcp-builtin).
    // See crates/capsem-mcp-builtin/ for tool-level tests.

    #[test]
    fn handle_tools_list_returns_empty_from_mock_aggregator() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/list".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_none());
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        // Mock aggregator returns empty tool list
        assert!(tools.is_empty());
    }

    #[test]
    fn handle_tools_call_blocked_by_policy() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "evil__delete_all",
                "arguments": {}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy {
            blocked_servers: vec!["evil".to_string()],
            ..McpPolicy::new()
        };
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("blocked by policy"));
    }

    #[test]
    fn max_line_len_is_1mb() {
        assert_eq!(MAX_LINE_LEN, 1_048_576);
    }

    #[test]
    fn parse_meta_line() {
        let meta = "\0CAPSEM_META:claude\n";
        let name = meta
            .strip_prefix('\0')
            .and_then(|s| s.strip_prefix("CAPSEM_META:"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        assert_eq!(name, "claude");
    }

    #[test]
    fn parse_meta_line_missing_prefix() {
        let meta = "not a meta line\n";
        let name = meta
            .strip_prefix('\0')
            .and_then(|s| s.strip_prefix("CAPSEM_META:"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        assert_eq!(name, "unknown");
    }

    #[test]
    fn handle_tools_call_empty_tool_name() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({"name": "", "arguments": {}})),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("missing tool name"));
    }

    #[test]
    fn handle_tools_call_missing_name_field() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({"arguments": {}})),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("missing tool name"));
    }

    #[test]
    fn handle_tools_call_no_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("missing tool name"));
    }

    #[test]
    fn handle_tools_call_nonexistent_external_server() {
        // Tool name with namespace prefix for a server that doesn't exist.
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "nonexistent__some_tool",
                "arguments": {}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        // Should fail because no server named "nonexistent" is registered.
        assert!(resp.error.is_some());
    }

    #[test]
    fn handle_tools_call_namespaced_fetch_http_not_builtin() {
        // "fake__fetch_http" should NOT route to builtin -- the namespace means
        // it targets a server called "fake", not the built-in fetch_http.
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "fake__fetch_http",
                "arguments": {"url": "https://example.com"}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        // Should fail -- "fake" server doesn't exist, so it's an error,
        // NOT routed to the builtin handler.
        assert!(resp.error.is_some(), "fake__fetch_http must not route to builtin");
    }

    #[test]
    fn handle_builtin_tool_blocked_by_per_tool_policy() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "fetch_http",
                "arguments": {"url": "https://example.com"}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let mut tool_decisions = std::collections::HashMap::new();
        tool_decisions.insert("fetch_http".to_string(), ToolDecision::Block);
        let policy = McpPolicy {
            tool_decisions,
            ..McpPolicy::new()
        };
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("blocked by policy"));
    }

    #[test]
    fn handle_resources_list_empty() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "resources/list".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_none());
        let resources = resp.result.unwrap()["resources"].as_array().unwrap().clone();
        assert!(resources.is_empty());
    }

    #[test]
    fn handle_resources_read_missing_uri() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "resources/read".into(),
            params: Some(serde_json::json!({})),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().message.contains("missing resource URI"));
    }

    #[test]
    fn truncate_preview_utf8_safe() {
        // Build a string where byte 200 falls inside a 4-byte emoji.
        // Each emoji is 4 bytes, 50 emojis = 200 bytes exactly, but
        // put 49 emojis (196 bytes) + "abcd" (4 bytes) = 200, then add
        // an emoji right at byte 200 boundary to test floor_char_boundary.
        let mut s = String::new();
        for _ in 0..49 {
            s.push('\u{1F600}'); // 4 bytes each = 196 bytes
        }
        s.push_str("abc"); // 199 bytes total
        s.push('\u{1F600}'); // bytes 199..203 -- spans the 200 boundary

        assert!(s.len() > 200);
        // This must NOT panic (the old s[..200] would panic here).
        let truncated = if s.len() > 200 {
            s[..s.floor_char_boundary(200)].to_string()
        } else {
            s.clone()
        };
        // floor_char_boundary(200) should be 199 (before the emoji at 199..203)
        assert_eq!(truncated.len(), 199);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn handle_prompts_list_empty() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "prompts/list".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_none());
        let prompts = resp.result.unwrap()["prompts"].as_array().unwrap().clone();
        assert!(prompts.is_empty());
    }
}
