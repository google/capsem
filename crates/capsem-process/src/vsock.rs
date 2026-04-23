use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::{Context, Result};
use capsem_core::{
    VsockConnection,
    read_control_msg, write_control_msg,
};
use capsem_proto::{GuestToHost, HostToGuest};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, error, warn};
use std::io::{Read, Write};

use crate::helpers::clone_fd;
use crate::job_store::{JobStore, JobResult, with_quiescence};

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

pub(crate) struct VsockOptions {
    pub(crate) vm_id: String,
    pub(crate) vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    pub(crate) vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    pub(crate) ipc_tx: broadcast::Sender<ProcessToService>,
    pub(crate) _ctrl_tx: mpsc::Sender<ServiceToProcess>,
    pub(crate) ctrl_rx: mpsc::Receiver<ServiceToProcess>,
    pub(crate) terminal_output: Arc<capsem_core::TerminalOutputQueue>,
    pub(crate) job_store: Arc<JobStore>,
    pub(crate) session_dir: PathBuf,
    pub(crate) cli_env: Vec<(String, String)>,
    pub(crate) guest_config: capsem_core::net::policy_config::GuestConfig,
    pub(crate) mitm_config: Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>,
    pub(crate) mcp_config: Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    pub(crate) _net_state: Arc<capsem_core::SandboxNetworkState>,
    pub(crate) is_restore: bool,
    pub(crate) vm_ready: Arc<AtomicBool>,
    pub(crate) uds_path: PathBuf,
    pub(crate) db: Arc<capsem_logger::DbWriter>,
    pub(crate) pty_log: Option<Arc<crate::pty_log::PtyLog>>,
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
        job_store,
        session_dir,
        cli_env,
        guest_config,
        mitm_config,
        mcp_config,
        is_restore,
        vm_ready,
        uds_path,
        db,
        pty_log,
        ..
    } = options;

    // Stable channels to re-key the bridges when connections reset
    let (control_rekey_tx, control_rekey_rx) = mpsc::channel::<Arc<VsockConnection>>(4);
    let (terminal_rekey_tx, mut terminal_rekey_rx) = mpsc::channel::<Arc<VsockConnection>>(4);
    
    // Channel for stable terminal input across reconnections
    let (term_in_tx, mut term_in_rx) = mpsc::channel::<Vec<u8>>(128);

    let mut deferred_conns: Vec<VsockConnection> = Vec::new();
    let (initial_t, initial_c) = {
        let mut attempt = 1usize;
        loop {
            let mut attempt_deferred: Vec<VsockConnection> = Vec::new();
            let (terminal_conn, control_conn) =
                collect_terminal_control_pair(&mut vsock_rx, &mut attempt_deferred).await?;

            let initial_ctrl_fd = control_conn.fd;
            let is_rest = is_restore;
            let cli_env_clone = cli_env.clone();
            let guest_config_clone = guest_config.clone();

            let handshake_res = tokio::task::spawn_blocking(move || {
                let mut fd = clone_fd(initial_ctrl_fd)?;
                perform_handshake(&mut fd, is_rest, &cli_env_clone, Some(guest_config_clone))
            })
                .await
                .context("handshake task panicked")?;

            match handshake_res {
                Ok(()) => {
                    deferred_conns.extend(attempt_deferred);
                    break (Arc::new(terminal_conn), Arc::new(control_conn));
                }
                Err(e) if attempt < HANDSHAKE_RETRY_MAX && is_retryable_handshake_error(&e) => {
                    warn!(attempt, "initial handshake failed (retryable), dropping fds and awaiting guest reconnect: {e:#}");
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
    let _ = terminal_rekey_tx.send(initial_t.clone()).await;
    let _ = control_rekey_tx.send(initial_c.clone()).await;

    info!(category = "boot_timeline", from = "Handshaking", to = "Running", trigger = "booted", "state transition");
    let _ = ipc_tx.send(ProcessToService::StateChanged {
        id: vm_id.clone(),
        state: "Running".into(),
        trigger: "booted".into()
    });
    vm_ready.store(true, Ordering::Release);
    let ready_path = uds_path.with_extension("ready");
    if let Err(e) = std::fs::File::create(&ready_path) { warn!("failed to create ready sentinel: {e}"); }

    // -----------------------------------------------------------------------
    // 1. Stable Terminal Bridge (Read + Write)
    // -----------------------------------------------------------------------
    let term_out = Arc::clone(&terminal_output);
    let serial_log_path = session_dir.join("serial.log");
    let pty_log_out = pty_log.clone();
    
    tokio::spawn(async move {
        let mut log_file = std::fs::OpenOptions::new()
            .create(true).append(true).open(&serial_log_path).ok();
            
        let mut current_conn = terminal_rekey_rx.recv().await;
        loop {
            let conn = match current_conn.take() {
                Some(c) => c,
                None => match terminal_rekey_rx.recv().await {
                    Some(c) => c,
                    None => break,
                },
            };

            info!("terminal bridge: active");
            let mut reader = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => continue };
            let mut writer = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => continue };
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(128);

            // Blocking read thread for this specific FD
            let read_handle = std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 { break; }
                    if tx.blocking_send(buf[..n].to_vec()).is_err() { break; }
                }
            });

            let mut coalesce = capsem_core::vm::vsock::CoalesceBuffer::new();
            loop {
                tokio::select! {
                    // Incoming from Guest
                    res = rx.recv() => {
                        match res {
                            Some(data) => {
                                coalesce.push(&data);
                                let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(coalesce.window_ms());
                                while !coalesce.is_full() {
                                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                                        Ok(Some(d)) => { coalesce.push(&d); }
                                        _ => break,
                                    }
                                }
                                coalesce.flush_to(|batch| {
                                    let b = batch.to_vec();
                                    if let Some(ref mut f) = log_file { let _ = f.write_all(&b); }
                                    if let Some(ref pl) = pty_log_out { pl.record_output(&b); }
                                    term_out.push(b);
                                });
                            }
                            None => break, // FD closed
                        }
                    }
                    // Outgoing to Guest (Terminal Input)
                    Some(data) = term_in_rx.recv() => {
                        if writer.write_all(&data).is_err() { break; }
                        let _ = writer.flush();
                    }
                    // Reconnection arriving
                    Some(new_conn) = terminal_rekey_rx.recv() => {
                        current_conn = Some(new_conn);
                        break;
                    }
                }
            }
            let _ = read_handle.join();
        }
        term_out.close();
    });

    // -----------------------------------------------------------------------
    // 2. Stable Control Bridge (Read + Write)
    // -----------------------------------------------------------------------
    let (ctrl_out_tx, mut ctrl_out_rx) = mpsc::channel::<HostToGuest>(128);
    let js = Arc::clone(&job_store);
    let db_ctrl = Arc::clone(&db);
    let mut control_rekey_rx_inner = control_rekey_rx;

    let js_for_teardown = Arc::clone(&job_store);
    let vm_ready_for_reader = Arc::clone(&vm_ready);
    let ready_path_for_reader = ready_path.clone();

    tokio::spawn(async move {
        let mut current_conn = control_rekey_rx_inner.recv().await;
        loop {
            let conn = match current_conn.take() {
                Some(c) => c,
                None => match control_rekey_rx_inner.recv().await {
                    Some(c) => c,
                    None => {
                        js_for_teardown.fail_all("control channel closed");
                        vm_ready_for_reader.store(false, Ordering::Release);
                        let _ = std::fs::remove_file(&ready_path_for_reader);
                        break;
                    }
                },
            };

            info!("control bridge: active");
            let mut writer_fd = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => continue };
            let mut reader_fd = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => continue };
            
            let (msg_tx, mut msg_rx) = mpsc::channel::<Result<GuestToHost>>(32);
            
            // Reader thread
            std::thread::spawn(move || {
                loop {
                    let res = read_control_msg(&mut reader_fd);
                    let is_err = res.is_err();
                    if msg_tx.blocking_send(res).is_err() || is_err { break; }
                }
            });

            loop {
                tokio::select! {
                    // Outgoing messages
                    Some(msg) = ctrl_out_rx.recv() => {
                        if let Err(e) = write_control_msg(&mut writer_fd, &msg) {
                            error!("control bridge: write failed: {e}");
                            break;
                        }
                    }
                    // Incoming messages
                    res = msg_rx.recv() => {
                        match res {
                            Some(Ok(msg)) => handle_guest_msg(msg, &js, &db_ctrl).await,
                            _ => break, // Error or closed, wait for rekey
                        }
                    }
                    // Connection reset
                    Some(new_conn) = control_rekey_rx_inner.recv() => {
                        current_conn = Some(new_conn);
                        break;
                    }
                }
            }
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
            if hb_tx.send(HostToGuest::Ping { epoch_secs }).await.is_err() { break; }
        }
    });

    // -----------------------------------------------------------------------
    // 3. Command Multiplexer (IPC -> Hub)
    // -----------------------------------------------------------------------
    let hub_tx = ctrl_out_tx.clone();
    let js_for_cmd = Arc::clone(&job_store);
    let ipc_tx_for_cmd = ipc_tx.clone();
    let vm_id_for_cmd = vm_id_original;
    let vm_handle_for_cmd = vm_handle_original;
    let db_for_cmd = Arc::clone(&db);
    let pty_log_for_cmd = pty_log.clone();
    let mut ctrl_rx = ctrl_rx;

    tokio::spawn(async move {
        while let Some(msg) = ctrl_rx.recv().await {
            match msg {
                ServiceToProcess::TerminalInput { data } => {
                    if let Some(ref pl) = pty_log_for_cmd { pl.record_input(&data); }
                    let _ = term_in_tx.send(data).await;
                }
                ServiceToProcess::TerminalResize { cols, rows } => { let _ = hub_tx.send(HostToGuest::Resize { cols, rows }).await; }
                ServiceToProcess::Exec { id, command } => {
                    *js_for_cmd.active_exec.lock().unwrap() = Some(crate::job_store::ActiveExec::new(id));
                    db_for_cmd.try_write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
                        timestamp: std::time::SystemTime::now(), exec_id: id, command: command.clone(),
                        source: "api".into(), mcp_call_id: None, trace_id: None, process_name: None,
                    }));
                    let _ = hub_tx.send(HostToGuest::Exec { id, command }).await;
                }
                ServiceToProcess::WriteFile { id, path, data } => { let _ = hub_tx.send(HostToGuest::FileWrite { id, path, data, mode: 0o644 }).await; }
                ServiceToProcess::ReadFile { id, path } => { let _ = hub_tx.send(HostToGuest::FileRead { id, path }).await; }
                ServiceToProcess::Suspend { checkpoint_path } => {
                    let full_path = session_dir.join(checkpoint_path);
                    let h_tx = hub_tx.clone();
                    let j_s = Arc::clone(&js_for_cmd);
                    let v_m = Arc::clone(&vm_handle_for_cmd);
                    let i_tx = ipc_tx_for_cmd.clone();
                    let v_id = vm_id_for_cmd.clone();
                    tokio::spawn(async move {
                        let _ = with_quiescence(&h_tx, &j_s, std::time::Duration::from_secs(10), || async {
                            tokio::task::spawn_blocking(move || {
                                #[cfg(target_os = "macos")]
                                capsem_core::hypervisor::apple_vz::run_on_main_thread(move || {
                                    let v = v_m.blocking_lock();
                                    v.pause()?;
                                    v.save_state(&full_path)?;
                                    Ok(())
                                })?;
                                #[cfg(not(target_os = "macos"))]
                                {
                                    let v = v_m.blocking_lock();
                                    v.pause()?;
                                    v.save_state(&full_path)?;
                                }
                                Ok(())
                            }).await?
                        }).await;
                        let _ = i_tx.send(ProcessToService::StateChanged { id: v_id, state: "Suspended".into(), trigger: "suspend_requested".into() });
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        std::process::exit(0);
                    });
                }
                ServiceToProcess::Shutdown => {
                    let _ = hub_tx.send(HostToGuest::Shutdown).await;
                    let v_m = Arc::clone(&vm_handle_for_cmd);
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let _ = tokio::task::spawn_blocking(move || {
                            #[cfg(target_os = "macos")]
                            let _ = capsem_core::hypervisor::apple_vz::run_on_main_thread(move || { v_m.blocking_lock().stop() });
                            #[cfg(not(target_os = "macos"))]
                            let _ = v_m.blocking_lock().stop();
                        }).await;
                        std::process::exit(0);
                    });
                }
                ServiceToProcess::Ping => {
                    let epoch_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                    let _ = hub_tx.send(HostToGuest::Ping { epoch_secs }).await;
                }
                _ => {}
            }
        }
    });

    // -----------------------------------------------------------------------
    // 4. Central Dispatcher Loop (Vsock -> Hub)
    // -----------------------------------------------------------------------
    let mitm_config_loop = Arc::clone(&mitm_config);
    let mcp_config_loop = Arc::clone(&mcp_config);
    let db_for_audit = Arc::clone(&db);
    let ipc_tx_lifecycle = ipc_tx.clone();
    let ctrl_tx_lifecycle = options._ctrl_tx.clone();
    let vm_id_lifecycle = vm_id.clone();
    let job_store_vsock = Arc::clone(&job_store);

    let mut current_is_restore = true; // Always true after initial handshake
    let mut initial_handshake_done = true;
    
    tokio::spawn(async move {
        let mut pending_aux = deferred_conns;
        
        // Immediately dispatch the deferred_conns from the initial collect
        for conn in pending_aux.drain(..) {
            dispatch_aux_connection(conn, &mitm_config_loop, &mcp_config_loop, &job_store_vsock, &db_for_audit, &ipc_tx_lifecycle, &ctrl_tx_lifecycle, &vm_id_lifecycle);
        }

        while let Some(conn) = vsock_rx.recv().await {
            match conn.port {
                capsem_core::VSOCK_PORT_CONTROL => {
                    info!("control port: connection accepted, performing handshake");
                    let mut fd = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => { pending_aux.clear(); continue; } };
                    let is_rest = current_is_restore;
                    let cli_env_clone = cli_env.clone();
                    let guest_config_clone = guest_config.clone();
                    let hs_res = tokio::task::spawn_blocking(move || perform_handshake(&mut fd, is_rest, &cli_env_clone, Some(guest_config_clone))).await;
                    
                    match hs_res {
                        Ok(Ok(())) => {
                            info!("control port: handshake successful, re-keying bridge");
                            let conn_arc = Arc::new(conn);
                            let _ = control_rekey_tx.send(conn_arc).await;
                            
                            // Handshake succeeded: dispatch any auxiliary connections that arrived with it
                            for aux_conn in pending_aux.drain(..) {
                                dispatch_aux_connection(aux_conn, &mitm_config_loop, &mcp_config_loop, &job_store_vsock, &db_for_audit, &ipc_tx_lifecycle, &ctrl_tx_lifecycle, &vm_id_lifecycle);
                            }
                            
                            if !initial_handshake_done {
                                initial_handshake_done = true;
                                current_is_restore = true; // Subsequent connections are always "restores"
                            }
                        }
                        Ok(Err(e)) => {
                            error!("control port: handshake failed: {e:#}");
                            pending_aux.clear(); // Drop dead FDs
                        }
                        Err(e) => {
                            error!("control port: handshake panicked: {e}");
                            pending_aux.clear();
                        }
                    }
                }
                capsem_core::VSOCK_PORT_TERMINAL => {
                    info!("terminal port: connection accepted, re-keying bridge");
                    let conn_arc = Arc::new(conn);
                    let _ = terminal_rekey_tx.send(conn_arc).await;
                }
                _ => {
                    if initial_handshake_done {
                        // After initial boot, buffer them until the next handshake succeeds
                        // Actually, if we're mid-session and receive an aux conn, it might be legit.
                        // But if it arrives *during* a reconnection storm, we should buffer.
                        // To keep it simple: just dispatch them. The control port rekeying is the
                        // only thing that requires a successful handshake lock-step.
                        dispatch_aux_connection(conn, &mitm_config_loop, &mcp_config_loop, &job_store_vsock, &db_for_audit, &ipc_tx_lifecycle, &ctrl_tx_lifecycle, &vm_id_lifecycle);
                    } else {
                        // Before initial boot, buffer them
                        pending_aux.push(conn);
                    }
                }
            }
        }
    });

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dispatch_aux_connection(
    conn: VsockConnection,
    mitm_config: &Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>,
    mcp_config: &Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    job_store: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    ipc_tx: &broadcast::Sender<ProcessToService>,
    ctrl_tx: &mpsc::Sender<ServiceToProcess>,
    vm_id: &str,
) {
    match conn.port {
        capsem_core::VSOCK_PORT_SNI_PROXY => {
            let config = Arc::clone(mitm_config);
            tokio::spawn(async move { capsem_core::net::mitm_proxy::handle_connection(conn.fd, config).await; drop(conn); });
        }
        capsem_core::VSOCK_PORT_MCP_GATEWAY => {
            let mcp = Arc::clone(mcp_config);
            tokio::spawn(async move { capsem_core::mcp::gateway::serve_mcp_session(conn.fd, mcp).await; drop(conn); });
        }
        capsem_core::VSOCK_PORT_EXEC => {
            let js = Arc::clone(job_store);
            std::thread::spawn(move || {
                let mut file = match clone_fd(conn.fd) { Ok(f) => f, Err(e) => { error!("exec port: clone_fd failed: {e}"); return; } };
                if let Ok(GuestToHost::ExecStarted { id }) = read_control_msg(&mut file) {
                    info!(id, "exec port: received ExecStarted");
                    let mut local_buf = Vec::new();
                    let mut read_buf = [0u8; 8192];
                    loop {
                        match std::io::Read::read(&mut file, &mut read_buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => local_buf.extend_from_slice(&read_buf[..n]),
                        }
                    }
                    // Deposit captured bytes and signal ExecDone it can
                    // proceed. notify_one stores a permit if ExecDone is
                    // not yet parked, so the common "deposit finishes
                    // first" path wakes ExecDone immediately.
                    let notify = {
                        let mut guard = js.active_exec.lock().unwrap();
                        if let Some(ref mut active) = *guard {
                            if active.id == id {
                                active.captured = local_buf;
                                Some(active.deposited.clone())
                            } else { None }
                        } else { None }
                    };
                    if let Some(n) = notify { n.notify_one(); }
                }
                drop(conn);
            });
        }
        capsem_proto::VSOCK_PORT_AUDIT => {
            let db_clone = Arc::clone(db);
            std::thread::spawn(move || {
                let mut file = match clone_fd(conn.fd) { Ok(f) => f, Err(e) => { error!("audit port: clone_fd failed: {e}"); return; } };
                info!("audit port: connected, reading audit records");
                let mut len_buf = [0u8; 4];
                loop {
                    if std::io::Read::read_exact(&mut file, &mut len_buf).is_err() { break; }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    if len > capsem_proto::MAX_FRAME_SIZE as usize { break; }
                    let mut payload = vec![0u8; len];
                    if std::io::Read::read_exact(&mut file, &mut payload).is_err() { break; }
                    if let Ok(record) = capsem_proto::decode_audit_record(&payload) {
                        let timestamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(record.timestamp_us);
                        db_clone.try_write(capsem_logger::WriteOp::AuditEvent(capsem_logger::AuditEvent {
                            timestamp, pid: record.pid, ppid: record.ppid, uid: record.uid, exe: record.exe, comm: record.comm,
                            argv: record.argv, cwd: record.cwd, tty: record.tty, session_id: record.session_id, audit_id: Some(record.audit_id),
                            exec_event_id: None, parent_exe: record.parent_exe,
                        }));
                    }
                }
                drop(conn);
            });
        }
        capsem_core::VSOCK_PORT_LIFECYCLE => {
            let itx = ipc_tx.clone();
            let ctx = ctrl_tx.clone();
            let id = vm_id.to_string();
            std::thread::spawn(move || {
                let mut f = match clone_fd(conn.fd) { Ok(f) => f, Err(_) => return };
                match read_control_msg(&mut f) {
                    Ok(GuestToHost::ShutdownRequest) => {
                        info!("guest requested shutdown via lifecycle port");
                        let _ = itx.send(ProcessToService::ShutdownRequested { id });
                        let _ = ctx.blocking_send(ServiceToProcess::Shutdown);
                    }
                    Ok(GuestToHost::SuspendRequest) => {
                        info!("guest requested suspend via lifecycle port");
                        let _ = itx.send(ProcessToService::SuspendRequested { id });
                        let _ = ctx.blocking_send(ServiceToProcess::Suspend { checkpoint_path: "checkpoint.vzsave".into() });
                    }
                    _ => {}
                }
                drop(conn);
            });
        }
        _ => {}
    }
}

async fn handle_guest_msg(msg: GuestToHost, js: &Arc<JobStore>, db: &Arc<capsem_logger::DbWriter>) {
    match msg {
        GuestToHost::ExecDone { id, exit_code } => {
            // The guest closes the EXEC socket before sending ExecDone, and
            // the host's EXEC-port reader thread may still be finishing its
            // read loop + deposit. Wait on the deposit notifier so we read
            // the actual captured buffer, not a stale empty one. Short
            // timeout guards against lost connections (guest never opened
            // the EXEC port) so we still return in bounded time.
            let notify = js.active_exec.lock().unwrap().as_ref()
                .filter(|a| a.id == id).map(|a| a.deposited.clone());
            if let Some(n) = notify {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    n.notified(),
                ).await;
            }
            let stdout = js.active_exec.lock().unwrap().take()
                .filter(|a| a.id == id).map(|a| a.captured).unwrap_or_default();

            db.try_write(capsem_logger::WriteOp::ExecEventComplete(capsem_logger::ExecEventComplete {
                exec_id: id, exit_code, duration_ms: 0,
                stdout_preview: Some(String::from_utf8_lossy(&stdout[..stdout.len().min(1024)]).into()),
                stderr_preview: None, stdout_bytes: stdout.len() as u64, stderr_bytes: 0, pid: None,
            }));
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                let _ = tx.send(JobResult::Exec { stdout, stderr: vec![], exit_code });
            }
        }
        GuestToHost::FileContent { id, data, .. } => {
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                let _ = tx.send(JobResult::ReadFile { data: Some(data), error: None });
            }
        }
        GuestToHost::FileOpDone { id } => {
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                let _ = tx.send(JobResult::WriteFile { success: true, error: None });
            }
        }
        GuestToHost::SnapshotReady => {
            if let Some(tx) = js.snapshot_ready.lock().unwrap().take() { let _ = tx.send(()); }
        }
        GuestToHost::Error { id, message } => {
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                let _ = tx.send(JobResult::Error { message });
            }
        }
        _ => {}
    }
}

/// Run the boot handshake on an already-accepted control fd.
///
/// Must be invoked from `spawn_blocking`: all I/O here is synchronous on
/// a `std::fs::File` wrapper over the vsock fd, and doing it inline on a
/// tokio worker starves the runtime under multi-VM boot contention.
///
/// `.context()` (not `map_err(anyhow!)`) is used throughout so the
/// underlying `std::io::Error` stays in the error source chain, which
/// `is_retryable_handshake_error` downcasts to decide whether to retry.
fn perform_handshake(
    fd: &mut std::fs::File,
    is_restore: bool,
    env: &[(String, String)],
    conf: Option<capsem_core::net::policy_config::GuestConfig>,
) -> Result<()> {
    read_control_msg(fd).context("initial Ready read failed")?;
    if is_restore {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        write_control_msg(fd, &HostToGuest::BootConfig { epoch_secs: epoch })
            .context("restore BootConfig write failed")?;
        // Re-inject timezone in case host TZ changed since suspend. These
        // writes are best-effort: failing to reset the guest clock is not
        // itself a handshake failure.
        if let Ok(link) = std::fs::read_link("/etc/localtime") {
            if let Some(s) = link.to_str() {
                if let Some(idx) = s.find("/zoneinfo/") {
                    let tz = &s[idx + "/zoneinfo/".len()..];
                    let _ = write_control_msg(
                        fd,
                        &HostToGuest::SetEnv { key: "TZ".into(), value: tz.to_string() },
                    );
                    if let Ok(tz_data) = std::fs::read("/etc/localtime") {
                        let _ = write_control_msg(
                            fd,
                            &HostToGuest::FileWrite {
                                id: 0,
                                path: "/etc/localtime".into(),
                                data: tz_data,
                                mode: 0o644,
                            },
                        );
                    }
                }
            }
        }
        write_control_msg(fd, &HostToGuest::BootConfigDone)
            .context("restore BootConfigDone write failed")?;
    } else {
        capsem_core::send_boot_config(fd, env, conf).context("send_boot_config failed")?;
    }
    read_control_msg(fd).context("BootReady read failed")?;
    Ok(())
}

/// Collect a terminal+control pair from the vsock accept stream.
///
/// Auxiliary connections (SNI proxy, MCP gateway, audit) that race ahead
/// of the pair are parked in `deferred_conns` so the caller can hand
/// them to the long-running dispatcher once the handshake succeeds.
async fn collect_terminal_control_pair(
    vsock_rx: &mut mpsc::UnboundedReceiver<VsockConnection>,
    deferred_conns: &mut Vec<VsockConnection>,
) -> Result<(VsockConnection, VsockConnection)> {
    let mut terminal = None;
    let mut control = None;
    while terminal.is_none() || control.is_none() {
        let Some(conn) = vsock_rx.recv().await else {
            anyhow::bail!("vsock channel closed before terminal/control pair arrived");
        };
        match conn.port {
            capsem_core::VSOCK_PORT_TERMINAL => terminal = Some(conn),
            capsem_core::VSOCK_PORT_CONTROL => control = Some(conn),
            capsem_core::VSOCK_PORT_SNI_PROXY
            | capsem_core::VSOCK_PORT_MCP_GATEWAY
            | capsem_proto::VSOCK_PORT_AUDIT => {
                deferred_conns.push(conn);
            }
            _ => {}
        }
    }
    Ok((terminal.unwrap(), control.unwrap()))
}

/// Narrowly classify a handshake error as retryable.
///
/// Only `BrokenPipe` / `ConnectionReset` at any level of the error source
/// chain count: these are the Apple VZ half-open vsock fingerprints.
/// Other I/O errors (`UnexpectedEof`, `NotFound`, etc.) and decode errors
/// are intentionally NOT retried — retrying against a genuinely broken
/// guest just burns the readiness budget.
fn is_retryable_handshake_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| {
                matches!(
                    io.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                )
            })
    })
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
enum VsockPortKind {
    Terminal,
    Control,
    SniProxy,
    McpGateway,
    Exec,
    Lifecycle,
    Unknown,
}

#[cfg(test)]
fn classify_vsock_port(port: u32) -> VsockPortKind {
    match port {
        capsem_core::VSOCK_PORT_TERMINAL => VsockPortKind::Terminal,
        capsem_core::VSOCK_PORT_CONTROL => VsockPortKind::Control,
        capsem_core::VSOCK_PORT_SNI_PROXY => VsockPortKind::SniProxy,
        capsem_core::VSOCK_PORT_MCP_GATEWAY => VsockPortKind::McpGateway,
        capsem_core::VSOCK_PORT_EXEC => VsockPortKind::Exec,
        capsem_core::VSOCK_PORT_LIFECYCLE => VsockPortKind::Lifecycle,
        _ => VsockPortKind::Unknown,
    }
}

#[cfg(test)]
mod tests;
