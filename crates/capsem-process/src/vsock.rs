use anyhow::{Context, Result};
use capsem_core::{read_control_msg, write_control_msg, VsockConnection};
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
#[cfg(test)]
use capsem_proto::HostVsockService;
use capsem_proto::{self as proto, GuestToHost, HostToGuest};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, trace, warn};

use crate::helpers::clone_fd;
use crate::job_store::{ActiveFileOp, JobResult, JobStore};

mod audit;
mod aux;
#[cfg(test)]
use audit::serve_audit_records;
use aux::{dispatch_aux_connection, AuxConnectionLimits, MAX_DEFERRED_AUX_CONNECTIONS};
mod clone_state;
#[cfg(test)]
use audit::{handle_audit_frame, read_bounded_frame};
mod handshake;
mod streams;
mod suspend;
mod terminal;
use handshake::{collect_terminal_control_pair, is_retryable_handshake_error, perform_handshake_bounded};
pub(crate) use suspend::Files as SuspendFiles;
pub(crate) use terminal::TerminalBridge;
mod guest_report;
use guest_report::{ackable_id, ackable_response_id, is_guest_liveness_message, warn_unhandled_guest_msg};
mod exec_completion;
mod exec_dispatch;
mod exec_input;
mod exec_output;
mod file_content;
mod shutdown;

use crate::ipc::file_boundary::{file_content_preview, PluginPolicyHandle, SecurityRulesHandle};
#[cfg(test)]
use crate::ipc::file_boundary::{rewritten_file_content, FILE_SECURITY_CONTENT_PREVIEW_MAX};

/// Maximum attempts for the initial handshake before giving up.
///
/// Apple VZ occasionally hands us a half-open vsock control fd on the
/// first accept after `restore_state` (~4% of resume cycles). The guest
/// detects the broken pipe on its side and its outer reconnect loop
/// produces a fresh terminal+control pair within ~100-500ms. This cap
/// absorbs that tail while still failing fast on a genuinely broken
/// guest. Post-initial handshakes (on re-keyed connections) do not
/// retry: the guest drives retry at the transport layer.
const HANDSHAKE_RETRY_MAX: usize = 3;

/// Bound for a control-channel `ExecDone` that arrives before the dedicated
/// EXEC-port reader deposits the command's captured bytes.
///
/// The guest closes the EXEC socket before queuing `ExecDone`, so a healthy
/// transport signals the notifier immediately once the reader is scheduled.
/// Loaded qualification runners can delay that reader well beyond 100 ms,
/// especially just after resume. Keep a generous transport-loss bound without
/// imposing any delay on already-deposited or genuinely empty commands.
const EXEC_OUTPUT_DEPOSIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) struct VsockOptions {
    pub(crate) vm_id: String,
    pub(crate) vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    pub(crate) vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    pub(crate) ipc_tx: broadcast::Sender<ProcessToService>,
    pub(crate) _ctrl_tx: mpsc::Sender<ServiceToProcess>,
    pub(crate) ctrl_rx: mpsc::Receiver<ServiceToProcess>,
    pub(crate) terminal_output: Arc<capsem_core::terminal::SessionTerminal>,
    pub(crate) session_browser: Arc<crate::browser::SessionBrowser>,
    pub(crate) job_store: Arc<JobStore>,
    pub(crate) cli_env: Vec<(String, String)>,
    pub(crate) guest_config: capsem_core::net::policy_config::GuestConfig,
    pub(crate) upstream_grants: Arc<capsem_core::net::upstream_grant::UpstreamGrantClient>,
    pub(crate) security_rules: SecurityRulesHandle,
    pub(crate) plugin_policy: PluginPolicyHandle,
    pub(crate) approvals: Option<Arc<capsem_core::security_engine::approval::client::ApprovalCapability>>,
    pub(crate) _net_state: Arc<capsem_core::SandboxNetworkState>,
    pub(crate) is_restore: bool,
    pub(crate) vm_ready: Arc<AtomicBool>,
    pub(crate) ready: std::fs::File,
    pub(crate) db: Arc<capsem_logger::DbWriter>,
    pub(crate) pty_log: Option<capsem_core::pty_log::PtyRecorder>,
    pub(crate) serial_log: capsem_foundation::telemetry::SharedLogWriter,
    pub(crate) checkpoint: Arc<SuspendFiles>,
    pub(crate) shutdown: Arc<tokio::sync::Mutex<crate::Shutdown>>,
}

pub(crate) async fn setup_vsock(options: VsockOptions) -> Result<()> {
    let vm_id_original = options.vm_id.clone();
    let vm_handle_original = options.vm.clone();

    let VsockOptions {
        vm_id,
        mut vsock_rx,
        ipc_tx,
        ctrl_rx,
        terminal_output,
        session_browser,
        job_store,
        cli_env,
        guest_config,
        upstream_grants,
        security_rules,
        plugin_policy,
        approvals,
        is_restore,
        vm_ready,
        mut ready,
        db,
        pty_log,
        serial_log,
        checkpoint,
        shutdown,
        ..
    } = options;

    // Stable channels to re-key the bridges when connections reset
    let (control_rekey_tx, control_rekey_rx) = mpsc::channel::<Arc<VsockConnection>>(4);
    let (terminal_rekey_tx, terminal_rekey_rx) = mpsc::channel::<Arc<VsockConnection>>(4);

    // Channel for stable terminal input across reconnections
    let (term_in_tx, term_in_rx) = mpsc::channel::<Vec<u8>>(128);

    let mut deferred_conns: Vec<VsockConnection> = Vec::new();
    let (initial_t, initial_c) = {
        let mut attempt = 1usize;
        loop {
            let mut attempt_deferred: Vec<VsockConnection> = Vec::new();
            let (terminal_conn, control_conn) =
                collect_terminal_control_pair(&mut vsock_rx, &mut attempt_deferred).await?;

            let initial_ctrl = control_conn
                .try_clone_file()
                .context("duplicate initial control vsock for handshake")?;
            let is_rest = is_restore;
            let cli_env_clone = cli_env.clone();
            let guest_config_clone = guest_config.clone();

            let handshake_res = perform_handshake_bounded(
                initial_ctrl,
                is_rest,
                cli_env_clone,
                Some(guest_config_clone),
                std::time::Duration::from_secs(60),
            )
            .await;

            match handshake_res {
                Ok(()) => {
                    deferred_conns.extend(attempt_deferred);
                    break (Arc::new(terminal_conn), Arc::new(control_conn));
                }
                Err(e) if attempt < HANDSHAKE_RETRY_MAX && is_retryable_handshake_error(&e) => {
                    warn!(
                        attempt,
                        error = format!("{e:#}"),
                        "initial handshake failed (retryable), dropping fds and awaiting guest reconnect"
                    );
                    drop(terminal_conn);
                    drop(control_conn);
                    drop(attempt_deferred);
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(e.context("initial handshake failed")),
            }
        }
    };

    // Send the initial FDs into the rekey channels to prime the bridges
    capsem_core::try_send!("terminal_rekey", terminal_rekey_tx.send(initial_t.clone()).await);
    capsem_core::try_send!("control_rekey", control_rekey_tx.send(initial_c.clone()).await);

    info!(
        category = "boot_timeline",
        from = "Handshaking",
        to = "Running",
        trigger = "booted",
        "state transition"
    );
    capsem_core::try_send!(
        "ipc_state_change",
        ipc_tx.send(ProcessToService::StateChanged {
            id: vm_id.clone(),
            state: "Running".into(),
            trigger: "booted".into()
        })
    );
    vm_ready.store(true, Ordering::Release);
    let ready_for_reader = ready.try_clone().context("clone ready sentinel")?;
    if let Err(e) = ready.write_all(b"ready\n").and_then(|()| ready.sync_data()) {
        warn!(error = %e, "failed to create ready sentinel");
    }

    let terminal_bridge = terminal::spawn(
        Arc::clone(&terminal_output),
        terminal_rekey_rx,
        term_in_rx,
        pty_log.clone(),
        serial_log,
    );
    shutdown.lock().await.terminal_bridge = Some(terminal_bridge);

    // 2. Stable Control Bridge (Read + Write)
    let (ctrl_out_tx, mut ctrl_out_rx) = mpsc::channel::<HostToGuest>(128);
    let js = Arc::clone(&job_store);
    let db_ctrl = Arc::clone(&db);
    let security_rules_ctrl = Arc::clone(&security_rules);
    let plugin_policy_ctrl = Arc::clone(&plugin_policy);
    let mut control_rekey_rx_inner = control_rekey_rx;

    let js_for_teardown = Arc::clone(&job_store);
    let vm_ready_for_reader = Arc::clone(&vm_ready);
    let ready_for_reader = ready_for_reader;

    // Pending-ack map lives on `JobStore` (see job_store.rs::pending_acks)
    // and the bridge end here can replay-on-rekey. See the field doc on
    // `JobStore::pending_acks` for the full reasoning.
    let pending_for_bridge = Arc::clone(&job_store);

    tokio::spawn(async move {
        let pending = pending_for_bridge;
        let mut current_conn = control_rekey_rx_inner.recv().await;
        loop {
            let conn = match current_conn.take() {
                Some(c) => c,
                None => match control_rekey_rx_inner.recv().await {
                    Some(c) => c,
                    None => {
                        js_for_teardown.fail_all("control channel closed");
                        vm_ready_for_reader.store(false, Ordering::Release);
                        let _ = ready_for_reader.set_len(0).and_then(|()| ready_for_reader.sync_data());
                        break;
                    }
                },
            };

            info!("control bridge: active");
            let mut control = match capsem_core::vm::control::Connection::new(conn) {
                Ok(control) => control,
                Err(error) => {
                    error!(%error, "control bridge: connection setup failed");
                    continue;
                }
            };

            // Re-write every pending (unacked) message on the fresh conn.
            // Snapshot under the lock so a concurrent insert during replay
            // doesn't double-replay the same message.
            let to_replay: Vec<HostToGuest> = pending.pending_acks.lock().unwrap().values().cloned().collect();
            if !to_replay.is_empty() {
                info!(
                    count = to_replay.len(),
                    "control bridge: replaying pending unacked messages"
                );
                let mut replay_failed = false;
                for msg in &to_replay {
                    let frame = match proto::encode_host_msg(msg) {
                        Ok(frame) => frame,
                        Err(error) => {
                            error!(%error, "control bridge: invalid replay frame");
                            replay_failed = true;
                            break;
                        }
                    };
                    if let Err(e) = control.write(&frame).await {
                        error!(error = %e, "control bridge: replay write failed");
                        replay_failed = true;
                        break;
                    }
                }
                if replay_failed {
                    control.close().await;
                    continue;
                }
            }

            if let Err(error) = js.publisher.control_ready() {
                error!(%error, "control bridge: publication lease refused");
                control.close().await;
                continue;
            }

            loop {
                tokio::select! {
                    // Outgoing messages -- record ackable ones in `pending`
                    // *before* writing so a write-fail/silent-drop is
                    // recoverable via the next rekey replay.
                    Some(msg) = ctrl_out_rx.recv() => {
                        if let HostToGuest::ConnectPort { flow, .. } = &msg {
                            if !js.publisher.pending_connection(*flow) {
                                tracing::debug!(?flow, "control bridge: stale publication setup dropped");
                                continue;
                            }
                        }
                        // Frame first: a message the guest would drop must
                        // not be parked for replay or mistaken for a
                        // transport failure.
                        let frame = match proto::encode_host_msg(&msg) {
                            Ok(frame) => frame,
                            Err(e) => {
                                error!(error = %e, "control bridge: unframeable message dropped");
                                continue;
                            }
                        };
                        if let Some(id) = ackable_id(&msg) {
                            pending.pending_acks.lock().unwrap().insert(id, msg.clone());
                        }
                        if let Err(e) = control.write(&frame).await {
                            error!(error = %e, "control bridge: write failed");
                            break;
                        }
                    }
                    // Incoming messages -- intercept Ack here, dispatch
                    // everything else to handle_guest_msg. Ackable
                    // responses (ExecDone / FileOpDone / FileContent /
                    // Error{id}) get an immediate `AckReply` written
                    // back so the agent's symmetric pending_responses
                    // map can drop the entry; without this the agent
                    // would replay every response on every rekey.
                    res = control.recv() => {
                        match res {
                            Some(Ok(GuestToHost::Ack { id })) => {
                                pending.pending_acks.lock().unwrap().remove(&id);
                            }
                            Some(Ok(GuestToHost::PortClosed { flow, report })) => {
                                // One flow's bookkeeping never ends the control link every other flow rides on.
                                if let Err(error) = js.publisher.report_close(flow, report) {
                                    error!(%error, flow = flow.id, "guest close report rejected");
                                }
                                let frame = proto::encode_host_msg(&HostToGuest::PortCloseAck { flow })
                                    .expect("fixed-size flow acknowledgement");
                                if let Err(error) = control.write(&frame).await {
                                    error!(%error, "flow close acknowledgement failed");
                                    break;
                                }
                            }
                            Some(Ok(msg)) => {
                                if let GuestToHost::OpenUrl { url } = &msg {
                                    // Never log a login challenge or turn guest
                                    // browser requests into terminal output.
                                    if session_browser.request(&url.0).is_err() {
                                        warn!("guest browser request rejected");
                                    }
                                    continue;
                                }
                                if let Some(id) = ackable_response_id(&msg) {
                                    let frame = proto::encode_host_msg(&HostToGuest::AckReply { id })
                                        .expect("fixed-size control acknowledgement");
                                    if let Err(e) = control.write(&frame).await {
                                        error!(error = %e, "control bridge: AckReply write failed");
                                        break;
                                    }
                                }
                                handle_guest_msg(
                                    msg,
                                    &js,
                                    &db_ctrl,
                                    &security_rules_ctrl,
                                    &plugin_policy_ctrl,
                                )
                                .await
                            }
                            Some(Err(error)) => {
                                warn!(%error, "control bridge: guest frame failed; waiting for rekey");
                                break;
                            }
                            None => {
                                info!("control bridge: guest closed the control stream; waiting for rekey");
                                break;
                            }
                        }
                    }
                    // Connection reset
                    Some(new_conn) = control_rekey_rx_inner.recv() => {
                        current_conn = Some(new_conn);
                        break;
                    }
                }
            }
            js.publisher.control_lost();
            control.close().await;
        }
    });

    // Heartbeat
    let hb_tx = ctrl_out_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let epoch_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if hb_tx.send(HostToGuest::Ping { epoch_secs }).await.is_err() {
                break;
            }
        }
    });

    // 3. Command Multiplexer (IPC -> Hub)
    let hub_tx = ctrl_out_tx;
    let js_for_cmd = Arc::clone(&job_store);
    let ipc_tx_for_cmd = ipc_tx.clone();
    let vm_id_for_cmd = vm_id_original;
    let vm_handle_for_cmd = vm_handle_original;
    let pty_log_for_cmd = pty_log.clone();
    let shutdown_for_cmd = Arc::clone(&shutdown);
    let exec_dispatch = exec_dispatch::ExecDispatch {
        vm_id: vm_id_for_cmd.clone(),
        db: Arc::clone(&db),
        rules: Arc::clone(&security_rules),
        plugins: Arc::clone(&plugin_policy),
        jobs: Arc::clone(&job_store),
        hub: hub_tx.clone(),
        approvals,
    };
    let mut ctrl_rx = ctrl_rx;

    tokio::spawn(async move {
        let mut exec_calls = tokio::task::JoinSet::new();
        while let Some(msg) = ctrl_rx.recv().await {
            while exec_calls.try_join_next().is_some() {}
            match msg {
                ServiceToProcess::TerminalInput { data } => {
                    if let Some(ref pl) = pty_log_for_cmd {
                        // The optional recorder logs its first failure once.
                        // Never refuse terminal input because storage failed.
                        let _ = pl.record_input(&data);
                    }
                    capsem_core::try_send!("term_in", term_in_tx.send(data).await);
                }
                ServiceToProcess::TerminalResize { cols, rows } => {
                    capsem_core::try_send!("hub_resize", hub_tx.send(HostToGuest::Resize { cols, rows }).await);
                }
                ServiceToProcess::ConnectPort { flow, port, target } => {
                    capsem_core::try_send!(
                        "hub_publication",
                        hub_tx.send(HostToGuest::ConnectPort { flow, port, target }).await
                    );
                }
                ServiceToProcess::AbortPorts { flows } => {
                    capsem_core::try_send!(
                        "hub_publication_abort",
                        hub_tx.send(HostToGuest::AbortPorts { flows }).await
                    );
                }
                ServiceToProcess::PlugCable { cable, address, prefix } => {
                    capsem_core::try_send!(
                        "hub_cable",
                        hub_tx.send(HostToGuest::PlugCable { cable, address, prefix }).await
                    );
                }
                ServiceToProcess::UnplugCable { cable } => {
                    capsem_core::try_send!("hub_cable", hub_tx.send(HostToGuest::UnplugCable { cable }).await);
                }
                ServiceToProcess::Exec { id, command, target } => {
                    exec_dispatch.spawn(&mut exec_calls, id, command, target)
                }
                ServiceToProcess::CancelExec { id } => {
                    js_for_cmd.cancel_exec(id);
                    let cancellation_id = js_for_cmd
                        .next_control_id
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    capsem_core::try_send!(
                        "hub_cancel_exec",
                        hub_tx.send(HostToGuest::CancelExec { id, cancellation_id }).await
                    );
                }
                ServiceToProcess::WriteFile { id, path, data } => {
                    js_for_cmd.active_file_ops.lock().unwrap().insert(
                        id,
                        ActiveFileOp::Write {
                            path: path.clone(),
                            data: data.clone(),
                        },
                    );
                    capsem_core::try_send!(
                        "hub_file_write",
                        hub_tx
                            .send(HostToGuest::FileWrite {
                                id,
                                path,
                                data,
                                mode: 0o644
                            })
                            .await
                    );
                }
                ServiceToProcess::ReadFile { id, path } => {
                    js_for_cmd
                        .active_file_ops
                        .lock()
                        .unwrap()
                        .insert(id, ActiveFileOp::Read { path: path.clone() });
                    capsem_core::try_send!("hub_file_read", hub_tx.send(HostToGuest::FileRead { id, path }).await);
                }
                ServiceToProcess::CloneState { id } => {
                    clone_state::spawn(&hub_tx, &js_for_cmd, &ipc_tx_for_cmd, id);
                }
                ServiceToProcess::Suspend => {
                    js_for_cmd.cancel_execs();
                    suspend::Suspend {
                        vm: Arc::clone(&vm_handle_for_cmd),
                        hub: hub_tx.clone(),
                        jobs: Arc::clone(&js_for_cmd),
                        ipc: ipc_tx_for_cmd.clone(),
                        id: vm_id_for_cmd.clone(),
                        shutdown: Arc::clone(&shutdown_for_cmd),
                        terminal: Arc::clone(&terminal_output),
                        files: Arc::clone(&checkpoint),
                    }
                    .run()
                    .await;
                }
                ServiceToProcess::Shutdown => {
                    js_for_cmd.cancel_execs();
                    capsem_core::try_send!("hub_shutdown", hub_tx.send(HostToGuest::Shutdown).await);
                    shutdown::spawn_stop_after_guest(
                        Arc::clone(&js_for_cmd),
                        Arc::clone(&vm_handle_for_cmd),
                        Arc::clone(&shutdown_for_cmd),
                    );
                }
                ServiceToProcess::Ping => {
                    let epoch_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    capsem_core::try_send!("hub_ping", hub_tx.send(HostToGuest::Ping { epoch_secs }).await);
                }
                _ => {}
            }
        }
        js_for_cmd.fail_all("guest control command channel closed");
        while exec_calls.join_next().await.is_some() {}
    });

    // -----------------------------------------------------------------------
    // 4. Central Dispatcher Loop (Vsock -> Hub)
    // -----------------------------------------------------------------------
    let upstream_grants_loop = Arc::clone(&upstream_grants);
    let security_rules_loop = Arc::clone(&security_rules);
    let db_for_audit = Arc::clone(&db);
    let ipc_tx_lifecycle = ipc_tx.clone();
    let ctrl_tx_lifecycle = options._ctrl_tx.clone();
    let vm_id_lifecycle = vm_id.clone();
    let job_store_vsock = Arc::clone(&job_store);

    tokio::spawn(async move {
        let aux_limits = AuxConnectionLimits::default();
        let handshake_slot = Arc::new(tokio::sync::Semaphore::new(1));

        // Immediately dispatch the auxiliary connections from the initial collect.
        for conn in deferred_conns {
            dispatch_aux_connection(
                conn,
                &upstream_grants_loop,
                &security_rules_loop,
                &job_store_vsock,
                &db_for_audit,
                &ipc_tx_lifecycle,
                &ctrl_tx_lifecycle,
                &vm_id_lifecycle,
                &aux_limits,
            );
        }

        while let Some(conn) = vsock_rx.recv().await {
            match conn.port {
                proto::VSOCK_PORT_CONTROL => {
                    info!("control port: connection accepted, performing handshake");
                    let Ok(permit) = Arc::clone(&handshake_slot).try_acquire_owned() else {
                        warn!("control port: handshake already in progress; dropping reconnect");
                        continue;
                    };
                    let Some(fd) = clone_fd(&conn, "duplicate-control-vsock-handshake") else {
                        continue;
                    };
                    let cli_env_clone = cli_env.clone();
                    let guest_config_clone = guest_config.clone();
                    let rekey_tx = control_rekey_tx.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        match perform_handshake_bounded(
                            fd,
                            true,
                            cli_env_clone,
                            Some(guest_config_clone),
                            std::time::Duration::from_secs(10),
                        )
                        .await
                        {
                            Ok(()) => {
                                info!("control port: handshake successful, re-keying bridge");
                                capsem_core::try_send!("control_rekey", rekey_tx.send(Arc::new(conn)).await);
                            }
                            Err(e) => error!(error = format!("{e:#}"), "control port: handshake failed"),
                        }
                    });
                }
                proto::VSOCK_PORT_TERMINAL => {
                    info!("terminal port: connection accepted, re-keying bridge");
                    let conn_arc = Arc::new(conn);
                    capsem_core::try_send!("terminal_rekey", terminal_rekey_tx.send(conn_arc).await);
                }
                _ => {
                    dispatch_aux_connection(
                        conn,
                        &upstream_grants_loop,
                        &security_rules_loop,
                        &job_store_vsock,
                        &db_for_audit,
                        &ipc_tx_lifecycle,
                        &ctrl_tx_lifecycle,
                        &vm_id_lifecycle,
                        &aux_limits,
                    );
                }
            }
        }
    });

    Ok(())
}

/// The message to fail an exec job with, or `None` when the command may run.
///
/// A boundary that could not be evaluated refuses too: the exec rail is decided
/// before dispatch, so "we do not know" has to mean "not yet" rather than a
/// command that slipped through while the ledger was broken.
fn exec_boundary_refusal(
    id: u64,
    boundary: &Result<Option<capsem_core::security_engine::SecurityRuleEmission>, String>,
) -> Option<String> {
    match boundary {
        Ok(Some(emission)) if emission.enforcement.is_allowed() => None,
        Ok(Some(emission)) => {
            let rule = emission.enforcement.rule_id.as_deref().unwrap_or("unknown").to_string();
            warn!(
                id,
                rule = rule.as_str(),
                decision = ?emission.enforcement.action,
                "exec refused by security policy"
            );
            Some(emission.enforcement.reason.clone().unwrap_or_else(|| {
                format!(
                    "capsem: command {} by security rule: {rule}",
                    match emission.enforcement.action {
                        capsem_core::security_engine::SecurityEnforcementAction::Ask => "requires approval",
                        _ => "blocked",
                    }
                )
            }))
        }
        Ok(None) => {
            warn!(id, "failed to write exec boundary security event");
            Some("capsem: command refused, security ledger unavailable".to_string())
        }
        Err(error) => {
            warn!(id, error, "failed to evaluate exec boundary");
            Some(format!("capsem: command refused, security evaluation failed: {error}"))
        }
    }
}

async fn handle_guest_msg(
    msg: GuestToHost,
    js: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
    plugin_policy: &PluginPolicyHandle,
) {
    match msg {
        GuestToHost::ShutdownComplete => js.shutdown_complete.notify_one(),
        GuestToHost::BootTiming { stages } => drop(guest_report::record_boot_timing(stages)),
        GuestToHost::ExecDone { id, exit_code } => {
            let streaming = {
                let mut guard = js.active_execs.lock().unwrap();
                let Some(active) = guard.get_mut(&id) else {
                    return;
                };
                if active.completion_started {
                    return;
                }
                active.completion_started = true;
                let streaming = active.stream.is_some();
                drop(guard);
                streaming
            };
            if streaming {
                // At most one completion task per registered job. Slow output
                // consumers must never occupy the control/ack/rekey loop.
                let js = Arc::clone(js);
                let db = Arc::clone(db);
                let rules = Arc::clone(security_rules);
                tokio::spawn(async move {
                    exec_completion::complete(id, exit_code, &js, &db, &rules).await;
                });
            } else {
                exec_completion::complete(id, exit_code, js, db, security_rules).await;
            }
        }
        GuestToHost::FileContent { id, path: _, data } => {
            let context = {
                let mut active_file_ops = js.active_file_ops.lock().unwrap();
                if matches!(active_file_ops.get(&id), Some(ActiveFileOp::Read { .. })) {
                    active_file_ops.remove(&id)
                } else {
                    None
                }
            };
            let Some(context) = context else {
                warn!(id, "unsolicited file content ignored");
                return;
            };
            let js = Arc::clone(js);
            let db = Arc::clone(db);
            let security_rules = Arc::clone(security_rules);
            let plugin_policy = Arc::clone(plugin_policy);
            tokio::spawn(async move {
                file_content::complete(id, context, data, js, db, security_rules, plugin_policy).await;
            });
        }
        GuestToHost::FileOpDone { id } => {
            let context = {
                let mut active_file_ops = js.active_file_ops.lock().unwrap();
                active_file_ops.remove(&id)
            };
            if let Some(context) = context {
                match context {
                    ActiveFileOp::Write { path, data } => {
                        let rules = security_rules.read().unwrap().clone();
                        let plugins = plugin_policy.read().unwrap().clone();
                        if let Err(error) = capsem_core::security_engine::emit_observed_explicit_file_security_write_and_rules_with_plugins(
                            db,
                            &rules,
                            plugins,
                            capsem_core::security_engine::ExplicitFileSecurityEvent {
                                action: capsem_logger::FileAction::Modified,
                                path,
                                size: Some(data.len() as u64),
                                content: Some(file_content_preview(&data)),
                                mime_type: None,
                                trace_id: None,
                                credential_ref: None,
                            },
                        )
                        .await
                        {
                            warn!(id, error, "failed to evaluate file write completion boundary");
                        }
                    }
                    ActiveFileOp::Read { path } => {
                        warn!(
                            id,
                            path, "FileOpDone received for read context; skipping explicit file security event"
                        );
                    }
                }
            } else {
                warn!(
                    id,
                    "FileOpDone arrived without active file context; skipping explicit file security event"
                );
            }
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!(
                    "job_result_write_file",
                    tx.send(JobResult::WriteFile {
                        success: true,
                        error: None
                    })
                );
            }
        }
        GuestToHost::SnapshotTransitioned { id, frozen, error } => js.finish_snapshot_transition(id, frozen, error),
        GuestToHost::Error { id, message } => {
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!("job_result_error", tx.send(JobResult::Error { message }));
            }
        }
        other if is_guest_liveness_message(&other) => {
            trace!(target: "ipc", message = ?other, "guest liveness reply");
        }
        other => {
            warn_unhandled_guest_msg(&other);
        }
    }
}

#[cfg(test)]
mod tests;
