use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
use tracing::{info, error, warn, debug};

use crate::job_store::{JobStore, JobResult};
use crate::terminal::TerminalRelay;

pub(crate) async fn handle_ipc_connection(
    stream: tokio::net::UnixStream,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    ipc_tx: broadcast::Sender<ProcessToService>,
    term_relay: Arc<TerminalRelay>,
    job_store: Arc<JobStore>,
    net_state: Arc<capsem_core::SandboxNetworkState>,
    mcp_config: Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    vm_ready: Arc<AtomicBool>,
) -> Result<()> {
    let std_stream = stream.into_std()?;
    let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) = channel_from_std(std_stream)?;

    // Serialize all IPC writes through a single channel to prevent concurrent
    // sendmsg() interleaving that corrupts the data stream. tokio_unix_ipc's
    // Sender::send() writes header + payload as two separate syscalls with no
    // internal locking, so concurrent use from multiple tasks is unsafe.
    let (ipc_tx_out, mut ipc_rx_out) = mpsc::channel::<ProcessToService>(256);
    tokio::spawn(async move {
        while let Some(msg) = ipc_rx_out.recv().await {
            if tx.send(msg).await.is_err() { break; }
        }
    });

    while let Ok(msg) = rx.recv().await {
        match msg {
            ServiceToProcess::StartTerminalStream => {
                    info!("Starting terminal stream for connection");
                    let out_tx = ipc_tx_out.clone();
                    // Atomic snapshot + subscribe so the client sees buffered
                    // banner bytes and then a gap-free live stream.
                    let (replay, mut term_rx) = term_relay.subscribe();
                    tokio::spawn(async move {
                        if !replay.is_empty()
                            && out_tx.send(ProcessToService::TerminalOutput { data: replay }).await.is_err()
                        {
                            return;
                        }
                        while let Ok(data) = term_rx.recv().await {
                            if out_tx.send(ProcessToService::TerminalOutput { data }).await.is_err() { break; }
                        }
                    });

                    let out_tx2 = ipc_tx_out.clone();
                    let mut rx_c = ipc_tx.subscribe();
                    tokio::spawn(async move {
                        while let Ok(msg) = rx_c.recv().await {
                            if out_tx2.send(msg).await.is_err() { break; }
                        }
                    });
                }
                ServiceToProcess::Ping => {
                    if vm_ready.load(Ordering::Acquire) {
                        let _ = ipc_tx_out.send(ProcessToService::Pong).await;
                    } else {
                        debug!("Ping received but VM not ready, closing connection");
                        return Ok(());
                    }
                }
                ServiceToProcess::TerminalInput { data } => { let _ = ctrl_tx.send(ServiceToProcess::TerminalInput { data }).await; }
                ServiceToProcess::TerminalResize { cols, rows } => { let _ = ctrl_tx.send(ServiceToProcess::TerminalResize { cols, rows }).await; }
                ServiceToProcess::Exec { id, command } => {
                    let job_store = job_store.clone();
                    let ctrl_tx = ctrl_tx.clone();
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        info!(id, command, "Received Exec command via IPC");
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);

                        // Set as active exec to start capturing output
                        *job_store.active_exec.lock().unwrap() = Some((id, Vec::new()));

                        let _ = ctrl_tx.send(ServiceToProcess::Exec { id, command }).await;
                        match j_rx.await {
                            Ok(JobResult::Exec { stdout, stderr, exit_code }) => {
                                info!(id, exit_code, "Sending ExecResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ExecResult { id, stdout, stderr, exit_code }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending Exec error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ExecResult { id, stdout: vec![], stderr: message.into_bytes(), exit_code: -1 }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for Exec");
                            }
                        }
                    });
                }
                ServiceToProcess::WriteFile { id, path, data } => {
                    let job_store = job_store.clone();
                    let ctrl_tx = ctrl_tx.clone();
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        info!(id, path, len = data.len(), "Received WriteFile command via IPC");
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);
                        let _ = ctrl_tx.send(ServiceToProcess::WriteFile { id, path, data }).await;
                        match j_rx.await {
                            Ok(JobResult::WriteFile { success, error }) => {
                                info!(id, success, "Sending WriteFileResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::WriteFileResult { id, success, error }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending WriteFile error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::WriteFileResult { id, success: false, error: Some(message) }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for WriteFile");
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
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);
                        let _ = ctrl_tx.send(ServiceToProcess::ReadFile { id, path }).await;
                        match j_rx.await {
                            Ok(JobResult::ReadFile { data, error }) => {
                                info!(id, success = data.is_some(), "Sending ReadFileResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ReadFileResult { id, data, error }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending ReadFile error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ReadFileResult { id, data: None, error: Some(message) }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for ReadFile");
                            }
                        }
                    });
                }
                ServiceToProcess::ReloadConfig => {
                    info!("Reloading policies from disk");
                    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();

                    let new_domain = Arc::new(capsem_core::net::policy_config::settings_to_domain_policy(&capsem_core::net::policy_config::resolve_settings(&user_sf, &corp_sf)));
                    let new_network = Arc::new(capsem_core::net::policy_config::build_network_policy(&capsem_core::net::policy_config::resolve_settings(&user_sf, &corp_sf)));

                    let user_mcp = user_sf.mcp.clone().unwrap_or_default();
                    let corp_mcp = corp_sf.mcp.clone().unwrap_or_default();
                    let new_mcp = Arc::new(user_mcp.to_policy(&corp_mcp));

                    *net_state.policy.write().unwrap() = new_network;
                    *mcp_config.domain_policy.write().unwrap() = Arc::clone(&new_domain);
                    *mcp_config.policy.write().await = new_mcp;

                    let _ = ipc_tx_out.send(ProcessToService::Pong).await;
                }
                ServiceToProcess::Shutdown => {
                    let _ = ctrl_tx.send(ServiceToProcess::Shutdown).await;
                    info!("Received Shutdown command, exiting IPC loop gracefully");
                    break;
                }
                ServiceToProcess::Suspend { checkpoint_path } => {
                    info!("Received Suspend command, forwarding to ctrl channel");
                    let _ = ctrl_tx.send(ServiceToProcess::Suspend { checkpoint_path }).await;
                }
                ServiceToProcess::McpListServers { id } => {
                    let mcp = Arc::clone(&mcp_config);
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        match mcp.aggregator.list_servers().await {
                            Ok(agg_servers) => {
                                let servers = agg_servers.into_iter().map(|s| {
                                    capsem_proto::ipc::McpServerStatus {
                                        name: s.name,
                                        url: s.url,
                                        enabled: s.enabled,
                                        source: s.source,
                                        is_stdio: s.is_stdio,
                                        connected: s.connected,
                                        tool_count: s.tool_count,
                                    }
                                }).collect();
                                let _ = ipc_tx_out.send(ProcessToService::McpServersResult { id, servers }).await;
                            }
                            Err(e) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpServersResult { id, servers: vec![] }).await;
                                warn!(error = %e, "failed to list MCP servers");
                            }
                        }
                    });
                }
                ServiceToProcess::McpListTools { id } => {
                    let mcp = Arc::clone(&mcp_config);
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        match mcp.aggregator.list_tools().await {
                            Ok(tools) => {
                                let tools = tools.into_iter().map(|t| {
                                    capsem_proto::ipc::McpToolStatus {
                                        namespaced_name: t.namespaced_name,
                                        original_name: t.original_name,
                                        description: t.description,
                                        server_name: t.server_name,
                                        annotations: t.annotations.as_ref().map(|a| a.to_mcp_json()),
                                    }
                                }).collect();
                                let _ = ipc_tx_out.send(ProcessToService::McpToolsResult { id, tools }).await;
                            }
                            Err(e) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpToolsResult { id, tools: vec![] }).await;
                                warn!(error = %e, "failed to list MCP tools");
                            }
                        }
                    });
                }
                ServiceToProcess::McpRefreshTools { id } => {
                    let mcp = Arc::clone(&mcp_config);
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        // Reload config from disk and refresh aggregator.
                        let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();
                        let servers = capsem_core::mcp::build_server_list(
                            &user_sf.mcp.clone().unwrap_or_default(),
                            &corp_sf.mcp.clone().unwrap_or_default(),
                        );
                        match mcp.aggregator.refresh(servers).await {
                            Ok(()) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpRefreshResult { id, success: true, error: None }).await;
                            }
                            Err(e) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpRefreshResult { id, success: false, error: Some(e.to_string()) }).await;
                            }
                        }
                    });
                }
                ServiceToProcess::McpCallTool { id, namespaced_name, arguments } => {
                    let mcp = Arc::clone(&mcp_config);
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        match mcp.aggregator.call_tool(&namespaced_name, arguments).await {
                            Ok(result) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpCallToolResult { id, result: Some(result), error: None }).await;
                            }
                            Err(e) => {
                                let _ = ipc_tx_out.send(ProcessToService::McpCallToolResult { id, result: None, error: Some(e.to_string()) }).await;
                            }
                        }
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
    Ok(())
}

/// Maps an IPC ServiceToProcess message to the action category it triggers.
/// Used for dispatch validation and testing.
#[cfg(test)]
fn classify_ipc_message(msg: &ServiceToProcess) -> IpcAction {
    match msg {
        ServiceToProcess::StartTerminalStream => IpcAction::StreamSetup,
        ServiceToProcess::Ping => IpcAction::HealthCheck,
        ServiceToProcess::TerminalInput { .. } => IpcAction::Forward,
        ServiceToProcess::TerminalResize { .. } => IpcAction::Forward,
        ServiceToProcess::Exec { .. } => IpcAction::Job,
        ServiceToProcess::WriteFile { .. } => IpcAction::Job,
        ServiceToProcess::ReadFile { .. } => IpcAction::Job,
        ServiceToProcess::ReloadConfig => IpcAction::Reload,
        ServiceToProcess::Shutdown => IpcAction::Lifecycle,
        ServiceToProcess::Suspend { .. } => IpcAction::Lifecycle,
        ServiceToProcess::PrepareSnapshot
        | ServiceToProcess::Unfreeze
        | ServiceToProcess::Resume => IpcAction::Unexpected,
        ServiceToProcess::McpListServers { .. }
        | ServiceToProcess::McpListTools { .. }
        | ServiceToProcess::McpRefreshTools { .. }
        | ServiceToProcess::McpCallTool { .. } => IpcAction::Job,
    }
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
mod tests {
    use super::*;

    #[test]
    fn classify_ping() {
        assert_eq!(classify_ipc_message(&ServiceToProcess::Ping), IpcAction::HealthCheck);
    }

    #[test]
    fn classify_terminal_input() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::TerminalInput { data: vec![0x41] }),
            IpcAction::Forward
        );
    }

    #[test]
    fn classify_terminal_resize() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::TerminalResize { cols: 80, rows: 24 }),
            IpcAction::Forward
        );
    }

    #[test]
    fn classify_exec() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::Exec { id: 1, command: "ls".into() }),
            IpcAction::Job
        );
    }

    #[test]
    fn classify_write_file() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::WriteFile { id: 1, path: "/tmp/f".into(), data: vec![] }),
            IpcAction::Job
        );
    }

    #[test]
    fn classify_read_file() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::ReadFile { id: 1, path: "/tmp/f".into() }),
            IpcAction::Job
        );
    }

    #[test]
    fn classify_reload_config() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::ReloadConfig),
            IpcAction::Reload
        );
    }

    #[test]
    fn classify_shutdown() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::Shutdown),
            IpcAction::Lifecycle
        );
    }

    #[test]
    fn classify_suspend() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::Suspend { checkpoint_path: "cp.vzsave".into() }),
            IpcAction::Lifecycle
        );
    }

    #[test]
    fn classify_start_terminal_stream() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::StartTerminalStream),
            IpcAction::StreamSetup
        );
    }

    #[test]
    fn classify_prepare_snapshot_unexpected() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::PrepareSnapshot),
            IpcAction::Unexpected
        );
    }

    #[test]
    fn classify_unfreeze_unexpected() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::Unfreeze),
            IpcAction::Unexpected
        );
    }

    #[test]
    fn classify_resume_unexpected() {
        assert_eq!(
            classify_ipc_message(&ServiceToProcess::Resume),
            IpcAction::Unexpected
        );
    }
}
