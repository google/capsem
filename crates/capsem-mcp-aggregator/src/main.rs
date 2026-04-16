//! MCP aggregator subprocess.
//!
//! Low-privilege process that manages connections to external MCP servers.
//! Communicates with capsem-process via length-prefixed MessagePack frames
//! on stdin/stdout.
//!
//! Protocol:
//! 1. First frame on stdin: msgpack Vec<McpServerDef> (server definitions)
//! 2. Aggregator connects to all enabled HTTP servers
//! 3. Enters frame-based request/response loop
//!
//! Frame format: [4 bytes big-endian payload length] [N bytes msgpack]
//!
//! This subprocess intentionally has NO access to the VM, session DB,
//! filesystem, or service IPC. It only has network access to reach
//! external MCP servers.

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};
use tokio::sync::Mutex;

use capsem_core::mcp::aggregator::*;
use capsem_core::mcp::server_manager::McpServerManager;
use capsem_core::mcp::types::McpServerDef;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "capsem_mcp_aggregator=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    info!("capsem-mcp-aggregator starting");

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    // Step 1: Read server definitions from first frame on stdin.
    let defs: Vec<McpServerDef> = read_frame(&mut stdin)
        .await
        .context("failed to read server definitions")?
        .context("stdin closed before server definitions")?;

    info!(count = defs.len(), "received server definitions");

    // Step 2: Initialize connections to all enabled HTTP servers.
    let manager = Arc::new(Mutex::new(McpServerManager::new(
        defs,
        reqwest::Client::new(),
    )));

    {
        let mut mgr = manager.lock().await;
        if let Err(e) = mgr.initialize_all().await {
            warn!(error = %e, "some MCP servers failed to initialize");
        }
    }

    info!("aggregator ready, entering request loop");

    // Step 3: MessagePack frame request/response loop.
    loop {
        let req: AggregatorRequest = match read_frame(&mut stdin).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                // EOF -- parent closed stdin, shut down gracefully.
                info!("stdin closed, shutting down");
                let mut mgr = manager.lock().await;
                mgr.shutdown_all().await;
                break;
            }
            Err(e) => {
                error!(error = %e, "failed to read request frame");
                continue;
            }
        };

        let is_shutdown = matches!(req.method, AggregatorMethod::Shutdown);
        let resp = handle_request(&manager, req).await;

        if let Err(e) = write_frame(&mut stdout, &resp).await {
            error!(error = %e, "failed to write response frame");
            break;
        }

        if is_shutdown {
            info!("shutdown acknowledged, exiting");
            break;
        }
    }

    Ok(())
}

async fn handle_request(
    manager: &Arc<Mutex<McpServerManager>>,
    req: AggregatorRequest,
) -> AggregatorResponse {
    let id = req.id;

    match req.method {
        AggregatorMethod::ListServers => {
            let mgr = manager.lock().await;
            let servers = mgr
                .definitions()
                .iter()
                .map(|d| AggregatorServerStatus {
                    name: d.name.clone(),
                    url: d.url.clone(),
                    enabled: d.enabled,
                    source: d.source.clone(),
                    is_stdio: d.is_stdio(),
                    connected: mgr.is_running(&d.name),
                    tool_count: mgr.tool_count_for_server(&d.name),
                    resource_count: mgr
                        .resource_catalog()
                        .iter()
                        .filter(|r| r.server_name == d.name)
                        .count(),
                    prompt_count: mgr
                        .prompt_catalog()
                        .iter()
                        .filter(|p| p.server_name == d.name)
                        .count(),
                })
                .collect();
            AggregatorResponse {
                id,
                body: AggregatorResult::Servers { servers },
            }
        }

        AggregatorMethod::ListTools => {
            let mgr = manager.lock().await;
            let tools = mgr.tool_catalog().to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Tools { tools },
            }
        }

        AggregatorMethod::ListResources => {
            let mgr = manager.lock().await;
            let resources = mgr.resource_catalog().to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Resources { resources },
            }
        }

        AggregatorMethod::ListPrompts => {
            let mgr = manager.lock().await;
            let prompts = mgr.prompt_catalog().to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Prompts { prompts },
            }
        }

        AggregatorMethod::CallTool { name, arguments } => {
            let mgr = manager.lock().await;
            match mgr.call_tool(&name, arguments).await {
                Ok(resp) => {
                    let result = resp.result.unwrap_or(serde_json::Value::Null);
                    AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult { result },
                    }
                }
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::ReadResource { uri } => {
            let mgr = manager.lock().await;
            match mgr.read_resource(&uri).await {
                Ok(resp) => {
                    let result = resp.result.unwrap_or(serde_json::Value::Null);
                    AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult { result },
                    }
                }
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::GetPrompt { name, arguments } => {
            let mgr = manager.lock().await;
            match mgr.get_prompt(&name, arguments).await {
                Ok(resp) => {
                    let result = resp.result.unwrap_or(serde_json::Value::Null);
                    AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult { result },
                    }
                }
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::Refresh { servers } => {
            let mut mgr = manager.lock().await;
            debug!(count = servers.len(), "refreshing server definitions");
            mgr.shutdown_all().await;
            *mgr = McpServerManager::new(servers, reqwest::Client::new());
            if let Err(e) = mgr.initialize_all().await {
                warn!(error = %e, "some servers failed during refresh");
            }
            AggregatorResponse {
                id,
                body: AggregatorResult::Ok { ok: true },
            }
        }

        AggregatorMethod::Shutdown => {
            info!("shutdown requested");
            let mut mgr = manager.lock().await;
            mgr.shutdown_all().await;
            AggregatorResponse {
                id,
                body: AggregatorResult::Ok { ok: true },
            }
        }
    }
}
