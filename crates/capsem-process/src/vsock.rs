use anyhow::{Context, Result};
use capsem_core::{read_control_msg, write_control_msg, VsockConnection};
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use capsem_proto::{GuestToHost, HostToGuest, HostVsockService};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, warn};

use crate::helpers::clone_fd;
use crate::job_store::{with_quiescence, ActiveFileOp, JobResult, JobStore};

type SecurityRulesHandle = Arc<RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>;
type PluginPolicyHandle =
    Arc<RwLock<BTreeMap<String, capsem_core::net::policy_config::SecurityPluginConfig>>>;

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

fn checkpoint_complete_path(checkpoint_path: &std::path::Path) -> PathBuf {
    let marker_name = checkpoint_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.complete"))
        .unwrap_or_else(|| "checkpoint.vzsave.complete".to_string());
    checkpoint_path.with_file_name(marker_name)
}

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
    /// Handler for DNS queries forwarded over vsock port 5007. DNS
    /// NXDOMAIN decisions come from the shared security rules; the network
    /// policy handle remains for resolver mechanics such as redirects/cache.
    pub(crate) dns_handler: Arc<capsem_core::net::dns::DnsHandler>,
    pub(crate) security_rules: SecurityRulesHandle,
    pub(crate) plugin_policy: PluginPolicyHandle,
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
        dns_handler,
        security_rules,
        plugin_policy,
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
    capsem_core::try_send!(
        "terminal_rekey",
        terminal_rekey_tx.send(initial_t.clone()).await
    );
    capsem_core::try_send!(
        "control_rekey",
        control_rekey_tx.send(initial_c.clone()).await
    );

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
    let ready_path = uds_path.with_extension("ready");
    if let Err(e) = std::fs::File::create(&ready_path) {
        warn!("failed to create ready sentinel: {e}");
    }

    // -----------------------------------------------------------------------
    // 1. Stable Terminal Bridge (Read + Write)
    // -----------------------------------------------------------------------
    let term_out = Arc::clone(&terminal_output);
    let serial_log_path = session_dir.join("serial.log");
    let pty_log_out = pty_log.clone();

    tokio::spawn(async move {
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&serial_log_path)
            .ok();

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
            let mut reader = match clone_fd(conn.fd) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let mut writer = match clone_fd(conn.fd) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(128);

            // Blocking read thread for this specific FD
            let read_handle = std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
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
    let security_rules_ctrl = Arc::clone(&security_rules);
    let plugin_policy_ctrl = Arc::clone(&plugin_policy);
    let mut control_rekey_rx_inner = control_rekey_rx;

    let js_for_teardown = Arc::clone(&job_store);
    let vm_ready_for_reader = Arc::clone(&vm_ready);
    let ready_path_for_reader = ready_path.clone();

    // Pending-ack map lives on `JobStore` (see job_store.rs::pending_acks)
    // so IPC handlers can remove entries once no caller is still waiting
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
                        let _ = std::fs::remove_file(&ready_path_for_reader);
                        break;
                    }
                },
            };

            info!("control bridge: active");
            let mut writer_fd = match clone_fd(conn.fd) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let mut reader_fd = match clone_fd(conn.fd) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // Re-write every pending (unacked) message on the fresh conn.
            // Snapshot under the lock so a concurrent insert during replay
            // doesn't double-replay the same message.
            let to_replay: Vec<HostToGuest> = pending
                .pending_acks
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect();
            if !to_replay.is_empty() {
                info!(
                    count = to_replay.len(),
                    "control bridge: replaying pending unacked messages"
                );
                let mut replay_failed = false;
                for msg in &to_replay {
                    if let Err(e) = write_control_msg(&mut writer_fd, msg) {
                        error!("control bridge: replay write failed: {e}");
                        replay_failed = true;
                        break;
                    }
                }
                if replay_failed {
                    continue;
                }
            }

            let (msg_tx, mut msg_rx) = mpsc::channel::<Result<GuestToHost>>(32);

            // Reader thread
            std::thread::spawn(move || loop {
                let res = read_control_msg(&mut reader_fd);
                let is_err = res.is_err();
                if msg_tx.blocking_send(res).is_err() || is_err {
                    break;
                }
            });

            loop {
                tokio::select! {
                    // Outgoing messages -- record ackable ones in `pending`
                    // *before* writing so a write-fail/silent-drop is
                    // recoverable via the next rekey replay.
                    Some(msg) = ctrl_out_rx.recv() => {
                        if let Some(id) = ackable_id(&msg) {
                            pending.pending_acks.lock().unwrap().insert(id, msg.clone());
                        }
                        if let Err(e) = write_control_msg(&mut writer_fd, &msg) {
                            error!("control bridge: write failed: {e}");
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
                    res = msg_rx.recv() => {
                        match res {
                            Some(Ok(GuestToHost::Ack { id })) => {
                                pending.pending_acks.lock().unwrap().remove(&id);
                            }
                            Some(Ok(msg)) => {
                                if let Some(id) = ackable_response_id(&msg) {
                                    if let Err(e) = write_control_msg(&mut writer_fd, &HostToGuest::AckReply { id }) {
                                        error!("control bridge: AckReply write failed: {e}");
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
            if hb_tx.send(HostToGuest::Ping { epoch_secs }).await.is_err() {
                break;
            }
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
    let security_rules_for_cmd = Arc::clone(&security_rules);
    let pty_log_for_cmd = pty_log.clone();
    let mut ctrl_rx = ctrl_rx;

    tokio::spawn(async move {
        while let Some(msg) = ctrl_rx.recv().await {
            match msg {
                ServiceToProcess::TerminalInput { data } => {
                    if let Some(ref pl) = pty_log_for_cmd {
                        pl.record_input(&data);
                    }
                    capsem_core::try_send!("term_in", term_in_tx.send(data).await);
                }
                ServiceToProcess::TerminalResize { cols, rows } => {
                    capsem_core::try_send!(
                        "hub_resize",
                        hub_tx.send(HostToGuest::Resize { cols, rows }).await
                    );
                }
                ServiceToProcess::Exec { id, command } => {
                    // active_exec is owned by ipc.rs's Exec handler -- it
                    // creates the capture slot *before* sending here. The
                    // control bridge owns delivery/replay, so this layer just
                    // forwards without replacing the active_exec slot.
                    let trace_id =
                        capsem_core::telemetry::ambient_capsem_trace_id().or_else(|| {
                            capsem_core::telemetry::child_trace_env(&format!(
                                "{vm_id_for_cmd}-exec-{id}"
                            ))
                            .into_iter()
                            .find_map(|(key, value)| (key == "CAPSEM_TRACE_ID").then_some(value))
                        });
                    let rules = security_rules_for_cmd.read().unwrap().clone();
                    let event_id =
                        capsem_core::security_engine::emit_process_exec_security_write_and_rules(
                            &db_for_cmd,
                            &rules,
                            capsem_logger::ExecEvent {
                                event_id: None,
                                timestamp: std::time::SystemTime::now(),
                                exec_id: id,
                                command: command.clone(),
                                source: "api".into(),
                                trace_id,
                                process_name: None,
                                credential_ref: None,
                            },
                        )
                        .await;
                    if let Some(event_id) = event_id {
                        if let Some(active) = js_for_cmd
                            .active_exec
                            .lock()
                            .unwrap()
                            .as_mut()
                            .filter(|active| active.id == id)
                        {
                            active.event_id = Some(event_id);
                        }
                    }
                    capsem_core::try_send!(
                        "hub_exec",
                        hub_tx.send(HostToGuest::Exec { id, command }).await
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
                    capsem_core::try_send!(
                        "hub_file_read",
                        hub_tx.send(HostToGuest::FileRead { id, path }).await
                    );
                }
                ServiceToProcess::LogFileBoundary {
                    id,
                    action,
                    path,
                    data,
                    size,
                    mime_type,
                } => {
                    let file_action = match action {
                        FileBoundaryAction::Import => capsem_logger::FileAction::Imported,
                        FileBoundaryAction::Export => capsem_logger::FileAction::Exported,
                    };
                    let event_id = emit_explicit_file_security_event(
                        &db_for_cmd,
                        &security_rules_for_cmd,
                        &plugin_policy,
                        FileSecurityBoundary {
                            action: file_action,
                            path,
                            size: Some(size),
                            content: Some(file_content_preview(&data)),
                            mime_type,
                        },
                    )
                    .await;
                    let (success, data, error) = match event_id {
                        Ok(Some(emission)) if emission.enforcement.is_allowed() => (
                            true,
                            rewritten_file_content(&data, size, &emission.event),
                            None,
                        ),
                        Ok(Some(emission)) => (
                            false,
                            None,
                            Some(emission.enforcement.reason.unwrap_or_else(|| {
                                "file boundary blocked by security policy".into()
                            })),
                        ),
                        Ok(None) => (
                            false,
                            None,
                            Some("failed to write file boundary security event".into()),
                        ),
                        Err(error) => (false, None, Some(error)),
                    };
                    if let Some(tx) = js_for_cmd.jobs.lock().unwrap().remove(&id) {
                        capsem_core::try_send!(
                            "job_result_log_file_boundary",
                            tx.send(JobResult::LogFileBoundary {
                                success,
                                data,
                                error
                            })
                        );
                    }
                }
                ServiceToProcess::Suspend { checkpoint_path } => {
                    let full_path = session_dir.join(checkpoint_path);
                    let complete_path = checkpoint_complete_path(&full_path);
                    let _ = std::fs::remove_file(&complete_path);
                    let checkpoint_path_for_save = full_path.clone();
                    let rootfs_img = session_dir.join("guest").join("system").join("rootfs.img");
                    let h_tx = hub_tx.clone();
                    let j_s = Arc::clone(&js_for_cmd);
                    let v_m = Arc::clone(&vm_handle_for_cmd);
                    let i_tx = ipc_tx_for_cmd.clone();
                    let v_id = vm_id_for_cmd.clone();
                    tokio::spawn(async move {
                        // W4: explicit timing spans on every step of suspend so a
                        // future hang lands in process.log with `duration_ms` per
                        // stage. Pre-W4: only "suspend took 8s" total, no per-step
                        // attribution.
                        let suspend_start = std::time::Instant::now();
                        let mut suspend_result = with_quiescence(&h_tx, &j_s, std::time::Duration::from_secs(10), || async {
                            let pause_save_start = std::time::Instant::now();
                            let r = tokio::task::spawn_blocking(move || {
                                #[cfg(target_os = "macos")]
                                capsem_core::hypervisor::apple_vz::run_on_main_thread(move || {
                                    let v = v_m.blocking_lock();
                                    let t0 = std::time::Instant::now();
                                    v.pause()?;
                                    info!(target: "suspend", op = "apple_vz_pause", duration_ms = t0.elapsed().as_millis() as u64, "stage complete");
                                    let t1 = std::time::Instant::now();
                                    v.save_state(&checkpoint_path_for_save)?;
                                    info!(target: "suspend", op = "apple_vz_save_state", duration_ms = t1.elapsed().as_millis() as u64, "stage complete");
                                    Ok(())
                                })?;
                                #[cfg(not(target_os = "macos"))]
                                {
                                    let v = v_m.blocking_lock();
                                    let t0 = std::time::Instant::now();
                                    v.pause()?;
                                    info!(target: "suspend", op = "pause", duration_ms = t0.elapsed().as_millis() as u64, "stage complete");
                                    let t1 = std::time::Instant::now();
                                    v.save_state(&checkpoint_path_for_save)?;
                                    info!(target: "suspend", op = "save_state", duration_ms = t1.elapsed().as_millis() as u64, "stage complete");
                                }
                                Ok(())
                            }).await?;
                            info!(target: "suspend", op = "pause_save_block", duration_ms = pause_save_start.elapsed().as_millis() as u64, "block complete");
                            r
                        }).await;
                        info!(target: "suspend", op = "with_quiescence", duration_ms = suspend_start.elapsed().as_millis() as u64, "phase complete");

                        // After save_state, Apple VZ has stopped writing to rootfs.img
                        // (the virtio-blk-attached system overlay), but APFS may still
                        // be holding dirty pages for it in the host page cache. If
                        // capsem-process exits before APFS flushes them, the next boot
                        // mounts a stale rootfs.img and EXT4 fails with `iget: checksum
                        // invalid` -> overlayfs mount fails -> kernel panic. Force the
                        // flush before we declare success and exit.
                        if suspend_result.is_ok() {
                            let fsync_start = std::time::Instant::now();
                            let checkpoint_path = full_path.clone();
                            let complete_marker_path = complete_path.clone();
                            if let Err(e) =
                                tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                                    let checkpoint_file = std::fs::OpenOptions::new()
                                        .read(true)
                                        .open(&checkpoint_path)?;
                                    checkpoint_file.sync_all()?;

                                    let f = std::fs::OpenOptions::new()
                                        .read(true)
                                        .write(true)
                                        .open(&rootfs_img)?;
                                    f.sync_all()?;
                                    std::fs::write(&complete_marker_path, b"ok\n")?;
                                    let complete_file = std::fs::OpenOptions::new()
                                        .read(true)
                                        .open(&complete_marker_path)?;
                                    complete_file.sync_all()?;
                                    Ok(())
                                })
                                .await
                                .unwrap_or_else(|e| {
                                    Err(std::io::Error::other(format!("join: {e}")))
                                })
                            {
                                error!(target: "fs", op = "fsync", path = "checkpoint.vzsave+rootfs.img", duration_ms = fsync_start.elapsed().as_millis() as u64, error = %e, "host_fsync_checkpoint_and_rootfs failed");
                                suspend_result = Err(anyhow::anyhow!(
                                    "failed to fsync checkpoint/rootfs after save_state: {e}"
                                ));
                            } else {
                                info!(target: "fs", op = "fsync", path = "checkpoint.vzsave+rootfs.img", marker = %complete_path.display(), duration_ms = fsync_start.elapsed().as_millis() as u64, "host_fsync_checkpoint_and_rootfs ok");
                            }
                        } else if let Err(ref e) = suspend_result {
                            error!(target: "suspend", error = %e, "suspend failed");
                        }

                        // Only report Suspended when save_state actually succeeded.
                        // Previously we sent it unconditionally, which made the service
                        // mark a failed-suspend VM as "Suspended" and corrupt the registry.
                        if suspend_result.is_ok() {
                            capsem_core::try_send!(
                                "ipc_state_change",
                                i_tx.send(ProcessToService::StateChanged {
                                    id: v_id,
                                    state: "Suspended".into(),
                                    trigger: "suspend_requested".into()
                                })
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            std::process::exit(0);
                        }
                        // On suspend failure the VM is still running (we did pause but
                        // failed before save_state, or save_state failed). Exit so the
                        // service notices and can re-spawn cleanly; but DO NOT claim
                        // Suspended -- service treats process death without "Suspended"
                        // as crash and will not write a checkpoint marker.
                        let _ = std::fs::remove_file(&complete_path);
                        let _ = std::fs::remove_file(&full_path);
                        warn!("suspend did not complete; exiting without Suspended marker");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        std::process::exit(1);
                    });
                }
                ServiceToProcess::Shutdown => {
                    capsem_core::try_send!(
                        "hub_shutdown",
                        hub_tx.send(HostToGuest::Shutdown).await
                    );
                    let v_m = Arc::clone(&vm_handle_for_cmd);
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        // channel-closed-ok: spawn_blocking JoinHandle and stop()'s
                        // Result are best-effort cleanup tails; nothing waits on them.
                        let _ = tokio::task::spawn_blocking(move || {
                            #[cfg(target_os = "macos")]
                            let _ =
                                capsem_core::hypervisor::apple_vz::run_on_main_thread(move || {
                                    v_m.blocking_lock().stop()
                                });
                            #[cfg(not(target_os = "macos"))]
                            let _ = v_m.blocking_lock().stop();
                        })
                        .await;
                        std::process::exit(0);
                    });
                }
                ServiceToProcess::Ping => {
                    let epoch_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    capsem_core::try_send!(
                        "hub_ping",
                        hub_tx.send(HostToGuest::Ping { epoch_secs }).await
                    );
                }
                _ => {}
            }
        }
    });

    // -----------------------------------------------------------------------
    // 4. Central Dispatcher Loop (Vsock -> Hub)
    // -----------------------------------------------------------------------
    let mitm_config_loop = Arc::clone(&mitm_config);
    let dns_handler_loop = Arc::clone(&dns_handler);
    let security_rules_loop = Arc::clone(&security_rules);
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
            dispatch_aux_connection(
                conn,
                &mitm_config_loop,
                &dns_handler_loop,
                &security_rules_loop,
                &job_store_vsock,
                &db_for_audit,
                &ipc_tx_lifecycle,
                &ctrl_tx_lifecycle,
                &vm_id_lifecycle,
            );
        }

        while let Some(conn) = vsock_rx.recv().await {
            match conn.port {
                capsem_core::VSOCK_PORT_CONTROL => {
                    info!("control port: connection accepted, performing handshake");
                    let mut fd = match clone_fd(conn.fd) {
                        Ok(f) => f,
                        Err(_) => {
                            pending_aux.clear();
                            continue;
                        }
                    };
                    let is_rest = current_is_restore;
                    let cli_env_clone = cli_env.clone();
                    let guest_config_clone = guest_config.clone();
                    let hs_res = tokio::task::spawn_blocking(move || {
                        perform_handshake(
                            &mut fd,
                            is_rest,
                            &cli_env_clone,
                            Some(guest_config_clone),
                        )
                    })
                    .await;

                    match hs_res {
                        Ok(Ok(())) => {
                            info!("control port: handshake successful, re-keying bridge");
                            let conn_arc = Arc::new(conn);
                            capsem_core::try_send!(
                                "control_rekey",
                                control_rekey_tx.send(conn_arc).await
                            );

                            // Handshake succeeded: dispatch any auxiliary connections that arrived with it
                            for aux_conn in pending_aux.drain(..) {
                                dispatch_aux_connection(
                                    aux_conn,
                                    &mitm_config_loop,
                                    &dns_handler_loop,
                                    &security_rules_loop,
                                    &job_store_vsock,
                                    &db_for_audit,
                                    &ipc_tx_lifecycle,
                                    &ctrl_tx_lifecycle,
                                    &vm_id_lifecycle,
                                );
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
                    capsem_core::try_send!(
                        "terminal_rekey",
                        terminal_rekey_tx.send(conn_arc).await
                    );
                }
                _ => {
                    if initial_handshake_done {
                        // After initial boot, buffer them until the next handshake succeeds
                        // Actually, if we're mid-session and receive an aux conn, it might be legit.
                        // But if it arrives *during* a reconnection storm, we should buffer.
                        // To keep it simple: just dispatch them. The control port rekeying is the
                        // only thing that requires a successful handshake lock-step.
                        dispatch_aux_connection(
                            conn,
                            &mitm_config_loop,
                            &dns_handler_loop,
                            &security_rules_loop,
                            &job_store_vsock,
                            &db_for_audit,
                            &ipc_tx_lifecycle,
                            &ctrl_tx_lifecycle,
                            &vm_id_lifecycle,
                        );
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
    dns_handler: &Arc<capsem_core::net::dns::DnsHandler>,
    security_rules: &Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
    job_store: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    ipc_tx: &broadcast::Sender<ProcessToService>,
    ctrl_tx: &mpsc::Sender<ServiceToProcess>,
    vm_id: &str,
) {
    match HostVsockService::from_port(conn.port) {
        Some(HostVsockService::SniProxy) => {
            let config = Arc::clone(mitm_config);
            tokio::spawn(async move {
                capsem_core::net::mitm_proxy::handle_connection(conn.fd, config).await;
                drop(conn);
            });
        }
        Some(HostVsockService::DnsProxy) => {
            // DNS proxy connections are long-lived framed sessions.
            // The guest keeps a small worker pool of persistent vsock
            // fds; each frame is one DNS query/response round trip.
            // T3.3 -- after the handler returns we build a `DnsEvent`
            // and push it through the shared `DbWriter` so a
            // `dns_events` row is recorded for every query (allowed,
            // denied, error). `trace_id` is the ambient capsem trace
            // id so a single agent action joins across `dns_events`
            // and `net_events`.
            let handler = Arc::clone(dns_handler);
            let db_for_dns = Arc::clone(db);
            let security_rules = Arc::clone(security_rules);
            tokio::spawn(async move {
                serve_dns_session(conn, handler, db_for_dns, security_rules).await;
            });
        }
        Some(HostVsockService::Exec) => {
            let js = Arc::clone(job_store);
            std::thread::spawn(move || {
                let mut file = match clone_fd(conn.fd) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("exec port: clone_fd failed: {e}");
                        return;
                    }
                };
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
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(n) = notify {
                        n.notify_one();
                    }
                }
                drop(conn);
            });
        }
        Some(HostVsockService::Audit) => {
            let db_clone = Arc::clone(db);
            let security_rules = security_rules.read().unwrap().clone();
            std::thread::spawn(move || {
                let mut file = match clone_fd(conn.fd) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("audit port: clone_fd failed: {e}");
                        return;
                    }
                };
                info!("audit port: connected, reading audit records");
                let mut len_buf = [0u8; 4];
                loop {
                    if std::io::Read::read_exact(&mut file, &mut len_buf).is_err() {
                        break;
                    }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    if len > capsem_proto::MAX_FRAME_SIZE as usize {
                        break;
                    }
                    let mut payload = vec![0u8; len];
                    if std::io::Read::read_exact(&mut file, &mut payload).is_err() {
                        break;
                    }
                    if let Ok(record) = capsem_proto::decode_audit_record(&payload) {
                        let timestamp = std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_micros(record.timestamp_us);
                        capsem_core::security_engine::emit_process_audit_security_write_and_rules_blocking(
                            &db_clone,
                            &security_rules,
                            capsem_logger::AuditEvent {
                                event_id: None,
                                timestamp,
                                pid: record.pid,
                                ppid: record.ppid,
                                uid: record.uid,
                                exe: record.exe,
                                comm: record.comm,
                                argv: record.argv,
                                cwd: record.cwd,
                                tty: record.tty,
                                session_id: record.session_id,
                                audit_id: Some(record.audit_id),
                                exec_event_id: None,
                                parent_exe: record.parent_exe,
                                trace_id: capsem_core::telemetry::ambient_capsem_trace_id(),
                                credential_ref: None,
                            },
                        );
                    }
                }
                drop(conn);
            });
        }
        Some(HostVsockService::Lifecycle) => {
            let itx = ipc_tx.clone();
            let ctx = ctrl_tx.clone();
            let id = vm_id.to_string();
            std::thread::spawn(move || {
                let mut f = match clone_fd(conn.fd) {
                    Ok(f) => f,
                    Err(_) => return,
                };
                match read_control_msg(&mut f) {
                    Ok(GuestToHost::ShutdownRequest) => {
                        info!("guest requested shutdown via lifecycle port");
                        capsem_core::try_send!(
                            "ipc_lifecycle_shutdown",
                            itx.send(ProcessToService::ShutdownRequested { id })
                        );
                        capsem_core::try_send!(
                            "ctrl_lifecycle_shutdown",
                            ctx.blocking_send(ServiceToProcess::Shutdown)
                        );
                    }
                    Ok(GuestToHost::SuspendRequest) => {
                        info!("guest requested suspend via lifecycle port");
                        capsem_core::try_send!(
                            "ipc_lifecycle_suspend",
                            itx.send(ProcessToService::SuspendRequested { id })
                        );
                        capsem_core::try_send!(
                            "ctrl_lifecycle_suspend",
                            ctx.blocking_send(ServiceToProcess::Suspend {
                                checkpoint_path: "checkpoint.vzsave".into()
                            })
                        );
                    }
                    other => {
                        // W4: a lifecycle-port frame the host doesn't recognize
                        // is exactly the bug pattern that wedged us today.
                        warn!(target: "ipc", unhandled = ?other, "VSOCK_PORT_LIFECYCLE: unknown variant; this binary may be older than its peer");
                    }
                }
                drop(conn);
            });
        }
        Some(HostVsockService::Control | HostVsockService::Terminal) => {
            warn!(
                target: "ipc",
                port = conn.port,
                service = HostVsockService::from_port(conn.port).map(HostVsockService::as_str),
                "vsock dispatch: control/terminal service reached auxiliary dispatcher; connection ignored"
            );
        }
        other => {
            warn!(
                target: "ipc",
                port = conn.port,
                service = ?other.map(HostVsockService::as_str),
                "vsock dispatch: unknown port; auxiliary connection ignored"
            );
        }
    }
}

/// Persistent DNS query handler over the vsock DNS port (T3.2).
///
/// Wire shape:
///   guest -> host: `[u32 BE length][rmp DnsRequest]`
///   host -> guest: `[u32 BE length][rmp DnsResponse]`
///
/// Each connection carries many serialized request/response frames.
/// The guest-side worker pool owns concurrency: one in-flight DNS query
/// per persistent vsock fd. This removes per-query connection churn
/// without introducing response multiplexing ambiguity.
async fn serve_dns_session(
    conn: VsockConnection,
    handler: Arc<capsem_core::net::dns::DnsHandler>,
    db: Arc<capsem_logger::DbWriter>,
    security_rules: Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
) {
    use std::io::{Read as _, Write as _};

    let conn_fd = conn.fd;
    loop {
        // Move the fd in/out via spawn_blocking so we don't run sync I/O on
        // the tokio runtime. The DNS handler itself is async (UDP forwarder
        // returns Future), so we read one request, run the handler, then
        // write one response.
        let read_res = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
            let mut file = clone_fd(conn_fd)?;
            let mut len_buf = [0u8; 4];
            match file.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(error) => {
                    return Err(error).context("DNS port: failed to read length prefix");
                }
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > capsem_proto::MAX_FRAME_SIZE as usize {
                anyhow::bail!("DNS port: frame too large ({len} > MAX_FRAME_SIZE)");
            }
            let mut payload = vec![0u8; len];
            file.read_exact(&mut payload)
                .context("DNS port: failed to read payload")?;
            Ok(Some(payload))
        })
        .await;

        let payload = match read_res {
            Ok(Ok(Some(p))) => p,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                warn!(error = %e, "DNS port: read failed");
                break;
            }
            Err(e) => {
                warn!(error = %e, "DNS port: read task panicked");
                break;
            }
        };

        let req = match capsem_proto::decode_dns_request(&payload) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "DNS port: decode_dns_request failed");
                break;
            }
        };

        let result = handler.handle(&req.raw).await;

        // T3.3 -- record one `dns_events` row per query. trace_id ties it
        // back to the agent action; source_proto distinguishes UDP from
        // TCP DNS at the source side. Await the security emitter so DNS audit
        // rows are accepted by the single security/logging rail before the
        // DNS response is returned.
        let event = capsem_core::net::dns::build_dns_event(
            &result,
            Some(req.proto.as_str()),
            req.process_name.clone(),
            capsem_core::telemetry::ambient_capsem_trace_id(),
        );
        emit_dns_security_write_and_rules(&db, &security_rules, event).await;

        let response = capsem_proto::DnsResponse {
            raw: result.answer_bytes,
            decision: result.decision.as_str().to_string(),
            rcode: result.rcode,
        };

        let frame = match capsem_proto::encode_dns_response(&response) {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "DNS port: encode_dns_response failed");
                break;
            }
        };

        let write_res = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut file = clone_fd(conn_fd)?;
            file.write_all(&frame)
                .context("DNS port: failed to write response frame")?;
            Ok(())
        })
        .await;
        match write_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(error = %e, "DNS port: write failed");
                break;
            }
            Err(e) => {
                warn!(error = %e, "DNS port: write task panicked");
                break;
            }
        }
    }

    drop(conn);
}

async fn emit_dns_security_write_and_rules(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
    event: capsem_logger::DnsEvent,
) -> Option<capsem_core::security_engine::SecurityEventId> {
    let security_event = capsem_core::net::dns::security_event_from_dns_event(&event);
    let event_id = capsem_core::security_engine::emit_security_write(
        db,
        capsem_logger::WriteOp::DnsEvent(event),
    )
    .await?;
    let rules = security_rules.read().unwrap().clone();
    if let Err(error) = capsem_core::security_engine::emit_matching_security_rules(
        db,
        event_id.clone(),
        capsem_core::security_engine::RuntimeSecurityEventType::DnsQuery,
        &rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        warn!(error = %error, "failed to emit DNS security rule ledger rows");
    }
    Some(event_id)
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Returns `Some(id)` for HostToGuest variants whose delivery the host
/// bridge tracks via the pending-ack map. The agent acks these on
/// receipt; the bridge replays them on every fresh conn until acked.
/// Non-ackable variants (Resize, Ping, Shutdown, BootConfig, etc.) are
/// either side-effect-free or fire-and-forget at boot, so we don't
/// burden the wire with per-message acks for them.
fn ackable_id(msg: &HostToGuest) -> Option<u64> {
    match msg {
        HostToGuest::Exec { id, .. }
        | HostToGuest::FileWrite { id, .. }
        | HostToGuest::FileRead { id, .. }
        | HostToGuest::FileDelete { id, .. } => Some(*id),
        _ => None,
    }
}

/// Returns `Some(id)` for `GuestToHost` variants the agent retains in
/// its symmetric pending_responses map and replays on every fresh
/// control conn. The host emits `HostToGuest::AckReply { id }` on
/// receipt so the agent can drop the entry. Mirrors `ackable_id` but
/// for the return path.
fn ackable_response_id(msg: &GuestToHost) -> Option<u64> {
    match msg {
        GuestToHost::ExecDone { id, .. }
        | GuestToHost::FileOpDone { id }
        | GuestToHost::FileContent { id, .. }
        | GuestToHost::Error { id, .. } => Some(*id),
        _ => None,
    }
}

const FILE_SECURITY_CONTENT_PREVIEW_MAX: usize = 64 * 1024;

struct FileSecurityBoundary {
    action: capsem_logger::FileAction,
    path: String,
    size: Option<u64>,
    content: Option<String>,
    mime_type: Option<String>,
}

fn file_content_preview(data: &[u8]) -> String {
    String::from_utf8_lossy(&data[..data.len().min(FILE_SECURITY_CONTENT_PREVIEW_MAX)]).into_owned()
}

async fn emit_explicit_file_security_event(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
    plugin_policy: &PluginPolicyHandle,
    boundary: FileSecurityBoundary,
) -> Result<Option<capsem_core::security_engine::SecurityRuleEmission>, String> {
    let rules = security_rules.read().unwrap().clone();
    let plugins = plugin_policy.read().unwrap().clone();
    capsem_core::security_engine::emit_explicit_file_security_write_and_rules_with_plugins(
        db,
        &rules,
        plugins,
        capsem_core::security_engine::ExplicitFileSecurityEvent {
            action: boundary.action,
            path: boundary.path,
            size: boundary.size,
            content: boundary.content,
            mime_type: boundary.mime_type,
            trace_id: None,
            credential_ref: None,
        },
    )
    .await
}

fn rewritten_file_content(
    original_preview: &[u8],
    original_size: u64,
    event: &capsem_core::security_engine::SecurityEvent,
) -> Option<Vec<u8>> {
    if original_preview.len() as u64 != original_size {
        return None;
    }
    let mutating_rewrite = event.plugin_executions.iter().any(|execution| {
        execution.applied
            && !matches!(
                execution.stage,
                capsem_core::security_engine::SecurityPluginStage::Logging
            )
            && event.detections.iter().any(|detection| {
                detection.plugin_id.as_deref() == Some(execution.plugin_id.as_str())
                    && detection.plugin_mode
                        == Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite)
            })
    });
    if !mutating_rewrite {
        return None;
    }
    let file = event.file.as_ref()?;
    let content = file
        .import_content
        .as_deref()
        .or(file.export_content.as_deref())
        .or(file.read_content.as_deref())
        .or(file.write_content.as_deref())
        .or(file.create_content.as_deref())
        .or(file.delete_content.as_deref())
        .or(file.content.as_deref())?;
    if content.as_bytes() == original_preview {
        None
    } else {
        Some(content.as_bytes().to_vec())
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
        GuestToHost::ExecDone { id, exit_code } => {
            // The guest closes the EXEC socket before sending ExecDone, and
            // the host's EXEC-port reader thread may still be finishing its
            // read loop + deposit. Wait on the deposit notifier so we read
            // the actual captured buffer, not a stale empty one. Short
            // timeout guards against lost connections (guest never opened
            // the EXEC port) so we still return in bounded time.
            let notify = js
                .active_exec
                .lock()
                .unwrap()
                .as_ref()
                .filter(|a| a.id == id)
                .map(|a| a.deposited.clone());
            if let Some(n) = notify {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(100), n.notified()).await;
            }
            let active_exec = js.active_exec.lock().unwrap().take().filter(|a| a.id == id);
            let (event_id, duration_ms, stdout) = active_exec
                .map(|active| {
                    (
                        active.event_id,
                        active.started_at.elapsed().as_millis() as u64,
                        active.captured,
                    )
                })
                .unwrap_or((None, 0, Vec::new()));

            let complete = capsem_logger::ExecEventComplete {
                exec_id: id,
                exit_code,
                duration_ms,
                stdout_preview: Some(
                    String::from_utf8_lossy(&stdout[..stdout.len().min(1024)]).into(),
                ),
                stderr_preview: None,
                stdout_bytes: stdout.len() as u64,
                stderr_bytes: 0,
                pid: None,
            };
            if let Some(event_id) = event_id {
                let rules = security_rules.read().unwrap().clone();
                capsem_core::security_engine::emit_process_complete_security_write_and_rules(
                    db, &rules, event_id, complete,
                )
                .await;
            } else {
                warn!(
                    exec_id = id,
                    "exec completion arrived without a primary security event id; updating exec row without rule ledger match"
                );
                capsem_core::security_engine::emit_process_complete_security_write_only(
                    db, complete,
                )
                .await;
            }
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!(
                    "job_result_exec",
                    tx.send(JobResult::Exec {
                        stdout,
                        stderr: vec![],
                        exit_code
                    })
                );
            }
        }
        GuestToHost::FileContent { id, path, data } => {
            let context = {
                let mut active_file_ops = js.active_file_ops.lock().unwrap();
                active_file_ops.remove(&id)
            };
            let (path, action) = match context {
                Some(ActiveFileOp::Read { path }) => (path, capsem_logger::FileAction::Exported),
                Some(ActiveFileOp::Write { path, .. }) => (path, capsem_logger::FileAction::Read),
                None => (path, capsem_logger::FileAction::Read),
            };
            let boundary = emit_explicit_file_security_event(
                db,
                security_rules,
                plugin_policy,
                FileSecurityBoundary {
                    action,
                    path,
                    size: Some(data.len() as u64),
                    content: Some(file_content_preview(&data)),
                    mime_type: None,
                },
            )
            .await;
            match boundary {
                Ok(Some(emission)) if emission.enforcement.is_allowed() => {}
                Ok(Some(emission)) if action == capsem_logger::FileAction::Exported => {
                    let error = emission
                        .enforcement
                        .reason
                        .unwrap_or_else(|| "file export blocked by security policy".into());
                    if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                        capsem_core::try_send!(
                            "job_result_read_file_blocked",
                            tx.send(JobResult::ReadFile {
                                data: None,
                                error: Some(error)
                            })
                        );
                    }
                    return;
                }
                Ok(Some(emission)) => {
                    warn!(
                        id,
                        action = ?action,
                        decision = ?emission.enforcement.action,
                        "file boundary emitted non-allow decision after data was already local"
                    );
                }
                Ok(None) => {
                    warn!(id, action = ?action, "failed to write file boundary security event");
                }
                Err(error) => {
                    warn!(id, action = ?action, error, "failed to evaluate file boundary");
                }
            }
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!(
                    "job_result_read_file",
                    tx.send(JobResult::ReadFile {
                        data: Some(data),
                        error: None
                    })
                );
            }
        }
        GuestToHost::FileOpDone { id } => {
            let context = {
                let mut active_file_ops = js.active_file_ops.lock().unwrap();
                active_file_ops.remove(&id)
            };
            if let Some(context) = context {
                match context {
                    ActiveFileOp::Write { path, data } => {
                        if let Err(error) = emit_explicit_file_security_event(
                            db,
                            security_rules,
                            plugin_policy,
                            FileSecurityBoundary {
                                action: capsem_logger::FileAction::Modified,
                                path,
                                size: Some(data.len() as u64),
                                content: Some(file_content_preview(&data)),
                                mime_type: None,
                            },
                        )
                        .await
                        {
                            warn!(
                                id,
                                error, "failed to evaluate file write completion boundary"
                            );
                        }
                    }
                    ActiveFileOp::Read { path } => {
                        warn!(
                            id,
                            path,
                            "FileOpDone received for read context; skipping explicit file security event"
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
        GuestToHost::SnapshotReady => {
            if let Some(tx) = js.snapshot_ready.lock().unwrap().take() {
                capsem_core::try_send!("snapshot_ready", tx.send(()));
            }
        }
        GuestToHost::Error { id, message } => {
            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!("job_result_error", tx.send(JobResult::Error { message }));
            }
        }
        other => {
            warn!(target: "ipc", unhandled = ?other, "handle_guest_msg: unknown variant; this binary may be older than its peer");
        }
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
        let traceparent = capsem_core::telemetry::current_parent_traceparent().to_string();
        write_control_msg(
            fd,
            &HostToGuest::BootConfig {
                epoch_secs: epoch,
                traceparent,
            },
        )
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
                        &HostToGuest::SetEnv {
                            key: "TZ".into(),
                            value: tz.to_string(),
                        },
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
/// Auxiliary connections (MITM proxy, audit, DNS) that race ahead
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
            | capsem_proto::VSOCK_PORT_AUDIT
            | capsem_proto::VSOCK_PORT_DNS_PROXY => {
                deferred_conns.push(conn);
            }
            _ => {}
        }
    }
    Ok((terminal.unwrap(), control.unwrap()))
}

/// Classify a handshake error as retryable.
///
/// All cover the same observed pattern: Apple VZ tears the post-restoreState
/// vsock conn down between the guest sending one frame and the next, leaving
/// the host with a dead fd. The kind we get depends on which side closes
/// first and how:
///   - `BrokenPipe` / `ConnectionReset` -- guest's end shut down hard.
///   - `UnexpectedEof` -- guest closed cleanly mid-frame; we get EOF on
///     `read_exact`. Empirically this is the dominant kind under heavy
///     suspend/resume churn (see commit history of this file).
///
/// Retrying drops the dead pair and waits for the guest's reconnect loop to
/// open a fresh terminal+control pair, then re-runs the handshake. Capped
/// at `HANDSHAKE_RETRY_MAX` so a genuinely broken guest fails fast.
fn is_retryable_handshake_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
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
    Exec,
    Lifecycle,
    Audit,
    DnsProxy,
    Unknown,
}

#[cfg(test)]
fn classify_vsock_port(port: u32) -> VsockPortKind {
    match HostVsockService::from_port(port) {
        Some(HostVsockService::Terminal) => VsockPortKind::Terminal,
        Some(HostVsockService::Control) => VsockPortKind::Control,
        Some(HostVsockService::SniProxy) => VsockPortKind::SniProxy,
        Some(HostVsockService::Exec) => VsockPortKind::Exec,
        Some(HostVsockService::Lifecycle) => VsockPortKind::Lifecycle,
        Some(HostVsockService::Audit) => VsockPortKind::Audit,
        Some(HostVsockService::DnsProxy) => VsockPortKind::DnsProxy,
        None => VsockPortKind::Unknown,
    }
}

#[cfg(test)]
mod tests;
