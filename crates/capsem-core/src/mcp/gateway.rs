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
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use capsem_logger::{DbWriter, McpCall, WriteOp};

use crate::net::domain_policy::DomainPolicy;

use super::builtin_tools;
use super::policy::{McpPolicy, ToolDecision};
use super::server_manager::McpServerManager;
use super::types::*;

/// Maximum NDJSON line length (1MB). Reject lines larger than this.
const MAX_LINE_LEN: usize = 1_048_576;

/// Shared configuration for the MCP gateway.
pub struct McpGatewayConfig {
    pub server_manager: Mutex<McpServerManager>,
    pub db: Arc<DbWriter>,
    /// Double-Arc for atomic policy swap: outer RwLock protects inner Arc.
    /// New sessions clone the inner Arc for a consistent snapshot.
    pub policy: RwLock<Arc<McpPolicy>>,
    /// Domain policy for built-in HTTP tools (hot-reloadable).
    pub domain_policy: std::sync::RwLock<Arc<DomainPolicy>>,
    /// HTTP client for built-in tools.
    pub http_client: reqwest::Client,
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

    let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
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
            // Prepend built-in tools before external server tools.
            let builtin = builtin_tools::builtin_tool_defs();
            let mgr = config.server_manager.lock().await;
            let tools: Vec<serde_json::Value> = builtin
                .iter()
                .chain(mgr.tool_catalog().iter())
                .map(|t| {
                    serde_json::json!({
                        "name": t.namespaced_name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
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

            let (server_name, local_name) = parse_namespaced(tool_name)
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

            let arguments = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::json!({}));

            // Route built-in tools to the builtin handler.
            if server_name == "builtin" {
                let dp = config.domain_policy.read().unwrap().clone();
                return Some(builtin_tools::call_builtin_tool(
                    local_name,
                    &arguments,
                    &config.http_client,
                    &dp,
                    req.id.clone(),
                ).await);
            }

            let mut mgr = config.server_manager.lock().await;
            match mgr.call_tool(tool_name, arguments).await {
                Ok(resp) => Some(resp),
                Err(e) => Some(JsonRpcResponse::err(
                    req.id.clone(),
                    -32603,
                    format!("tool call failed: {e}"),
                )),
            }
        }

        "resources/list" => {
            let mgr = config.server_manager.lock().await;
            let resources: Vec<serde_json::Value> = mgr
                .resource_catalog()
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

            let mut mgr = config.server_manager.lock().await;
            Some(match mgr.read_resource(uri).await {
                Ok(resp) => resp,
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(),
                    -32603,
                    format!("resource read failed: {e}"),
                ),
            })
        }

        "prompts/list" => {
            let mgr = config.server_manager.lock().await;
            let prompts: Vec<serde_json::Value> = mgr
                .prompt_catalog()
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

            let mut mgr = config.server_manager.lock().await;
            Some(match mgr.get_prompt(prompt_name, arguments).await {
                Ok(resp) => resp,
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(),
                    -32603,
                    format!("prompt get failed: {e}"),
                ),
            })
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

    let server_name = tool_name
        .and_then(|t| parse_namespaced(t).map(|(s, _)| s))
        .unwrap_or("gateway");

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

    let req_preview = req
        .params
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok())
        .map(|s| if s.len() > 200 { s[..200].to_string() } else { s });

    let resp_preview = resp
        .result
        .as_ref()
        .and_then(|r| serde_json::to_string(r).ok())
        .map(|s| if s.len() > 200 { s[..200].to_string() } else { s });

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
    };

    config.db.write(WriteOp::McpCall(call)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::domain_policy::DomainPolicy;

    fn test_config(rt: &tokio::runtime::Runtime) -> McpGatewayConfig {
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        McpGatewayConfig {
            server_manager: Mutex::new(McpServerManager::new(vec![])),
            db,
            policy: RwLock::new(Arc::new(McpPolicy::new())),
            domain_policy: std::sync::RwLock::new(Arc::new(DomainPolicy::default_dev())),
            http_client: reqwest::Client::new(),
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

    #[test]
    fn handle_tools_list_returns_builtin_tools() {
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
        // 3 built-in tools, no external servers
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"builtin__fetch_http"));
        assert!(names.contains(&"builtin__grep_http"));
        assert!(names.contains(&"builtin__http_headers"));
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
    fn handle_builtin_tool_call_routes_to_builtin() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "builtin__fetch_http",
                "arguments": {"url": "https://evil-unknown-domain.xyz"}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        // Should route to builtin handler (domain blocked by policy, returns isError)
        assert!(resp.error.is_none()); // tool errors use isError in result
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("blocked"));
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
}
