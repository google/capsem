use anyhow::Result;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_unix_ipc::{channel_from_std, Receiver, Sender};
use tracing::{debug, error, info, warn};

use crate::job_store::{JobResult, JobStore};
use crate::mcp_runtime::McpRuntime;
use crate::runtime_config::RuntimeProfileSource;
use crate::terminal::TerminalRelay;

type SharedSnapshotScheduler =
    Arc<tokio::sync::Mutex<capsem_core::auto_snapshot::AutoSnapshotScheduler>>;

/// Per-attempt timeout the host watchdog waits before re-sending a quick
/// request/response HostToGuest payload.
///
/// With the control bridge's pending-ack map (see
/// `JobStore::pending_acks` and `vsock.rs::setup_vsock`), silent
/// host-to-guest drops are no longer the watchdog's job: the bridge
/// holds every ackable message and replays it on every fresh conn
/// until `GuestToHost::Ack` lands. The watchdog only exists to cover
/// the asymmetric return-path case where the agent processed and
/// sent the response (FileOpDone / FileContent) but those
/// bytes were silently dropped. Retrying quick file operations triggers the
/// agent's dedup-and-replay path, which re-emits the cached response.
///
/// 1s is chosen so a retry only fires after a request has clearly
/// missed its expected response window: the longest healthy guest
/// round-trip we observed was ~150ms (bash spawn + mkdir + echo +
/// cat), and 1s gives ~6× headroom over that without sitting idle
/// for 3s of dead time.
const GUEST_PAYLOAD_TIMEOUT: Duration = Duration::from_secs(1);
/// Maximum number of quick-operation watchdog retries. 16 × 1s = 16s. The bridge
/// replay layer takes care of forward-path losses regardless of this
/// number; the watchdog's job is just to cover return-path losses.
const GUEST_PAYLOAD_MAX_RETRIES: u16 = 16;

async fn await_exec_result(j_rx: oneshot::Receiver<JobResult>) -> Result<JobResult, String> {
    j_rx.await
        .map_err(|_| "Exec result channel closed".to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_ipc_connection(
    stream: tokio::net::UnixStream,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    ipc_tx: broadcast::Sender<ProcessToService>,
    term_relay: Arc<TerminalRelay>,
    job_store: Arc<JobStore>,
    net_state: Arc<capsem_core::SandboxNetworkState>,
    mcp_runtime: Arc<McpRuntime>,
    runtime_source: RuntimeProfileSource,
    mcp_builtin_binary: Option<PathBuf>,
    mcp_builtin_env: HashMap<String, String>,
    snapshot_scheduler: SharedSnapshotScheduler,
    vm_ready: Arc<AtomicBool>,
) -> Result<()> {
    let mut std_stream = stream.into_std()?;
    // First frame on every IPC connection is a Hello -- detect cross-version
    // mixes (capsem-service built before X, capsem-process built after) in
    // ~1s with a structured log line instead of a 30s silent timeout.
    match capsem_core::ipc_handshake::negotiate_responder(
        &mut std_stream,
        "capsem-process",
        capsem_core::telemetry::current_parent_traceparent(),
    ) {
        Ok(peer) => {
            info!(target: "ipc", peer = %peer.peer, "IPC handshake ok");
        }
        Err(e) => {
            error!(
                target: "ipc",
                error = %e,
                "IPC handshake failed; refusing connection"
            );
            return Ok(());
        }
    }
    let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) =
        channel_from_std(std_stream)?;

    // Serialize all IPC writes through a single channel to prevent concurrent
    // sendmsg() interleaving that corrupts the data stream. tokio_unix_ipc's
    // Sender::send() writes header + payload as two separate syscalls with no
    // internal locking, so concurrent use from multiple tasks is unsafe.
    let (ipc_tx_out, mut ipc_rx_out) = mpsc::channel::<ProcessToService>(256);
    tokio::spawn(async move {
        while let Some(msg) = ipc_rx_out.recv().await {
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Every connection receives low-volume lifecycle events (StateChanged,
    // ShutdownRequested, SuspendRequested) from the broadcast. TerminalOutput
    // is high-volume and still opt-in via StartTerminalStream. Without this,
    // a suspend-only connection never sees StateChanged { state: "Suspended" }
    // and the service times out waiting for confirmation.
    {
        let out_tx = ipc_tx_out.clone();
        let mut rx_bcast = ipc_tx.subscribe();
        tokio::spawn(async move {
            while let Ok(msg) = rx_bcast.recv().await {
                if matches!(msg, ProcessToService::TerminalOutput { .. }) {
                    continue;
                }
                if out_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });
    }

    // Live stream task spawned by StartTerminalStream. Held here so
    // StopTerminalStream and connection teardown can abort it instead of
    // letting it outlive the IPC connection.
    let mut stream_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(e) => {
                // Surface the decode error -- silent connection close on a
                // protocol mismatch wedged suspend for a full afternoon
                // while we hunted it as a "guest doesn't respond" bug.
                tracing::warn!(error = %e, "IPC: rx.recv() failed; closing connection");
                break;
            }
        };
        match msg {
            ServiceToProcess::StartTerminalStream => {
                info!("Starting terminal stream for connection");
                // Track the stream task so StopTerminalStream / connection
                // teardown can cancel it. Cancelling on stop prevents
                // late TerminalOutput frames from racing the client's
                // raw-mode-restore and leaking into the user's parent
                // shell.
                let prev: Option<tokio::task::JoinHandle<()>> = stream_task.take();
                if let Some(p) = prev {
                    p.abort();
                }
                let out_tx = ipc_tx_out.clone();
                // Atomic snapshot + subscribe so the client sees buffered
                // banner bytes and then a gap-free live stream.
                let (replay, mut term_rx) = term_relay.subscribe();
                let h = tokio::spawn(async move {
                    // Smoking-gun trace. If a buffer at the head of the
                    // PTY stream looks like an IPC frame, log it loudly
                    // -- something inside the guest is writing protocol
                    // bytes to its PTY and we want to know which session,
                    // which buffer prefix, and how often.
                    let warn_if_ipc_shaped = |buf: &[u8]| {
                        if capsem_proto::looks_like_ipc_frame(buf) {
                            let preview: Vec<String> =
                                buf.iter().take(16).map(|b| format!("{:02x}", b)).collect();
                            warn!(
                                    bytes = preview.join(" "),
                                    len = buf.len(),
                                    "PTY stream starts with IPC-frame-shaped bytes -- guest may be leaking control data into the terminal channel"
                                );
                        }
                    };
                    if !replay.is_empty() {
                        warn_if_ipc_shaped(&replay);
                        if out_tx
                            .send(ProcessToService::TerminalOutput { data: replay })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    while let Ok(data) = term_rx.recv().await {
                        warn_if_ipc_shaped(&data);
                        if out_tx
                            .send(ProcessToService::TerminalOutput { data })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                stream_task = Some(h);
            }
            ServiceToProcess::StopTerminalStream => {
                info!("Stopping terminal stream for connection");
                if let Some(h) = stream_task.take() {
                    h.abort();
                }
            }
            ServiceToProcess::Ping => {
                if vm_ready.load(Ordering::Acquire) {
                    capsem_core::try_send!(
                        "ipc_pong",
                        ipc_tx_out.send(ProcessToService::Pong).await
                    );
                } else {
                    debug!("Ping received but VM not ready, closing connection");
                    return Ok(());
                }
            }
            ServiceToProcess::TerminalInput { data } => {
                capsem_core::try_send!(
                    "ctrl_terminal_input",
                    ctrl_tx.send(ServiceToProcess::TerminalInput { data }).await
                );
            }
            ServiceToProcess::TerminalResize { cols, rows } => {
                capsem_core::try_send!(
                    "ctrl_terminal_resize",
                    ctrl_tx
                        .send(ServiceToProcess::TerminalResize { cols, rows })
                        .await
                );
            }
            ServiceToProcess::Exec { id, command } => {
                let job_store = job_store.clone();
                let ctrl_tx = ctrl_tx.clone();
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    info!(id, command, "Received Exec command via IPC");
                    let (j_tx, j_rx) = oneshot::channel();
                    job_store.jobs.lock().unwrap().insert(id, j_tx);

                    // Reset active_exec for this id. The EXEC-port
                    // handler keys its capture by active_exec.id, so
                    // this slot must be in place *before* we send.
                    *job_store.active_exec.lock().unwrap() =
                        Some(crate::job_store::ActiveExec::new(id));

                    capsem_core::try_send!(
                        "ctrl_exec",
                        ctrl_tx
                            .send(ServiceToProcess::Exec {
                                id,
                                command: command.clone()
                            })
                            .await
                    );

                    // Exec duration is user work, not transport liveness.
                    // The control bridge's Ack/AckReply replay layers own
                    // delivery in both directions; this task waits until
                    // the guest command exits, while the service caller may
                    // apply an explicit timeout when requested.
                    let result = await_exec_result(j_rx).await;
                    match result {
                        Ok(JobResult::Exec {
                            stdout,
                            stderr,
                            exit_code,
                        }) => {
                            info!(id, exit_code, "Sending ExecResult back via IPC");
                            capsem_core::try_send!(
                                "ipc_exec_result",
                                ipc_tx_out
                                    .send(ProcessToService::ExecResult {
                                        id,
                                        stdout,
                                        stderr,
                                        exit_code
                                    })
                                    .await
                            );
                        }
                        Ok(JobResult::Error { message }) => {
                            error!(id, message, "Sending Exec error back via IPC");
                            capsem_core::try_send!(
                                "ipc_exec_result_err",
                                ipc_tx_out
                                    .send(ProcessToService::ExecResult {
                                        id,
                                        stdout: vec![],
                                        stderr: message.into_bytes(),
                                        exit_code: -1
                                    })
                                    .await
                            );
                        }
                        Ok(other) => {
                            error!(id, result = ?other, "unexpected job result for Exec");
                        }
                        Err(msg) => {
                            error!(id, msg, "Exec result channel closed");
                            let _ = job_store.jobs.lock().unwrap().remove(&id);
                            // No caller is still waiting on this IPC job;
                            // remove the pending-ack entry so the bridge
                            // stops replaying it.
                            job_store.pending_acks.lock().unwrap().remove(&id);
                            capsem_core::try_send!(
                                "ipc_exec_result_closed",
                                ipc_tx_out
                                    .send(ProcessToService::ExecResult {
                                        id,
                                        stdout: vec![],
                                        stderr: msg.into_bytes(),
                                        exit_code: -1,
                                    })
                                    .await
                            );
                        }
                    }
                });
            }
            ServiceToProcess::WriteFile { id, path, data } => {
                let job_store = job_store.clone();
                let ctrl_tx = ctrl_tx.clone();
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    info!(
                        id,
                        path,
                        len = data.len(),
                        "Received WriteFile command via IPC"
                    );
                    let (j_tx, mut j_rx) = oneshot::channel();
                    job_store.jobs.lock().unwrap().insert(id, j_tx);
                    capsem_core::try_send!(
                        "ctrl_write_file",
                        ctrl_tx
                            .send(ServiceToProcess::WriteFile {
                                id,
                                path: path.clone(),
                                data: data.clone()
                            })
                            .await
                    );
                    // Watchdog mirrors the quick FileRead retry: if no
                    // FileOpDone arrives within the short response window,
                    // re-send the FileWrite. The agent dedups by id, so a
                    // retry that races a late-arriving original cannot
                    // double-write.
                    let mut retries: u16 = 0;
                    let result = loop {
                        match tokio::time::timeout(GUEST_PAYLOAD_TIMEOUT, &mut j_rx).await {
                            Ok(res) => break Ok(res),
                            Err(_) => {
                                if retries >= GUEST_PAYLOAD_MAX_RETRIES {
                                    break Err("FileWrite watchdog exhausted retries");
                                }
                                retries += 1;
                                warn!(
                                    id,
                                    attempt = retries,
                                    "no FileOp ack in 1s; resending HostToGuest::FileWrite"
                                );
                                capsem_core::try_send!(
                                    "ctrl_write_file_retry",
                                    ctrl_tx
                                        .send(ServiceToProcess::WriteFile {
                                            id,
                                            path: path.clone(),
                                            data: data.clone()
                                        })
                                        .await
                                );
                            }
                        }
                    };
                    match result {
                        Ok(Ok(JobResult::WriteFile { success, error })) => {
                            info!(id, success, "Sending WriteFileResult back via IPC");
                            capsem_core::try_send!(
                                "ipc_write_file_result",
                                ipc_tx_out
                                    .send(ProcessToService::WriteFileResult { id, success, error })
                                    .await
                            );
                        }
                        Ok(Ok(JobResult::Error { message })) => {
                            error!(id, message, "Sending WriteFile error back via IPC");
                            capsem_core::try_send!(
                                "ipc_write_file_result_err",
                                ipc_tx_out
                                    .send(ProcessToService::WriteFileResult {
                                        id,
                                        success: false,
                                        error: Some(message)
                                    })
                                    .await
                            );
                        }
                        Ok(_) => {
                            error!(id, "Job result channel closed for WriteFile");
                        }
                        Err(msg) => {
                            error!(id, msg, "WriteFile watchdog exhausted");
                            // Surface a structured failure to the
                            // service so it can fail the request quickly
                            // instead of waiting for the IPC envelope.
                            let _ = job_store.jobs.lock().unwrap().remove(&id);
                            // Watchdog gave up -- remove the pending-ack
                            // entry so the bridge stops replaying a
                            // message no caller is still waiting for.
                            job_store.pending_acks.lock().unwrap().remove(&id);
                            capsem_core::try_send!(
                                "ipc_write_file_result_watchdog",
                                ipc_tx_out
                                    .send(ProcessToService::WriteFileResult {
                                        id,
                                        success: false,
                                        error: Some(msg.into()),
                                    })
                                    .await
                            );
                        }
                    }
                });
            }
            ServiceToProcess::ReadFile { id, path } => {
                let job_store = job_store.clone();
                let ctrl_tx = ctrl_tx.clone();
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    info!(id, path, "Received ReadFile command via IPC");
                    let (j_tx, mut j_rx) = oneshot::channel();
                    job_store.jobs.lock().unwrap().insert(id, j_tx);
                    capsem_core::try_send!(
                        "ctrl_read_file",
                        ctrl_tx
                            .send(ServiceToProcess::ReadFile {
                                id,
                                path: path.clone()
                            })
                            .await
                    );
                    let mut retries: u16 = 0;
                    let result = loop {
                        match tokio::time::timeout(GUEST_PAYLOAD_TIMEOUT, &mut j_rx).await {
                            Ok(res) => break Ok(res),
                            Err(_) => {
                                if retries >= GUEST_PAYLOAD_MAX_RETRIES {
                                    break Err("FileRead watchdog exhausted retries");
                                }
                                retries += 1;
                                warn!(
                                    id,
                                    attempt = retries,
                                    "no FileContent in 1s; resending HostToGuest::FileRead"
                                );
                                capsem_core::try_send!(
                                    "ctrl_read_file_retry",
                                    ctrl_tx
                                        .send(ServiceToProcess::ReadFile {
                                            id,
                                            path: path.clone()
                                        })
                                        .await
                                );
                            }
                        }
                    };
                    match result {
                        Ok(Ok(JobResult::ReadFile { data, error })) => {
                            info!(
                                id,
                                success = data.is_some(),
                                "Sending ReadFileResult back via IPC"
                            );
                            capsem_core::try_send!(
                                "ipc_read_file_result",
                                ipc_tx_out
                                    .send(ProcessToService::ReadFileResult { id, data, error })
                                    .await
                            );
                        }
                        Ok(Ok(JobResult::Error { message })) => {
                            error!(id, message, "Sending ReadFile error back via IPC");
                            capsem_core::try_send!(
                                "ipc_read_file_result_err",
                                ipc_tx_out
                                    .send(ProcessToService::ReadFileResult {
                                        id,
                                        data: None,
                                        error: Some(message)
                                    })
                                    .await
                            );
                        }
                        Ok(_) => {
                            error!(id, "Job result channel closed for ReadFile");
                        }
                        Err(msg) => {
                            error!(id, msg, "ReadFile watchdog exhausted");
                            let _ = job_store.jobs.lock().unwrap().remove(&id);
                            // Watchdog gave up -- remove the pending-ack
                            // entry so the bridge stops replaying a
                            // message no caller is still waiting for.
                            job_store.pending_acks.lock().unwrap().remove(&id);
                            capsem_core::try_send!(
                                "ipc_read_file_result_watchdog",
                                ipc_tx_out
                                    .send(ProcessToService::ReadFileResult {
                                        id,
                                        data: None,
                                        error: Some(msg.into()),
                                    })
                                    .await
                            );
                        }
                    }
                });
            }
            ServiceToProcess::LogFileBoundary {
                id,
                action,
                path,
                data,
                size,
                mime_type,
            } => {
                let job_store = job_store.clone();
                let ctrl_tx = ctrl_tx.clone();
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    info!(
                        id,
                        ?action,
                        path,
                        size,
                        "Received LogFileBoundary command via IPC"
                    );
                    let (j_tx, j_rx) = oneshot::channel();
                    job_store.jobs.lock().unwrap().insert(id, j_tx);
                    capsem_core::try_send!(
                        "ctrl_log_file_boundary",
                        ctrl_tx
                            .send(ServiceToProcess::LogFileBoundary {
                                id,
                                action,
                                path,
                                data,
                                size,
                                mime_type,
                            })
                            .await
                    );
                    match tokio::time::timeout(Duration::from_secs(5), j_rx).await {
                        Ok(Ok(JobResult::LogFileBoundary {
                            success,
                            data,
                            error,
                        })) => {
                            capsem_core::try_send!(
                                "ipc_log_file_boundary_result",
                                ipc_tx_out
                                    .send(ProcessToService::LogFileBoundaryResult {
                                        id,
                                        success,
                                        data,
                                        error,
                                    })
                                    .await
                            );
                        }
                        Ok(Ok(JobResult::Error { message })) => {
                            capsem_core::try_send!(
                                "ipc_log_file_boundary_result_err",
                                ipc_tx_out
                                    .send(ProcessToService::LogFileBoundaryResult {
                                        id,
                                        success: false,
                                        data: None,
                                        error: Some(message),
                                    })
                                    .await
                            );
                        }
                        Ok(Ok(other)) => {
                            error!(id, result = ?other, "unexpected job result for LogFileBoundary");
                            capsem_core::try_send!(
                                "ipc_log_file_boundary_result_unexpected",
                                ipc_tx_out
                                    .send(ProcessToService::LogFileBoundaryResult {
                                        id,
                                        success: false,
                                        data: None,
                                        error: Some("unexpected log file boundary result".into()),
                                    })
                                    .await
                            );
                        }
                        Ok(Err(_)) => {
                            let _ = job_store.jobs.lock().unwrap().remove(&id);
                            capsem_core::try_send!(
                                "ipc_log_file_boundary_result_closed",
                                ipc_tx_out
                                    .send(ProcessToService::LogFileBoundaryResult {
                                        id,
                                        success: false,
                                        data: None,
                                        error: Some(
                                            "log file boundary result channel closed".into()
                                        ),
                                    })
                                    .await
                            );
                        }
                        Err(_) => {
                            let _ = job_store.jobs.lock().unwrap().remove(&id);
                            capsem_core::try_send!(
                                "ipc_log_file_boundary_result_timeout",
                                ipc_tx_out
                                    .send(ProcessToService::LogFileBoundaryResult {
                                        id,
                                        success: false,
                                        data: None,
                                        error: Some("log file boundary timed out".into()),
                                    })
                                    .await
                            );
                        }
                    }
                });
            }
            ServiceToProcess::ReloadConfig => {
                info!(
                    active_profile = %runtime_source.active_profile_path().display(),
                    "Reloading profile runtime config"
                );
                let runtime_config = runtime_source.load()?;

                let new_network = Arc::new(runtime_config.network);
                let new_security_rules = Arc::new(runtime_config.security_rules);
                let new_plugin_policy = runtime_config.plugins;
                let new_model_endpoints = Arc::new(runtime_config.model_endpoints);

                *net_state.policy.write().unwrap() = new_network;
                *mcp_runtime.security_rules.write().unwrap() = new_security_rules;
                *mcp_runtime.plugin_policy.write().unwrap() = new_plugin_policy;
                *mcp_runtime.model_endpoints.write().unwrap() = new_model_endpoints;

                capsem_core::try_send!(
                    "ipc_pong_reload",
                    ipc_tx_out.send(ProcessToService::Pong).await
                );
            }
            ServiceToProcess::Shutdown => {
                capsem_core::try_send!(
                    "ctrl_shutdown",
                    ctrl_tx.send(ServiceToProcess::Shutdown).await
                );
                info!("Received Shutdown command, exiting IPC loop gracefully");
                break;
            }
            ServiceToProcess::Suspend { checkpoint_path } => {
                info!("Received Suspend command, forwarding to ctrl channel");
                capsem_core::try_send!(
                    "ctrl_suspend",
                    ctrl_tx
                        .send(ServiceToProcess::Suspend { checkpoint_path })
                        .await
                );
            }
            ServiceToProcess::McpListServers { id } => {
                let mcp = Arc::clone(&mcp_runtime);
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    match mcp.aggregator.list_servers().await {
                        Ok(agg_servers) => {
                            let servers = agg_servers
                                .into_iter()
                                .map(|s| capsem_proto::ipc::McpServerStatus {
                                    name: s.name,
                                    url: s.url,
                                    enabled: s.enabled,
                                    source: s.source,
                                    is_stdio: s.is_stdio,
                                    connected: s.connected,
                                    tool_count: s.tool_count,
                                })
                                .collect();
                            capsem_core::try_send!(
                                "ipc_mcp_servers",
                                ipc_tx_out
                                    .send(ProcessToService::McpServersResult { id, servers })
                                    .await
                            );
                        }
                        Err(e) => {
                            capsem_core::try_send!(
                                "ipc_mcp_servers_err",
                                ipc_tx_out
                                    .send(ProcessToService::McpServersResult {
                                        id,
                                        servers: vec![]
                                    })
                                    .await
                            );
                            warn!(error = %e, "failed to list MCP servers");
                        }
                    }
                });
            }
            ServiceToProcess::McpListTools { id } => {
                let mcp = Arc::clone(&mcp_runtime);
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    match mcp.aggregator.list_tools().await {
                        Ok(tools) => {
                            let tools = tools
                                .into_iter()
                                .map(|t| capsem_proto::ipc::McpToolStatus {
                                    namespaced_name: t.namespaced_name,
                                    original_name: t.original_name,
                                    description: t.description,
                                    server_name: t.server_name,
                                    annotations: t.annotations.as_ref().map(|a| a.to_mcp_json()),
                                })
                                .collect();
                            capsem_core::try_send!(
                                "ipc_mcp_tools",
                                ipc_tx_out
                                    .send(ProcessToService::McpToolsResult { id, tools })
                                    .await
                            );
                        }
                        Err(e) => {
                            capsem_core::try_send!(
                                "ipc_mcp_tools_err",
                                ipc_tx_out
                                    .send(ProcessToService::McpToolsResult { id, tools: vec![] })
                                    .await
                            );
                            warn!(error = %e, "failed to list MCP tools");
                        }
                    }
                });
            }
            ServiceToProcess::McpRefreshTools { id } => {
                let mcp = Arc::clone(&mcp_runtime);
                let ipc_tx_out = ipc_tx_out.clone();
                let runtime_source = runtime_source.clone();
                let mcp_builtin_binary = mcp_builtin_binary.clone();
                let mcp_builtin_env = mcp_builtin_env.clone();
                tokio::spawn(async move {
                    let runtime_config = match runtime_source.load() {
                        Ok(config) => config,
                        Err(e) => {
                            capsem_core::try_send!(
                                "ipc_mcp_refresh_profile_load_err",
                                ipc_tx_out
                                    .send(ProcessToService::McpRefreshResult {
                                        id,
                                        success: false,
                                        error: Some(e.to_string())
                                    })
                                    .await
                            );
                            return;
                        }
                    };
                    let servers =
                        runtime_config.mcp_servers(mcp_builtin_binary.as_deref(), mcp_builtin_env);
                    match mcp.aggregator.refresh(servers).await {
                        Ok(()) => {
                            capsem_core::try_send!(
                                "ipc_mcp_refresh",
                                ipc_tx_out
                                    .send(ProcessToService::McpRefreshResult {
                                        id,
                                        success: true,
                                        error: None
                                    })
                                    .await
                            );
                        }
                        Err(e) => {
                            capsem_core::try_send!(
                                "ipc_mcp_refresh_err",
                                ipc_tx_out
                                    .send(ProcessToService::McpRefreshResult {
                                        id,
                                        success: false,
                                        error: Some(e.to_string())
                                    })
                                    .await
                            );
                        }
                    }
                });
            }
            ServiceToProcess::SnapshotStatus { id } => {
                let scheduler = Arc::clone(&snapshot_scheduler);
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    let status = {
                        let scheduler = scheduler.lock().await;
                        snapshot_status_from_scheduler(&scheduler)
                    };
                    capsem_core::try_send!(
                        "ipc_snapshot_status",
                        ipc_tx_out
                            .send(ProcessToService::SnapshotStatusResult { id, status })
                            .await
                    );
                });
            }
            ServiceToProcess::McpCallTool {
                id,
                namespaced_name,
                arguments_json,
            } => {
                let mcp = Arc::clone(&mcp_runtime);
                let ipc_tx_out = ipc_tx_out.clone();
                tokio::spawn(async move {
                    // arguments travels as a JSON string because bincode
                    // (tokio-unix-ipc's wire format) cannot round-trip
                    // serde_json::Value through its non-self-describing
                    // deserialize_any. See crates/capsem-proto/src/ipc.rs.
                    let arguments: serde_json::Value =
                        serde_json::from_str(&arguments_json).unwrap_or(serde_json::Value::Null);
                    let request = capsem_core::mcp::types::JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        id: Some(serde_json::json!(id)),
                        method: "tools/call".to_string(),
                        params: Some(serde_json::json!({
                            "name": namespaced_name,
                            "arguments": arguments,
                        })),
                        meta: None,
                    };
                    let response = capsem_core::net::mitm_proxy::dispatch_logged_mcp_request(
                        Arc::clone(&mcp.endpoint),
                        Arc::clone(&mcp.db),
                        request,
                        "capsem-service".to_string(),
                    )
                    .await;
                    let result_json = response
                        .as_ref()
                        .and_then(|result| serde_json::to_string(&result.response).ok());
                    let event_id = response.as_ref().and_then(|result| result.event_id.clone());
                    let security_rule_events_json = response
                        .as_ref()
                        .map(|result| {
                            result
                                .security_rule_events
                                .iter()
                                .filter_map(|event| serde_json::to_string(event).ok())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let error = response
                        .as_ref()
                        .and_then(|result| result.response.error.as_ref())
                        .map(|error| error.message.clone())
                        .or_else(|| {
                            response
                                .is_none()
                                .then(|| "MCP request produced no response".to_string())
                        });
                    capsem_core::try_send!(
                        "ipc_mcp_call_tool",
                        ipc_tx_out
                            .send(ProcessToService::McpCallToolResult {
                                id,
                                result_json,
                                event_id,
                                security_rule_events_json,
                                error
                            })
                            .await
                    );
                });
            }
            ServiceToProcess::PrepareSnapshot
            | ServiceToProcess::Unfreeze
            | ServiceToProcess::Resume => {
                // These are sent directly by process internals (quiescence helper),
                // not expected over IPC from service.
                warn!("unexpected lifecycle IPC command received");
            }
        }
    }
    // Connection ended: cancel any in-flight stream task. Without this the
    // task lives on the runtime, holds its `out_tx`, and may attempt one
    // more send after the client has already closed the IPC socket --
    // benign for the underlying mpsc but a leak (the receiver's drop
    // chain finishes one tick later than necessary).
    if let Some(h) = stream_task.take() {
        h.abort();
    }
    Ok(())
}

/// Maps an IPC ServiceToProcess message to the action category it triggers.
/// Used for dispatch validation and testing.
#[cfg(test)]
fn classify_ipc_message(msg: &ServiceToProcess) -> IpcAction {
    match msg {
        ServiceToProcess::StartTerminalStream => IpcAction::StreamSetup,
        ServiceToProcess::StopTerminalStream => IpcAction::StreamSetup,
        ServiceToProcess::Ping => IpcAction::HealthCheck,
        ServiceToProcess::TerminalInput { .. } => IpcAction::Forward,
        ServiceToProcess::TerminalResize { .. } => IpcAction::Forward,
        ServiceToProcess::Exec { .. } => IpcAction::Job,
        ServiceToProcess::WriteFile { .. } => IpcAction::Job,
        ServiceToProcess::ReadFile { .. } => IpcAction::Job,
        ServiceToProcess::LogFileBoundary { .. } => IpcAction::Job,
        ServiceToProcess::ReloadConfig => IpcAction::Reload,
        ServiceToProcess::Shutdown => IpcAction::Lifecycle,
        ServiceToProcess::Suspend { .. } => IpcAction::Lifecycle,
        ServiceToProcess::PrepareSnapshot
        | ServiceToProcess::Unfreeze
        | ServiceToProcess::Resume => IpcAction::Unexpected,
        ServiceToProcess::McpListServers { .. }
        | ServiceToProcess::McpListTools { .. }
        | ServiceToProcess::McpRefreshTools { .. }
        | ServiceToProcess::McpCallTool { .. }
        | ServiceToProcess::SnapshotStatus { .. } => IpcAction::Job,
    }
}

fn snapshot_status_from_scheduler(
    scheduler: &capsem_core::auto_snapshot::AutoSnapshotScheduler,
) -> capsem_proto::ipc::SnapshotStatus {
    let snapshots = scheduler.list_snapshots();
    let auto_count = snapshots
        .iter()
        .filter(|slot| slot.origin == capsem_core::auto_snapshot::SnapshotOrigin::Auto)
        .count();
    let manual_count = snapshots.len().saturating_sub(auto_count);
    let snapshots = snapshots
        .into_iter()
        .map(|slot| capsem_proto::ipc::SnapshotSlotStatus {
            checkpoint: format!("cp-{}", slot.slot),
            slot: slot.slot,
            origin: match slot.origin {
                capsem_core::auto_snapshot::SnapshotOrigin::Auto => "auto",
                capsem_core::auto_snapshot::SnapshotOrigin::Manual => "manual",
            }
            .to_string(),
            name: slot.name,
            timestamp: snapshot_timestamp(slot.timestamp),
            hash: slot.hash,
        })
        .collect();

    capsem_proto::ipc::SnapshotStatus {
        total: auto_count + manual_count,
        auto_count,
        manual_count,
        manual_available: scheduler.available_manual_slots(),
        snapshots,
    }
}

fn snapshot_timestamp(timestamp: std::time::SystemTime) -> String {
    let secs = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{secs}")
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
enum IpcAction {
    StreamSetup,
    HealthCheck,
    Forward,
    Job,
    Reload,
    Lifecycle,
    Unexpected,
}

#[cfg(test)]
mod tests;
