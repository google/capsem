use std::io::Read;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capsem_core::{
    CoalesceBuffer, GuestToHost, HostState, VsockConnection,
    VSOCK_PORT_CONTROL, VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
    decode_guest_msg, validate_guest_msg, MAX_FRAME_SIZE,
};
use capsem_core::mcp::gateway::{self, McpGatewayConfig};
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy_config;
use tauri::{Emitter, Manager};
use tracing::{info, warn};

use crate::boot::{clone_fd, read_control_msg, send_boot_config};
use crate::logging::write_perf_log;
use crate::session_mgmt::{session_dir_for, flush_session_summary};
use crate::state::AppState;

/// Forward serial console bytes to the terminal output queue.
pub(crate) async fn serial_to_events(
    terminal_output: Arc<crate::state::TerminalOutputQueue>,
    mut rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
) {
    loop {
        match rx.recv().await {
            Ok(bytes) => {
                terminal_output.push(bytes);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                info!("serial broadcast channel closed");
                break;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                info!("serial receiver lagged by {n} messages");
            }
        }
    }
}

/// Forward vsock terminal data to the terminal output queue with coalescing.
pub(crate) async fn vsock_terminal_to_events(
    terminal_output: Arc<crate::state::TerminalOutputQueue>,
    vsock_fd: RawFd,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);

    std::thread::spawn(move || {
        let mut file = match clone_fd(vsock_fd) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("vsock terminal: failed to clone fd: {e}");
                return;
            }
        };
        let mut buf = [0u8; 65536];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut coalesce = CoalesceBuffer::new();
    loop {
        match rx.recv().await {
            Some(chunk) => { coalesce.push(&chunk); }
            None => break,
        }

        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(coalesce.window_ms());
        while !coalesce.is_full() {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(chunk)) => { coalesce.push(&chunk); }
                _ => break,
            }
        }

        coalesce.flush_to(|batch| {
            terminal_output.push(batch.to_vec());
        });
    }
}

/// Handle vsock control channel: read incoming messages, handle heartbeat.
pub(crate) async fn vsock_control_handler(app_handle: tauri::AppHandle, control_fd: RawFd) {
    use capsem_core::{HostToGuest, encode_host_msg};
    use std::io::Write;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuestToHost>(32);

    // Reader thread.
    std::thread::spawn(move || {
        let mut file = match clone_fd(control_fd) {
            Ok(f) => f,
            Err(e) => {
                warn!("vsock control: failed to clone fd for reading: {e}");
                return;
            }
        };
        loop {
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > MAX_FRAME_SIZE as usize {
                warn!("vsock control: frame too large ({len} bytes), dropping connection");
                break;
            }
            let mut payload = vec![0u8; len];
            if file.read_exact(&mut payload).is_err() {
                break;
            }
            match decode_guest_msg(&payload) {
                Ok(msg) => {
                    if tx.blocking_send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("vsock control: decode error: {e}");
                }
            }
        }
    });

    let mut write_file = match clone_fd(control_fd) {
        Ok(f) => f,
        Err(e) => {
            warn!("vsock control: failed to clone fd for writing: {e}");
            return;
        }
    };

    let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(30));
    // Skip the immediate tick.
    heartbeat_interval.tick().await;

    loop {
        tokio::select! {
            msg_opt = rx.recv() => {
                let msg = match msg_opt {
                    Some(m) => m,
                    None => break,
                };

                {
                    let state = app_handle.state::<AppState>();
                    let vm_id = state.active_session_id.lock().unwrap().clone();
                    if let Some(ref id) = vm_id {
                        let vms = state.vms.lock().unwrap();
                        if let Some(instance) = vms.get(id) {
                            if let Err(e) = validate_guest_msg(&msg, instance.state_machine.state()) {
                                warn!("vsock: rejected control message: {e}");
                                continue;
                            }
                        }
                    }
                }

                match msg {
                    GuestToHost::Pong => {
                        info!("vsock: heartbeat pong received");
                    }
                    GuestToHost::ExecDone { id, exit_code } => {
                        info!("vsock: exec done (id={id}, exit_code={exit_code})");
                    }
                    GuestToHost::BootTiming { ref stages } => {
                        let clean: Vec<_> = stages.iter()
                            .filter(|s| s.name.len() <= 64
                                && s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                                && s.duration_ms <= 600_000)
                            .take(32)
                            .collect();
                        if clean.len() != stages.len() {
                            warn!("boot timing: dropped {} invalid entries", stages.len() - clean.len());
                        }
                        for s in &clean {
                            info!(stage = %s.name, duration_ms = s.duration_ms, "boot timing");
                        }
                        let total: u64 = clean.iter().map(|s| s.duration_ms).sum();
                        info!(total_ms = total, "boot timing total");
                        let _ = app_handle.emit("boot-timing", serde_json::json!({
                            "stages": clean.iter().map(|s| {
                                serde_json::json!({"name": s.name, "duration_ms": s.duration_ms})
                            }).collect::<Vec<_>>(),
                            "total_ms": total,
                        }));
                    }
                    other => {
                        info!("vsock: unhandled control message: {other:?}");
                    }
                }
            }
            _ = heartbeat_interval.tick() => {
                let epoch_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let ping = HostToGuest::Heartbeat { epoch_secs };
                if let Ok(frame) = encode_host_msg(&ping) {
                    if let Err(e) = write_file.write_all(&frame) {
                        warn!("vsock control: failed to send heartbeat: {e}");
                        break;
                    }
                    let _ = write_file.flush();
                }
            }
        }
    }
}

/// Initialize auto-snapshot scheduler and wire it into the MCP gateway config.
pub(crate) async fn wire_auto_snapshots(
    config: &Arc<McpGatewayConfig>,
    session_dir: &Path,
) -> Option<(Arc<tokio::sync::Mutex<capsem_core::auto_snapshot::AutoSnapshotScheduler>>, Duration)> {
    let workspace = session_dir.join("workspace");
    if !workspace.exists() {
        return None;
    }

    let snap_settings = policy_config::load_merged_settings();
    let snap_auto_max = snap_settings.iter()
        .find(|s| s.id == "vm.snapshots.auto_max")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(10) as usize;
    let snap_manual_max = snap_settings.iter()
        .find(|s| s.id == "vm.snapshots.manual_max")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(12) as usize;
    let snap_interval = snap_settings.iter()
        .find(|s| s.id == "vm.snapshots.auto_interval")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(300) as u64;

    let scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session_dir.to_path_buf(),
        snap_auto_max,
        snap_manual_max,
        Duration::from_secs(snap_interval),
    );
    let scheduler = Arc::new(tokio::sync::Mutex::new(scheduler));

    // Take initial snapshot and log it to the session DB.
    {
        let mut s = scheduler.lock().await;
        match s.take_snapshot() {
            Ok(slot) => {
                let stop_id = query_max_fs_event_id(&config.db);
                config.db.write(capsem_logger::WriteOp::SnapshotEvent(
                    capsem_logger::SnapshotEvent {
                        timestamp: slot.timestamp,
                        slot: slot.slot,
                        origin: "auto".into(),
                        name: None,
                        files_count: slot.files_count,
                        start_fs_event_id: 0,
                        stop_fs_event_id: stop_id,
                    },
                )).await;
            }
            Err(e) => warn!("failed to take initial snapshot: {e}"),
        }
    }

    let config_ptr = Arc::as_ptr(config) as *mut McpGatewayConfig;
    unsafe {
        (*config_ptr).auto_snapshots = Some(Arc::clone(&scheduler));
    }

    let interval = Duration::from_secs(snap_interval);
    Some((scheduler, interval))
}

/// Query the current MAX(id) from fs_events, or 0 if empty/error.
fn query_max_fs_event_id(db: &capsem_logger::DbWriter) -> i64 {
    db.reader().ok()
        .and_then(|r| r.query_raw("SELECT COALESCE(MAX(id),0) FROM fs_events").ok())
        .and_then(|json| {
            let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
            parsed["rows"].get(0)?.get(0)?.as_i64()
        })
        .unwrap_or(0)
}

/// Spawn a periodic auto-snapshot timer that takes a snapshot every `interval`.
///
/// Snapshot creation does blocking I/O (directory cloning, walkdir, blake3 hashing)
/// so it runs on a spawn_blocking thread to avoid starving the tokio runtime.
pub(crate) fn spawn_auto_snapshot_timer(
    rt: &tokio::runtime::Handle,
    scheduler: Arc<tokio::sync::Mutex<capsem_core::auto_snapshot::AutoSnapshotScheduler>>,
    interval: Duration,
    db: Arc<capsem_logger::DbWriter>,
) {
    // Initialize the fs_event boundary from the DB.
    let initial_stop = query_max_fs_event_id(&db);

    rt.spawn(async move {
        let mut last_stop = initial_stop;
        let mut tick = tokio::time::interval(interval);
        tick.tick().await;
        loop {
            tick.tick().await;
            let sched = Arc::clone(&scheduler);
            let result = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    let mut s = sched.lock().await;
                    s.take_snapshot()
                })
            }).await;
            match result {
                Ok(Ok(slot)) => {
                    let stop_id = query_max_fs_event_id(&db);
                    db.write(capsem_logger::WriteOp::SnapshotEvent(
                        capsem_logger::SnapshotEvent {
                            timestamp: slot.timestamp,
                            slot: slot.slot,
                            origin: "auto".into(),
                            name: None,
                            files_count: slot.files_count,
                            start_fs_event_id: last_stop,
                            stop_fs_event_id: stop_id,
                        },
                    )).await;
                    last_stop = stop_id;
                }
                Ok(Err(e)) => tracing::warn!("auto-snapshot failed: {e}"),
                Err(e) => tracing::warn!("auto-snapshot task panicked: {e}"),
            }
        }
    });
}

/// Set up vsock listeners and handle connections after VM boot.
pub(crate) async fn setup_vsock(
    app_handle: tauri::AppHandle,
    mut vsock_rx: tokio::sync::mpsc::UnboundedReceiver<VsockConnection>,
    serial_task: tauri::async_runtime::JoinHandle<()>,
) {
    let mut terminal_conn = None;
    let mut control_conn = None;
    let mut deferred_conns = Vec::new();

    while terminal_conn.is_none() || control_conn.is_none() {
        match vsock_rx.recv().await {
            Some(conn) => {
                info!(port = conn.port, fd = conn.fd, "vsock: accepted connection");
                match conn.port {
                    VSOCK_PORT_TERMINAL => terminal_conn = Some(conn),
                    VSOCK_PORT_CONTROL => control_conn = Some(conn),
                    VSOCK_PORT_SNI_PROXY | VSOCK_PORT_MCP_GATEWAY => {
                        info!("vsock: port {} connection before terminal/control ready, deferring", conn.port);
                        deferred_conns.push(conn);
                    }
                    other => warn!("vsock: unexpected port {other}, ignoring"),
                }
            }
            None => {
                warn!("vsock: manager channel closed before all connections established");
                return;
            }
        }
    }

    let terminal = terminal_conn.unwrap();
    let control = control_conn.unwrap();

    // Transition: Booting -> VsockConnected
    {
        let state = app_handle.state::<AppState>();
        let vm_id = state.active_session_id.lock().unwrap().clone();
        if let Some(ref id) = vm_id {
            let mut vms = state.vms.lock().unwrap();
            if let Some(instance) = vms.get_mut(id) {
                if let Err(e) = instance.state_machine.transition(HostState::VsockConnected, "vsock_ports_connected") {
                    warn!("state machine: {e}");
                }
            }
        }
    }

    info!("vsock: both channels connected, performing boot handshake");

    let mut ctrl_file = match clone_fd(control.fd) {
        Ok(f) => f,
        Err(e) => {
            warn!("vsock: failed to clone control fd: {e}");
            return;
        }
    };

    match read_control_msg(&mut ctrl_file) {
        Ok(GuestToHost::Ready { version }) => {
            info!("vsock: guest agent ready (version {version})");
            let state = app_handle.state::<AppState>();
            let vm_id = state.active_session_id.lock().unwrap().clone();
            if let Some(ref id) = vm_id {
                let mut vms = state.vms.lock().unwrap();
                if let Some(instance) = vms.get_mut(id) {
                    if let Err(e) = instance.state_machine.transition(HostState::Handshaking, "ready_received") {
                        warn!("state machine: {e}");
                    }
                }
            }
        }
        Ok(other) => {
            warn!("vsock: expected Ready, got {other:?}");
        }
        Err(e) => {
            warn!("vsock: failed to read Ready: {e}");
        }
    }

    if let Err(e) = send_boot_config(&mut ctrl_file, &[]) {
        warn!("vsock: failed to send boot config: {e}");
    }

    let boot_ready_deadline = Instant::now() + Duration::from_secs(5);
    let mut boot_ready_received = false;
    while Instant::now() < boot_ready_deadline {
        match read_control_msg(&mut ctrl_file) {
            Ok(GuestToHost::BootReady) => {
                info!("vsock: guest boot ready");
                boot_ready_received = true;
                break;
            }
            Ok(other) => {
                info!("vsock: control message during boot handshake: {other:?}");
            }
            Err(e) => {
                warn!("vsock: control channel error during boot handshake: {e}");
                break;
            }
        }
    }
    if !boot_ready_received {
        warn!("vsock: BootReady not received within 5s, proceeding anyway");
    }

    serial_task.abort();
    info!("vsock: boot handshake complete, stopping serial forwarding");

    // Store vsock fds and transition to Running.
    let (mitm_config, mcp_config) = {
        let state = app_handle.state::<AppState>();
        let vm_id = state.active_session_id.lock().unwrap().clone();
        let mut vms = state.vms.lock().unwrap();
        if let Some(instance) = vm_id.as_ref().and_then(|id| vms.get_mut(id)) {
            instance.vsock_terminal_fd = Some(terminal.fd);
            instance.vsock_control_fd = Some(control.fd);
            if let Err(e) = instance.state_machine.transition(HostState::Running, "boot_ready_received") {
                warn!("state machine: {e}");
            }
            write_perf_log(&instance.state_machine);
            let mitm = instance.net_state.as_ref().map(|ns| {
                Arc::new(MitmProxyConfig {
                    ca: Arc::clone(&ns.ca),
                    policy: Arc::clone(&ns.policy),
                    db: Arc::clone(&ns.db),
                    upstream_tls: Arc::clone(&ns.upstream_tls),
                    pricing: capsem_core::net::ai_traffic::pricing::PricingTable::load(),
                    trace_state: std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new()),
                })
            });
            let mcp = instance.mcp_state.clone();
            (mitm, mcp)
        } else {
            (None, None)
        }
    };

    let _ = app_handle.emit("vm-state-changed", serde_json::json!({
        "state": "Running",
        "trigger": "boot_ready_received",
    }));
    let _ = app_handle.emit("terminal-source-changed", "vsock");

    if let Some(ref config) = mcp_config {
        let state = app_handle.state::<AppState>();
        let vm_id = state.active_session_id.lock().unwrap().clone();
        if let Some(session_dir) = vm_id.as_deref().and_then(session_dir_for) {
            if let Some((scheduler, interval)) = wire_auto_snapshots(config, &session_dir).await {
                let db = Arc::clone(&config.db);
                let handle = tokio::runtime::Handle::current();
                spawn_auto_snapshot_timer(&handle, scheduler, interval, db);
            }
        }
    }

    let terminal_output = {
        let state = app_handle.state::<AppState>();
        Arc::clone(&state.terminal_output)
    };
    tokio::spawn(vsock_terminal_to_events(terminal_output, terminal.fd));
    tokio::spawn(vsock_control_handler(app_handle.clone(), control.fd));

    // Spawn periodic flush task.
    {
        let flush_handle = app_handle.clone();
        let state = app_handle.state::<AppState>();
        let session_id = state.active_session_id.lock().unwrap().clone();
        let db = {
            let vms = state.vms.lock().unwrap();
            session_id.as_ref()
                .and_then(|id| vms.get(id))
                .and_then(|i| i.net_state.as_ref())
                .map(|ns| Arc::clone(&ns.db))
        };
        if let (Some(sid), Some(db)) = (session_id, db) {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.tick().await;
                let mut tick_count: u64 = 0;
                loop {
                    interval.tick().await;
                    tick_count += 1;
                    let sid = sid.clone();
                    let db = Arc::clone(&db);
                    let flush_handle = flush_handle.clone();
                    let checkpoint_main = tick_count.is_multiple_of(10);
                    let _ = tokio::task::spawn_blocking(move || {
                        use tauri::Manager;
                        let reader = match db.reader() {
                            Ok(r) => r,
                            Err(_) => return,
                        };
                        let state = flush_handle.state::<AppState>();
                        let idx = match state.session_index.lock() {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        if let Ok(counts) = reader.net_event_counts() {
                            let _ = idx.update_request_counts(
                                &sid,
                                counts.total as u64,
                                counts.allowed as u64,
                                counts.denied as u64,
                            );
                        }
                        flush_session_summary(&sid, &idx, &reader);
                        if checkpoint_main {
                            let _ = idx.checkpoint();
                        }
                    }).await;
                }
            });
        }
    }

    let _keep_terminal = terminal;
    let _keep_control = control;

    // Process deferred connections.
    for conn in deferred_conns {
        match conn.port {
            VSOCK_PORT_SNI_PROXY => {
                if let Some(ref config) = mitm_config {
                    let fd = conn.fd;
                    let config = Arc::clone(config);
                    tokio::spawn(async move {
                        let _conn = conn;
                        mitm_proxy::handle_connection(fd, config).await;
                    });
                }
            }
            VSOCK_PORT_MCP_GATEWAY => {
                if let Some(ref mcp) = mcp_config {
                    let fd = conn.fd;
                    let mcp = Arc::clone(mcp);
                    tokio::spawn(async move {
                        let _conn = conn;
                        gateway::serve_mcp_session(fd, mcp).await;
                    });
                }
            }
            _ => {}
        }
    }

    // Accept MITM proxy + MCP gateway connections indefinitely.
    info!("vsock: listening for proxy connections on ports 5002/5003");
    loop {
        match vsock_rx.recv().await {
            Some(conn) if conn.port == VSOCK_PORT_SNI_PROXY => {
                if let Some(ref config) = mitm_config {
                    let fd = conn.fd;
                    let config = Arc::clone(config);
                    tokio::spawn(async move {
                        let _conn = conn;
                        mitm_proxy::handle_connection(fd, config).await;
                    });
                } else {
                    warn!("vsock: SNI proxy connection rejected (no config)");
                }
            }
            Some(conn) if conn.port == VSOCK_PORT_MCP_GATEWAY => {
                if let Some(ref mcp) = mcp_config {
                    let fd = conn.fd;
                    let mcp = Arc::clone(mcp);
                    tokio::spawn(async move {
                        let _conn = conn;
                        gateway::serve_mcp_session(fd, mcp).await;
                    });
                } else {
                    warn!("vsock: MCP connection rejected (no config)");
                }
            }
            Some(conn) => {
                warn!(port = conn.port, "vsock: unexpected port after setup, ignoring");
            }
            None => {
                info!("vsock: manager channel closed, stopping accept loop");
                break;
            }
        }
    }
}
