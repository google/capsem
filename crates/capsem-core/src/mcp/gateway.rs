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

use rmcp::model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams};

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
    /// Auto-snapshot scheduler for file tools (VirtioFS mode only).
    pub auto_snapshots: Option<Arc<tokio::sync::Mutex<crate::auto_snapshot::AutoSnapshotScheduler>>>,
    /// Workspace directory for file tools (VirtioFS mode only).
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
            // Prepend built-in tools (HTTP + file) before external server tools.
            let mut builtin = builtin_tools::builtin_tool_defs();
            if config.workspace_dir.is_some() {
                builtin.extend(super::file_tools::file_tool_defs());
            }
            let mgr = config.server_manager.lock().await;
            let tools: Vec<serde_json::Value> = builtin
                .iter()
                .chain(mgr.tool_catalog().iter())
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

            // Route file tools (VirtioFS mode only).
            if super::file_tools::is_file_tool(tool_name) {
                if let (Some(ref sched), Some(ref ws)) = (&config.auto_snapshots, &config.workspace_dir) {
                    let decision = policy.evaluate("builtin", Some(tool_name));
                    match decision {
                        ToolDecision::Block => {
                            return Some(JsonRpcResponse::err(
                                req.id.clone(),
                                -32600,
                                format!("tool blocked by policy: {tool_name}"),
                            ));
                        }
                        ToolDecision::Warn => {
                            debug!(tool = tool_name, "MCP file tool warned by policy");
                        }
                        ToolDecision::Allow => {}
                    }
                    let mut sched = sched.lock().await;
                    return Some(match tool_name {
                        "snapshots_changes" => {
                            super::file_tools::handle_list_changed_files(&sched, ws, req.id.clone())
                        }
                        "snapshots_list" => {
                            super::file_tools::handle_list_snapshots(&sched, ws, req.id.clone())
                        }
                        "snapshots_revert" => {
                            super::file_tools::handle_revert_file(&arguments, &sched, ws, req.id.clone())
                        }
                        "snapshots_create" => {
                            super::file_tools::handle_snapshot(&arguments, &mut sched, req.id.clone())
                        }
                        "snapshots_delete" => {
                            super::file_tools::handle_delete_snapshot(&arguments, &sched, req.id.clone())
                        }
                        "snapshots_history" => {
                            super::file_tools::handle_snapshots_history(&arguments, &sched, ws, req.id.clone())
                        }
                        "snapshots_compact" => {
                            super::file_tools::handle_snapshots_compact(&arguments, &mut sched, req.id.clone())
                        }
                        _ => JsonRpcResponse::err(req.id.clone(), -32602, format!("unknown file tool: {tool_name}")),
                    });
                } else {
                    return Some(JsonRpcResponse::err(
                        req.id.clone(),
                        -32603,
                        "file tools unavailable (not in VirtioFS mode)",
                    ));
                }
            }

            // Route built-in HTTP tools (no namespace prefix needed).
            if builtin_tools::is_builtin_tool(tool_name) {
                let decision = policy.evaluate("builtin", Some(tool_name));
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

                let dp = config.domain_policy.read().unwrap().clone();
                return Some(builtin_tools::call_builtin_tool(
                    tool_name,
                    &arguments,
                    &config.http_client,
                    &dp,
                    req.id.clone(),
                    &config.db,
                ).await);
            }

            // External server tools: parse namespace prefix.
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

            // Clone peer and drop lock before the (potentially slow) RPC call.
            let (peer, original_name) = {
                let mgr = config.server_manager.lock().await;
                match mgr.lookup_tool_peer(tool_name) {
                    Ok(p) => p,
                    Err(e) => return Some(JsonRpcResponse::err(
                        req.id.clone(), -32603, format!("tool call failed: {e}"),
                    )),
                }
            };

            let args: Option<serde_json::Map<String, serde_json::Value>> = match arguments {
                serde_json::Value::Object(map) if !map.is_empty() => Some(map),
                _ => None,
            };
            let mut params = CallToolRequestParams::new(original_name.clone());
            if let Some(args) = args {
                params = params.with_arguments(args);
            }

            match peer.call_tool(params).await {
                Ok(result) => {
                    let result_json = serde_json::to_value(&result)
                        .unwrap_or(serde_json::json!({}));
                    Some(JsonRpcResponse::ok(req.id.clone(), result_json))
                }
                Err(e) => Some(JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("tool call failed: {e}"),
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

            let (peer, original_uri) = {
                let mgr = config.server_manager.lock().await;
                match mgr.lookup_resource_peer(uri) {
                    Ok(p) => p,
                    Err(e) => return Some(JsonRpcResponse::err(
                        req.id.clone(), -32603, format!("resource read failed: {e}"),
                    )),
                }
            };

            let params = ReadResourceRequestParams::new(original_uri.clone());
            Some(match peer.read_resource(params).await {
                Ok(result) => {
                    let result_json = serde_json::to_value(&result)
                        .unwrap_or(serde_json::json!({}));
                    JsonRpcResponse::ok(req.id.clone(), result_json)
                }
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("resource read failed: {e}"),
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

            let (peer, original_name) = {
                let mgr = config.server_manager.lock().await;
                match mgr.lookup_prompt_peer(prompt_name) {
                    Ok(p) => p,
                    Err(e) => return Some(JsonRpcResponse::err(
                        req.id.clone(), -32603, format!("prompt get failed: {e}"),
                    )),
                }
            };

            let mut params = GetPromptRequestParams::new(original_name.clone());
            if let serde_json::Value::Object(map) = arguments {
                if !map.is_empty() {
                    params = params.with_arguments(map);
                }
            }

            Some(match peer.get_prompt(params).await {
                Ok(result) => {
                    let result_json = serde_json::to_value(&result)
                        .unwrap_or(serde_json::json!({}));
                    JsonRpcResponse::ok(req.id.clone(), result_json)
                }
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(), -32603, format!("prompt get failed: {e}"),
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

    let server_name = match tool_name {
        Some(t) if builtin_tools::is_builtin_tool(t) || super::file_tools::is_file_tool(t) => "builtin",
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
    use crate::net::domain_policy::DomainPolicy;

    fn test_config(rt: &tokio::runtime::Runtime) -> McpGatewayConfig {
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        McpGatewayConfig {
            server_manager: Mutex::new(McpServerManager::new(vec![], reqwest::Client::new())),
            db,
            policy: RwLock::new(Arc::new(McpPolicy::new())),
            domain_policy: std::sync::RwLock::new(Arc::new(DomainPolicy::default_dev())),
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
        assert!(names.contains(&"fetch_http"));
        assert!(names.contains(&"grep_http"));
        assert!(names.contains(&"http_headers"));
        // Names must NOT have the builtin__ prefix
        assert!(!names.iter().any(|n| n.starts_with("builtin__")));
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
                "name": "fetch_http",
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
    fn handle_tools_list_includes_annotations() {
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
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        // All 3 builtins must have annotations with camelCase keys
        for tool in &tools {
            let ann = tool.get("annotations").unwrap_or_else(|| {
                panic!("tool '{}' missing annotations", tool["name"]);
            });
            let obj = ann.as_object().unwrap();
            assert!(obj.contains_key("readOnlyHint"), "missing readOnlyHint in {}", tool["name"]);
            assert!(obj.contains_key("destructiveHint"), "missing destructiveHint in {}", tool["name"]);
            assert!(obj.contains_key("idempotentHint"), "missing idempotentHint in {}", tool["name"]);
            assert!(obj.contains_key("openWorldHint"), "missing openWorldHint in {}", tool["name"]);
            // Must NOT have snake_case keys (wire format violation)
            assert!(!obj.contains_key("read_only_hint"), "snake_case key in wire format: {}", tool["name"]);
        }
    }

    #[test]
    fn handle_tools_list_builtin_annotations_correct() {
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
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        // All 3 builtins are read-only, non-destructive, idempotent, open-world
        for tool in &tools {
            let ann = tool.get("annotations").unwrap();
            assert_eq!(ann["readOnlyHint"], true, "{} should be read-only", tool["name"]);
            assert_eq!(ann["destructiveHint"], false, "{} should not be destructive", tool["name"]);
            assert_eq!(ann["idempotentHint"], true, "{} should be idempotent", tool["name"]);
            assert_eq!(ann["openWorldHint"], true, "{} should be open-world", tool["name"]);
        }
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
    fn handle_builtin_tool_blocked_by_server_policy() {
        // Blocking "builtin" server should block all built-in tools.
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "grep_http",
                "arguments": {"url": "https://example.com", "pattern": "test"}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy {
            blocked_servers: vec!["builtin".to_string()],
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
    fn file_tool_returns_error_without_virtiofs() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "snapshots_create",
                "arguments": {"name": "test"}
            })),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = test_config(&rt);
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("not in VirtioFS mode"));
    }

    #[test]
    fn tools_list_excludes_file_tools_without_virtiofs() {
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
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for file_tool in super::super::file_tools::FILE_TOOL_NAMES {
            assert!(!names.contains(file_tool), "file tool {file_tool} should not appear without VirtioFS");
        }
    }

    #[test]
    fn tools_list_includes_file_tools_with_virtiofs() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/list".into(),
            params: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = rt.block_on(async {
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        let config = McpGatewayConfig {
            server_manager: Mutex::new(McpServerManager::new(vec![], reqwest::Client::new())),
            db,
            policy: RwLock::new(Arc::new(McpPolicy::new())),
            domain_policy: std::sync::RwLock::new(Arc::new(DomainPolicy::default_dev())),
            http_client: reqwest::Client::new(),
            auto_snapshots: None,
            workspace_dir: Some(dir.path().to_path_buf()),
        };
        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        // 3 HTTP builtins + 7 file tools = 10
        assert_eq!(tools.len(), 10);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for file_tool in super::super::file_tools::FILE_TOOL_NAMES {
            assert!(names.contains(file_tool), "file tool {file_tool} missing from tools/list");
        }
    }

    #[test]
    fn file_tool_succeeds_with_virtiofs() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("session");
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::create_dir_all(session_dir.join("auto_snapshots")).unwrap();

        let scheduler = crate::auto_snapshot::AutoSnapshotScheduler::new(
            session_dir.clone(),
            10,
            12,
            std::time::Duration::from_secs(300),
        );
        let scheduler = Arc::new(tokio::sync::Mutex::new(scheduler));

        let db = rt.block_on(async {
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        let config = McpGatewayConfig {
            server_manager: Mutex::new(McpServerManager::new(vec![], reqwest::Client::new())),
            db,
            policy: RwLock::new(Arc::new(McpPolicy::new())),
            domain_policy: std::sync::RwLock::new(Arc::new(DomainPolicy::default_dev())),
            http_client: reqwest::Client::new(),
            auto_snapshots: Some(scheduler),
            workspace_dir: Some(session_dir.join("workspace")),
        };

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "snapshots_create",
                "arguments": {"name": "test_snap"}
            })),
        };

        let policy = McpPolicy::new();
        let resp = rt.block_on(handle_json_rpc(&req, &config, &policy, "test"));
        let resp = resp.unwrap();
        assert!(resp.error.is_none(), "snapshot should succeed: {:?}", resp.error);
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
