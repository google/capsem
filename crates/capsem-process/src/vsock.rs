use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use capsem_core::{
    VsockConnection,
    read_control_msg, send_boot_config, write_control_msg,
};
use capsem_proto::{GuestToHost, HostToGuest};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, error, warn};
use std::io::{Read, Write};

use crate::helpers::clone_fd;
use crate::job_store::{JobStore, JobResult, with_quiescence};

pub(crate) struct VsockOptions {
    pub(crate) vm_id: String,
    pub(crate) vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    pub(crate) vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    pub(crate) ipc_tx: broadcast::Sender<ProcessToService>,
    pub(crate) ctrl_tx: mpsc::Sender<ServiceToProcess>,
    pub(crate) ctrl_rx: mpsc::Receiver<ServiceToProcess>,
    pub(crate) terminal_output: Arc<capsem_core::TerminalOutputQueue>,
    pub(crate) job_store: Arc<JobStore>,
    pub(crate) session_dir: PathBuf,
    pub(crate) cli_env: Vec<(String, String)>,
    pub(crate) guest_config: capsem_core::net::policy_config::GuestConfig,
    pub(crate) mitm_config: Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>,
    pub(crate) mcp_config: Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    pub(crate) net_state: Arc<capsem_core::SandboxNetworkState>,
    pub(crate) is_restore: bool,
    pub(crate) vm_ready: Arc<AtomicBool>,
    pub(crate) uds_path: PathBuf,
}

/// Classify a vsock connection by port number.
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

pub(crate) async fn setup_vsock(options: VsockOptions) -> Result<()> {
    let VsockOptions {
        vm_id,
        vm,
        mut vsock_rx,
        ipc_tx,
        ctrl_tx,
        mut ctrl_rx,
        terminal_output,
        job_store,
        session_dir,
        cli_env,
        guest_config,
        mitm_config,
        mcp_config,
        net_state: _net_state,
        is_restore,
        vm_ready,
        uds_path,
    } = options;
    let mut terminal_conn = None;
    let mut control_conn = None;
    let mut deferred_conns = Vec::new();
    while terminal_conn.is_none() || control_conn.is_none() {
        if let Some(conn) = vsock_rx.recv().await {
            match conn.port {
                capsem_core::VSOCK_PORT_TERMINAL => terminal_conn = Some(conn),
                capsem_core::VSOCK_PORT_CONTROL => control_conn = Some(conn),
                capsem_core::VSOCK_PORT_SNI_PROXY | capsem_core::VSOCK_PORT_MCP_GATEWAY => {
                    deferred_conns.push(conn);
                }
                _ => {}
            }
        }
    }

    let terminal = terminal_conn.unwrap();
    let control = control_conn.unwrap();
    let mut ctrl_file = clone_fd(control.fd)?;

    let _ = read_control_msg(&mut ctrl_file); // Initial Ready
    info!(category = "boot_timeline", from = "Booting", to = "Handshaking", trigger = "ready_received", "state transition");

    if is_restore {
        info!("Abbreviated handshake for restored VM");
        let _ = write_control_msg(&mut ctrl_file, &HostToGuest::BootConfigDone);
    } else {
        send_boot_config(&mut ctrl_file, &cli_env, Some(guest_config))?;
    }

    let _ = read_control_msg(&mut ctrl_file); // BootReady
    info!(category = "boot_timeline", from = "Handshaking", to = "Running", trigger = "booted", "state transition");

    let _ = ipc_tx.send(ProcessToService::StateChanged {
        id: vm_id.clone(),
        state: "Running".into(),
        trigger: "booted".into()
    });
    vm_ready.store(true, Ordering::Release);

    // Signal readiness to service via sentinel file (avoids IPC polling).
    let ready_path = uds_path.with_extension("ready");
    if let Err(e) = std::fs::File::create(&ready_path) {
        warn!("failed to create ready sentinel: {e}");
    }

    let term_out = Arc::clone(&terminal_output);
    let mut term_f = clone_fd(terminal.fd)?;
    let serial_log_path = session_dir.join("serial.log");
    tokio::spawn(async move {
        let mut log_file = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&serial_log_path)
                    .ok()
            }
            #[cfg(not(unix))]
            {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&serial_log_path)
                    .ok()
            }
        };
        // Ensure 0600 even if file already existed
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&serial_log_path, std::fs::Permissions::from_mode(0o600));
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);

        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = term_f.read(&mut buf) {
                if n == 0 { break; }
                let data = buf[..n].to_vec();
                if tx.blocking_send(data).is_err() {
                    break;
                }
            }
        });

        let mut coalesce = capsem_core::vm::vsock::CoalesceBuffer::new();
        loop {
            match rx.recv().await {
                Some(chunk) => { coalesce.push(&chunk); }
                None => break,
            }

            let deadline = tokio::time::Instant::now()
                + std::time::Duration::from_millis(coalesce.window_ms());
            while !coalesce.is_full() {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Some(chunk)) => { coalesce.push(&chunk); }
                    _ => break,
                }
            }

            coalesce.flush_to(|batch| {
                let data = batch.to_vec();

                // Write to serial.log
                if let Some(ref mut f) = log_file {
                    let _ = f.write_all(&data);
                }

                term_out.push(data);
            });
        }
        term_out.close();
    });

    for conn in deferred_conns {
        match conn.port {
            capsem_core::VSOCK_PORT_SNI_PROXY => {
                let config = Arc::clone(&mitm_config);
                tokio::spawn(async move {
                    capsem_core::net::mitm_proxy::handle_connection(conn.fd, config).await;
                    drop(conn); // Hold conn alive
                });
            }
            capsem_core::VSOCK_PORT_MCP_GATEWAY => {
                let mcp = Arc::clone(&mcp_config);
                tokio::spawn(async move {
                    capsem_core::mcp::gateway::serve_mcp_session(conn.fd, mcp).await;
                    drop(conn); // Hold conn alive
                });
            }
            _ => {}
        }
    }

    let mitm_config_loop = Arc::clone(&mitm_config);
    let mcp_config_loop = Arc::clone(&mcp_config);
    let ipc_tx_lifecycle = ipc_tx.clone();
    let ctrl_tx_lifecycle = ctrl_tx.clone();
    let vm_id_lifecycle = vm_id.clone();
    let job_store_vsock = Arc::clone(&job_store);
    tokio::spawn(async move {
        while let Some(conn) = vsock_rx.recv().await {
            match conn.port {
                    capsem_core::VSOCK_PORT_SNI_PROXY => {
                        let config = Arc::clone(&mitm_config_loop);
                        tokio::spawn(async move {
                            capsem_core::net::mitm_proxy::handle_connection(conn.fd, config).await;
                            drop(conn); // Hold conn alive
                        });
                    }
                    capsem_core::VSOCK_PORT_MCP_GATEWAY => {
                        let mcp = Arc::clone(&mcp_config_loop);
                        tokio::spawn(async move {
                            capsem_core::mcp::gateway::serve_mcp_session(conn.fd, mcp).await;
                            drop(conn); // Hold conn alive
                        });
                    }
                    capsem_core::VSOCK_PORT_EXEC => {
                        // Exec output connection: read ExecStarted handshake,
                        // then accumulate all output locally until EOF, then
                        // swap into active_exec in a single lock acquisition.
                        let js = Arc::clone(&job_store_vsock);
                        std::thread::spawn(move || {
                            let mut file = match clone_fd(conn.fd) {
                                Ok(f) => f,
                                Err(e) => {
                                    error!("exec port: clone_fd failed: {e}");
                                    return;
                                }
                            };
                            match read_control_msg(&mut file) {
                                Ok(GuestToHost::ExecStarted { id }) => {
                                    info!(id, "exec port: received ExecStarted");
                                    // Accumulate locally -- no lock contention during I/O.
                                    let mut local_buf = Vec::new();
                                    let mut read_buf = [0u8; 8192];
                                    loop {
                                        match std::io::Read::read(&mut file, &mut read_buf) {
                                            Ok(0) => break,
                                            Ok(n) => local_buf.extend_from_slice(&read_buf[..n]),
                                            Err(_) => break,
                                        }
                                    }
                                    // Single lock acquisition at EOF.
                                    if let Some((active_id, ref mut captured)) =
                                        *js.active_exec.lock().unwrap()
                                    {
                                        if active_id == id {
                                            *captured = local_buf;
                                        }
                                    }
                                }
                                Ok(other) => {
                                    error!("exec port: unexpected message: {other:?}");
                                }
                                Err(e) => {
                                    error!("exec port: read error: {e}");
                                }
                            }
                            drop(conn);
                        });
                    }
                    capsem_core::VSOCK_PORT_LIFECYCLE => {
                        let ipc_tx = ipc_tx_lifecycle.clone();
                        let ctrl_tx = ctrl_tx_lifecycle.clone();
                        let vm_id = vm_id_lifecycle.clone();
                        std::thread::spawn(move || {
                            let mut f = match clone_fd(conn.fd) {
                                Ok(f) => f,
                                Err(e) => {
                                    error!("lifecycle: clone_fd failed: {e}");
                                    return;
                                }
                            };
                            match read_control_msg(&mut f) {
                                Ok(GuestToHost::ShutdownRequest) => {
                                    info!("guest requested shutdown via lifecycle port");
                                    let _ = ipc_tx.send(ProcessToService::ShutdownRequested { id: vm_id });
                                    if let Err(e) = ctrl_tx.blocking_send(ServiceToProcess::Shutdown) {
                                        error!("lifecycle: ctrl_tx send failed: {e}");
                                    }
                                }
                                Ok(GuestToHost::SuspendRequest) => {
                                    info!("guest requested suspend via lifecycle port");
                                    let _ = ipc_tx.send(ProcessToService::SuspendRequested { id: vm_id });
                                    // Let capsem-process handle suspend internally just like shutdown
                                    if let Err(e) = ctrl_tx.blocking_send(ServiceToProcess::Suspend { checkpoint_path: "checkpoint.vzsave".into() }) {
                                        error!("lifecycle: ctrl_tx send failed: {e}");
                                    }
                                }
                                Ok(other) => {
                                    error!("lifecycle port: unexpected message: {other:?}");
                                }
                                Err(e) => {
                                    error!("lifecycle port: read error: {e}");
                                }
                            }
                            drop(conn);
                        });
                    }
                    _ => {}
                }
        }
    });

    let js = Arc::clone(&job_store);
    let mut ctrl_f_read = clone_fd(control.fd)?;
    tokio::task::spawn_blocking(move || {
        loop {
            match read_control_msg(&mut ctrl_f_read) {
                Ok(msg) => {
                    match msg {
                        GuestToHost::ExecDone { id, exit_code } => {
                            info!(id, exit_code, "Received ExecDone from guest");
                            // The exec port reader thread accumulates output
                            // locally and writes to active_exec atomically at
                            // EOF. The agent closes exec_fd before sending
                            // ExecDone, so the reader has already finished by
                            // the time we get here.
                            let stdout = {
                                let active = js.active_exec.lock().unwrap();
                                if let Some((active_id, captured)) = active.as_ref() {
                                    if *active_id == id {
                                        captured.clone()
                                    } else {
                                        Vec::new()
                                    }
                                } else {
                                    Vec::new()
                                }
                            };
                            // Clear active exec after capturing result
                            *js.active_exec.lock().unwrap() = None;

                            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                                let _ = tx.send(JobResult::Exec { stdout, stderr: vec![], exit_code });
                            }
                        }
                        GuestToHost::FileContent { id, data, .. } => {
                            info!(id, len = data.len(), "Received FileContent from guest");
                            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                                let _ = tx.send(JobResult::ReadFile { data: Some(data), error: None });
                            }
                        }
                        GuestToHost::FileOpDone { id } => {
                            info!(id, "Received FileOpDone from guest");
                            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                                let _ = tx.send(JobResult::WriteFile { success: true, error: None });
                            }
                        }
                        GuestToHost::Error { id, message } => {
                            error!(id, message, "Received error from guest");
                            if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
                                let _ = tx.send(JobResult::Error { message });
                            }
                        }
                        GuestToHost::SnapshotReady => {
                            info!("Received SnapshotReady from guest");
                            if let Some(tx) = js.snapshot_ready.lock().unwrap().take() {
                                let _ = tx.send(());
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("control channel closed: {e:#}");
                    break;
                }
            }
        }
    });

    let mut term_f_write = clone_fd(terminal.fd)?;
    let mut ctrl_f_write = clone_fd(control.fd)?;

    // Serialize all control channel writes through a single channel + writer
    // thread. The heartbeat and command handler previously wrote to separate
    // clones of the same vsock fd concurrently, corrupting protocol framing.
    let (ctrl_write_tx, ctrl_write_rx) = std::sync::mpsc::channel::<HostToGuest>();

    let ctrl_ping_tx = ctrl_write_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            if ctrl_ping_tx.send(HostToGuest::Ping).is_err() {
                break;
            }
        }
    });

    // Single control channel writer thread -- serializes heartbeat + commands
    std::thread::spawn(move || {
        while let Ok(msg) = ctrl_write_rx.recv() {
            if write_control_msg(&mut ctrl_f_write, &msg).is_err() {
                break;
            }
        }
    });

    // Command handler: blocking I/O on vsock fds, so use a dedicated thread.
    // Terminal writes go to term_f_write (sole user), control writes go through
    // the serialized ctrl_write_tx channel.
    let ctrl_cmd_tx = ctrl_write_tx;
    let vm_for_cmd = Arc::clone(&vm);
    let js_for_cmd = Arc::clone(&job_store);
    let ipc_tx_for_cmd = ipc_tx.clone();
    let vm_id_for_cmd = vm_id.clone();
    let session_dir_for_cmd = session_dir.clone();
    tokio::task::spawn_blocking(move || {
        while let Some(msg) = ctrl_rx.blocking_recv() {
            match msg {
                ServiceToProcess::TerminalInput { data } => { let _ = term_f_write.write_all(&data); let _ = term_f_write.flush(); }
                ServiceToProcess::TerminalResize { cols, rows } => { let _ = ctrl_cmd_tx.send(HostToGuest::Resize { cols, rows }); }
                ServiceToProcess::Exec { id, command } => { let _ = ctrl_cmd_tx.send(HostToGuest::Exec { id, command }); }
                ServiceToProcess::WriteFile { id, path, data } => { let _ = ctrl_cmd_tx.send(HostToGuest::FileWrite { id, path, data, mode: 0o644 }); }
                ServiceToProcess::ReadFile { id, path } => { let _ = ctrl_cmd_tx.send(HostToGuest::FileRead { id, path }); }
                ServiceToProcess::Shutdown => {
                    let _ = ctrl_cmd_tx.send(HostToGuest::Shutdown);
                    // Give the guest agent SHUTDOWN_GRACE_SECS + margin for kernel
                    // teardown, then force-stop the VM and exit. Without this,
                    // CFRunLoopRun keeps the process alive indefinitely.
                    let vm_clone = Arc::clone(&vm_for_cmd);
                    let rt = tokio::runtime::Handle::current();
                    rt.spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            (capsem_proto::SHUTDOWN_GRACE_SECS * 1000) + 500
                        )).await;
                        let v = vm_clone.lock().await;
                        let _ = v.stop();
                        std::process::exit(0);
                    });
                }
                ServiceToProcess::Ping => { let _ = ctrl_cmd_tx.send(HostToGuest::Ping); }
                ServiceToProcess::Suspend { checkpoint_path } => {
                    info!("Suspend requested, pausing VM...");
                    let vm_clone = Arc::clone(&vm_for_cmd);
                    let ctrl_cmd_tx_clone = ctrl_cmd_tx.clone();
                    let js_clone = Arc::clone(&js_for_cmd);
                    let ipc_tx_clone = ipc_tx_for_cmd.clone();
                    let vm_id_clone = vm_id_for_cmd.clone();
                    let full_path = if std::path::Path::new(&checkpoint_path).is_absolute() {
                        std::path::PathBuf::from(checkpoint_path)
                    } else {
                        session_dir_for_cmd.join(checkpoint_path)
                    };

                    let rt = tokio::runtime::Handle::current();
                    rt.spawn(async move {
                        let res = with_quiescence(&ctrl_cmd_tx_clone, &js_clone, std::time::Duration::from_secs(10), || async {
                            let v = vm_clone.lock().await;
                            v.pause().context("failed to pause")?;
                            v.save_state(&full_path).context("failed to save state")?;
                            v.stop().context("failed to stop")?;
                            Ok(())
                        }).await;

                        if let Err(e) = res {
                            error!("Suspend sequence failed: {e:#}");
                            // Attempt to unfreeze if something failed
                            let _ = ctrl_cmd_tx_clone.send(HostToGuest::Unfreeze);
                        } else {
                            info!("VM suspended and stopped successfully.");
                            let _ = ipc_tx_clone.send(ProcessToService::StateChanged {
                                id: vm_id_clone,
                                state: "Suspended".into(),
                                trigger: "suspend_requested".into(),
                            });
                            // Delay slightly to let StateChanged propagate
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            std::process::exit(0);
                        }
                    });
                }
                _ => {}
            }
        }
    });

    Ok(())
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Vsock port classification
    // -----------------------------------------------------------------------

    #[test]
    fn classify_terminal_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_TERMINAL), VsockPortKind::Terminal);
    }

    #[test]
    fn classify_control_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_CONTROL), VsockPortKind::Control);
    }

    #[test]
    fn classify_sni_proxy_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_SNI_PROXY), VsockPortKind::SniProxy);
    }

    #[test]
    fn classify_mcp_gateway_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_MCP_GATEWAY), VsockPortKind::McpGateway);
    }

    #[test]
    fn classify_exec_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_EXEC), VsockPortKind::Exec);
    }

    #[test]
    fn classify_lifecycle_port() {
        assert_eq!(classify_vsock_port(capsem_core::VSOCK_PORT_LIFECYCLE), VsockPortKind::Lifecycle);
    }

    #[test]
    fn classify_unknown_port() {
        assert_eq!(classify_vsock_port(99999), VsockPortKind::Unknown);
    }

    #[test]
    fn classify_port_zero_unknown() {
        assert_eq!(classify_vsock_port(0), VsockPortKind::Unknown);
    }
}
