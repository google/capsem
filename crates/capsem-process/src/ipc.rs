use anyhow::Result;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use capsem_proto::metrics::VmMetricsSnapshot;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_unix_ipc::{channel_from_std, Receiver, Sender};
use tracing::{debug, error, info, warn};

use crate::job_store::{JobResult, JobStore};
use crate::mcp_runtime::McpRuntime;
use crate::terminal::TerminalRelay;

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
    mcp_runtime: Arc<McpRuntime>,
    vm_ready: Arc<AtomicBool>,
    vm_id: String,
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
            ServiceToProcess::GetMetricsSnapshot { id } => {
                let snapshot = default_metrics_snapshot(&vm_id);
                capsem_core::try_send!(
                    "ipc_metrics_snapshot",
                    ipc_tx_out
                        .send(ProcessToService::MetricsSnapshot {
                            id,
                            snapshot: Box::new(snapshot),
                        })
                        .await
                );
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
            ServiceToProcess::ReloadConfig { runtime_rules } => {
                info!("Reloading policies from disk");
                let runtime_state =
                    crate::mcp_runtime::load_runtime_policy_state_with_runtime_rules(
                        &mcp_runtime.session_dir,
                        runtime_rules.as_ref(),
                    );
                let servers = crate::mcp_runtime::build_servers_with_builtin(
                    &runtime_state.mcp_user,
                    &runtime_state.mcp_corp,
                    mcp_runtime.builtin_binary.as_deref(),
                    &mcp_runtime.session_dir,
                    &runtime_state.domain_policy,
                );

                let new_domain = Arc::new(runtime_state.domain_policy);
                let new_mcp = Arc::new(runtime_state.mcp_policy);
                *mcp_runtime.domain_policy.write().unwrap() = Arc::clone(&new_domain);
                *mcp_runtime.policy.write().await = new_mcp;
                mcp_runtime
                    .security_engine
                    .set(runtime_state.security_engine);

                let reload_result = mcp_runtime.aggregator.refresh(servers).await;
                let (success, error) = match reload_result {
                    Ok(()) => (true, None),
                    Err(e) => (false, Some(e.to_string())),
                };
                capsem_core::try_send!(
                    "ipc_reload_config_result",
                    ipc_tx_out
                        .send(ProcessToService::ReloadConfigResult { success, error })
                        .await
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
        ServiceToProcess::ReloadConfig { .. } => IpcAction::Reload,
        ServiceToProcess::GetMetricsSnapshot { .. } => IpcAction::HealthCheck,
        ServiceToProcess::Shutdown => IpcAction::Lifecycle,
        ServiceToProcess::Suspend { .. } => IpcAction::Lifecycle,
        ServiceToProcess::PrepareSnapshot
        | ServiceToProcess::Unfreeze
        | ServiceToProcess::Resume => IpcAction::Unexpected,
    }
}

fn default_metrics_snapshot(vm_id: &str) -> VmMetricsSnapshot {
    let captured_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    VmMetricsSnapshot::empty(vm_id, false, captured_at_unix_ms)
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
