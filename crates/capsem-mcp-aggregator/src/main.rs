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
//! Wire is full-duplex: the reader spawns one handler per request, handlers
//! send responses through an mpsc channel, and a single writer task drains
//! the channel to stdout. Out-of-order responses are fine because the
//! capsem-process driver routes by request id.
//!
//! This subprocess intentionally has NO access to the VM, session DB,
//! filesystem, or service IPC. It only has network access to reach
//! external MCP servers.

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use capsem_core::mcp::aggregator::*;
use capsem_core::mcp::server_manager::McpServerManager;
use capsem_core::mcp::types::McpServerDef;

#[derive(Parser, Debug)]
#[command(name = "capsem-mcp-aggregator", about = "MCP aggregator subprocess")]
struct Args {
    /// PID of the parent process
    #[arg(long)]
    parent_pid: Option<u32>,

    /// Path for the singleton lockfile
    #[arg(long)]
    lock_path: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // JSON output to stderr (capsem-process redirects it to
    // mcp-aggregator.stderr.log in the VM's session dir). Matches the
    // format capsem-process + capsem-service already emit, so every
    // host-side log is machine-parseable with the same schema.
    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-mcp-aggregator",
        sink: capsem_core::telemetry::LogSink::Stderr,
        default_filter: "capsem_mcp_aggregator=info",
    })?;

    // Root span: every log inherits `vm_id` and `trace_id` as
    // structured fields, so lines in mcp-aggregator.stderr.log can be
    // correlated with parent/service telemetry. `unknown` fallbacks let
    // the binary still run if invoked standalone (dev/debug), without
    // panicking.
    let vm_id = std::env::var("CAPSEM_VM_ID").unwrap_or_else(|_| "unknown".into());
    let trace_id = std::env::var("CAPSEM_TRACE_ID").unwrap_or_else(|_| "unknown".into());
    let profile_id = std::env::var("CAPSEM_PROFILE_ID").unwrap_or_else(|_| "unknown".into());
    let user_id = std::env::var("CAPSEM_USER_ID").unwrap_or_else(|_| "unknown".into());
    let root_span = tracing::info_span!(
        "aggregator",
        vm_id = %vm_id,
        profile_id = %profile_id,
        user_id = %user_id,
        trace_id = %trace_id
    );
    let _root_span_guard = root_span.enter();

    let args = Args::parse();

    if let (Some(pid), Some(lock)) = (args.parent_pid, args.lock_path) {
        match capsem_guard::install(Some(pid), &lock) {
            Ok(Some(guards)) => {
                // Keep the guards alive for the process's lifetime.
                Box::leak(Box::new(guards));
            }
            Ok(None) => {
                info!(lock = %lock.display(), "another instance holds the lock; exiting 0");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "refusing to run without live parent; exiting 0");
                return Ok(());
            }
        }
    }

    info!("capsem-mcp-aggregator starting");

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    // Step 1: Read server definitions from first frame on stdin.
    let defs: Vec<McpServerDef> = read_frame(&mut stdin)
        .await
        .context("failed to read server definitions")?
        .context("stdin closed before server definitions")?;

    info!(count = defs.len(), "received server definitions");

    // Step 2: Initialize connections to all enabled HTTP servers BEFORE
    // installing the manager into the shared lock. `initialize_all` is async
    // and would otherwise need to run while holding the sync RwLock guard.
    let mut mgr = McpServerManager::new(defs, reqwest::Client::new());
    if let Err(e) = mgr.initialize_all().await {
        warn!(error = %e, "some MCP servers failed to initialize");
    }
    let manager = Arc::new(RwLock::new(mgr));

    info!("aggregator ready, entering pipelined request loop");

    // Pipelined session: the reader spawns one handler per request and hands
    // the response back to a single writer task via mpsc. The capsem-process
    // driver matches responses to requests by `id`, so out-of-order delivery
    // is fine. Channel depth 256 is large enough that handlers don't normally
    // block on send, small enough that a stuck writer creates backpressure on
    // the reader instead of growing memory unbounded.
    let (resp_tx, mut resp_rx) = mpsc::channel::<AggregatorResponse>(256);

    let writer_task = tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            if let Err(e) = write_frame(&mut stdout, &resp).await {
                error!(error = %e, "failed to write response frame");
                break;
            }
        }
    });

    let reader_result: Result<()> = async {
        loop {
            let req: AggregatorRequest = match read_frame(&mut stdin).await {
                Ok(Some(r)) => r,
                Ok(None) => {
                    info!("stdin closed, shutting down");
                    return Ok(());
                }
                Err(e) => {
                    error!(error = %e, "failed to read request frame");
                    continue;
                }
            };

            // Ack Shutdown synchronously on the reader path so we can break
            // out cleanly without depending on a spawned handler completing.
            if matches!(req.method, AggregatorMethod::Shutdown) {
                let _ = resp_tx
                    .send(AggregatorResponse {
                        id: req.id,
                        body: AggregatorResult::Ok { ok: true },
                    })
                    .await;
                info!("shutdown acknowledged, exiting");
                return Ok(());
            }

            let mgr_h = Arc::clone(&manager);
            let tx_h = resp_tx.clone();
            tokio::spawn(async move {
                let resp = handle_request(&mgr_h, req).await;
                if tx_h.send(resp).await.is_err() {
                    debug!("aggregator writer channel closed; dropping response");
                }
            });
        }
    }
    .await;

    // Drop our sender so the writer drains in-flight handlers and exits.
    drop(resp_tx);
    let _ = writer_task.await;

    // Drain server connections outside any lock. Take ownership of the running
    // map under a brief write guard, then await cancellation after the guard
    // drops.
    let drain_fut = {
        let mut mgr = manager.write().expect("manager rwlock poisoned");
        mgr.drain_running()
    };
    drain_fut.await;

    reader_result
}

async fn handle_request(
    manager: &Arc<RwLock<McpServerManager>>,
    req: AggregatorRequest,
) -> AggregatorResponse {
    let id = req.id;

    match req.method {
        AggregatorMethod::ListServers => {
            let mgr = manager.read().expect("manager rwlock poisoned");
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
            let tools = manager
                .read()
                .expect("manager rwlock poisoned")
                .tool_catalog()
                .to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Tools { tools },
            }
        }

        AggregatorMethod::ListResources => {
            let resources = manager
                .read()
                .expect("manager rwlock poisoned")
                .resource_catalog()
                .to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Resources { resources },
            }
        }

        AggregatorMethod::ListPrompts => {
            let prompts = manager
                .read()
                .expect("manager rwlock poisoned")
                .prompt_catalog()
                .to_vec();
            AggregatorResponse {
                id,
                body: AggregatorResult::Prompts { prompts },
            }
        }

        AggregatorMethod::CallTool { name, arguments } => {
            // Resolve the dispatch under a sync read guard, then drop the
            // guard before awaiting the rmcp RPC. Concurrent CallTool
            // handlers proceed in parallel; the read lock never crosses an
            // `.await`.
            let dispatch = manager
                .read()
                .expect("manager rwlock poisoned")
                .dispatch_call_tool(&name, arguments);
            match dispatch {
                Ok(fut) => match fut.await {
                    Ok(resp) => AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult {
                            result: resp.result.unwrap_or(serde_json::Value::Null),
                        },
                    },
                    Err(e) => AggregatorResponse {
                        id,
                        body: AggregatorResult::Error {
                            error: e.to_string(),
                        },
                    },
                },
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::ReadResource { uri } => {
            let dispatch = manager
                .read()
                .expect("manager rwlock poisoned")
                .dispatch_read_resource(&uri);
            match dispatch {
                Ok(fut) => match fut.await {
                    Ok(resp) => AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult {
                            result: resp.result.unwrap_or(serde_json::Value::Null),
                        },
                    },
                    Err(e) => AggregatorResponse {
                        id,
                        body: AggregatorResult::Error {
                            error: e.to_string(),
                        },
                    },
                },
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::GetPrompt { name, arguments } => {
            let dispatch = manager
                .read()
                .expect("manager rwlock poisoned")
                .dispatch_get_prompt(&name, arguments);
            match dispatch {
                Ok(fut) => match fut.await {
                    Ok(resp) => AggregatorResponse {
                        id,
                        body: AggregatorResult::CallResult {
                            result: resp.result.unwrap_or(serde_json::Value::Null),
                        },
                    },
                    Err(e) => AggregatorResponse {
                        id,
                        body: AggregatorResult::Error {
                            error: e.to_string(),
                        },
                    },
                },
                Err(e) => AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                },
            }
        }

        AggregatorMethod::Refresh { servers } => {
            debug!(count = servers.len(), "refreshing server definitions");

            // Drain old running servers without holding the lock across .await.
            let drain_fut = {
                let mut mgr = manager.write().expect("manager rwlock poisoned");
                mgr.drain_running()
            };
            drain_fut.await;

            // Build and initialize the replacement manager off the lock,
            // then swap it in under a brief write guard.
            let mut new_mgr = McpServerManager::new(servers, reqwest::Client::new());
            let refresh_error = new_mgr.initialize_all_strict().await.err();
            *manager.write().expect("manager rwlock poisoned") = new_mgr;

            if let Some(e) = refresh_error {
                warn!(error = %e, "some servers failed during refresh");
                AggregatorResponse {
                    id,
                    body: AggregatorResult::Error {
                        error: e.to_string(),
                    },
                }
            } else {
                AggregatorResponse {
                    id,
                    body: AggregatorResult::Ok { ok: true },
                }
            }
        }

        AggregatorMethod::Shutdown => {
            // The reader path acks Shutdown directly before this handler runs,
            // so this branch is only reached if a stray Shutdown gets spawned
            // (it shouldn't). Drain and ack defensively.
            info!("shutdown reached spawned handler -- draining defensively");
            let drain_fut = {
                let mut mgr = manager.write().expect("manager rwlock poisoned");
                mgr.drain_running()
            };
            drain_fut.await;
            AggregatorResponse {
                id,
                body: AggregatorResult::Ok { ok: true },
            }
        }
    }
}
