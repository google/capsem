#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_core::{
    CoalesceBuffer, GuestToHost, HostState, HostStateMachine, HostToGuest, VirtualMachine,
    VmConfig, VsockManager, VSOCK_PORT_CONTROL, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
    create_scratch_disk, decode_guest_msg, encode_host_msg, validate_guest_msg, MAX_FRAME_SIZE,
};
use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy_config;
use capsem_core::net::telemetry::WebDb;
use capsem_core::session::{self, SessionIndex, SessionRecord};
use state::{AppState, VmInstance, VmNetworkState};
use tauri::{Emitter, Manager};
use tokio::sync::broadcast;
use tracing::{debug_span, error, info, info_span, warn};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

/// Clone a raw fd into an independently-owned File.
/// The original fd remains open and unaffected.
pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    // Safety: fd is valid (checked by caller context)
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone() // creates a dup'd fd owned by the returned File
}

/// Find the assets directory containing kernel, initrd, and rootfs.
///
/// Checks (in order):
/// 1. `CAPSEM_ASSETS_DIR` env var (development override)
/// 2. macOS .app bundle: `Contents/Resources/` (sibling of `Contents/MacOS/`)
/// 3. `./assets` (workspace root, for `cargo run`)
/// 4. `../../assets` (when CWD is `crates/capsem-app/`)
fn resolve_assets_dir() -> Result<PathBuf> {
    let _span = debug_span!("resolve_assets").entered();
    // 1. Explicit env var (development override)
    if let Ok(dir) = std::env::var("CAPSEM_ASSETS_DIR") {
        let p = PathBuf::from(dir);
        if p.join("vmlinuz").exists() {
            return Ok(p);
        }
    }

    // 2. macOS .app bundle: Contents/Resources/ (sibling of Contents/MacOS/)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            let resources = macos_dir.parent().map(|p| p.join("Resources"));
            if let Some(ref res) = resources {
                if res.join("vmlinuz").exists() {
                    return Ok(res.clone());
                }
            }
        }
    }

    // 3. ./assets (workspace root, for `cargo run`)
    let cwd_assets = PathBuf::from("assets");
    if cwd_assets.join("vmlinuz").exists() {
        return Ok(cwd_assets);
    }

    // 4. ../../assets (when CWD is crates/capsem-app/)
    let parent_assets = PathBuf::from("../../assets");
    if parent_assets.join("vmlinuz").exists() {
        return Ok(parent_assets);
    }

    Err(anyhow::anyhow!(
        "VM assets not found. Set CAPSEM_ASSETS_DIR or run from workspace root."
    ))
}

/// Write boot performance data from the state machine to ~/.capsem/perf/<timestamp>.log
fn write_perf_log(sm: &HostStateMachine) {
    let log = sm.format_perf_log();
    if log.is_empty() {
        return;
    }
    eprint!("{log}");
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return,
    };
    let dir = home.join(".capsem").join("perf");
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(format!("{ts}.log"));
    let _ = std::fs::write(&path, &log);
    eprintln!("perf log: {}", path.display());
}

/// Get the sessions base directory: ~/.capsem/sessions/
fn sessions_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| {
        PathBuf::from(h).join(".capsem").join("sessions")
    })
}

/// Get the session directory for a specific VM: ~/.capsem/sessions/<vm_id>/
fn session_dir_for(vm_id: &str) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(vm_id))
}

/// Clean up stale sessions on app startup using SessionIndex.
///
/// Deletes any leftover scratch.img files (always ephemeral) and marks
/// any "running" sessions as "crashed" (stale from ungraceful exit).
/// Also runs age-based, count-based, and disk-based culling.
fn cleanup_stale_sessions(index: &SessionIndex) {
    let base = match sessions_dir() {
        Some(d) => d,
        None => return,
    };

    // Delete leftover scratch.img files from all session dirs.
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let scratch = dir.join("scratch.img");
            if scratch.exists() {
                info!(path = %scratch.display(), "deleting stale scratch.img");
                let _ = std::fs::remove_file(&scratch);
            }
        }
    }

    // Mark stale "running" sessions as "crashed" in main.db.
    match index.mark_running_as_crashed() {
        Ok(0) => {}
        Ok(n) => info!(count = n, "marked stale sessions as crashed"),
        Err(e) => warn!("failed to mark stale sessions: {e}"),
    }

    // Age-based culling.
    let settings = policy_config::load_merged_settings();
    let retention_days = settings.iter()
        .find(|s| s.id == "session.retention_days")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(30) as u32;
    let max_sessions = settings.iter()
        .find(|s| s.id == "session.max_sessions")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(100) as usize;
    let max_disk_gb = settings.iter()
        .find(|s| s.id == "session.max_disk_gb")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(100) as u64;

    if let Ok(n) = index.delete_older_than_days(retention_days) {
        if n > 0 {
            info!(count = n, "culled old sessions (>{retention_days} days)");
        }
    }
    if let Ok(n) = index.delete_keeping_newest(max_sessions) {
        if n > 0 {
            info!(count = n, "culled sessions over cap ({max_sessions})");
        }
    }

    // Disk-based culling.
    let max_disk_bytes = max_disk_gb * 1024 * 1024 * 1024;
    let mut usage = session::disk_usage_bytes(&base);
    if usage > max_disk_bytes {
        if let Ok(stopped) = index.stopped_sessions_oldest_first() {
            for rec in stopped {
                if usage <= max_disk_bytes {
                    break;
                }
                let dir = base.join(&rec.id);
                if dir.is_dir() {
                    let dir_bytes = session::disk_usage_bytes(&dir);
                    if let Err(e) = std::fs::remove_dir_all(&dir) {
                        warn!(id = %rec.id, "failed to remove session dir: {e}");
                        continue;
                    }
                    usage = usage.saturating_sub(dir_bytes);
                    info!(id = %rec.id, "culled session dir for disk budget");
                }
            }
        }
    }

    // Remove orphan session dirs that no longer have a DB record.
    if let Ok(entries) = std::fs::read_dir(&base) {
        let known_ids: std::collections::HashSet<String> = index
            .recent(10_000)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.id)
            .collect();
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = match dir.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !session::is_valid_session_id(&name) {
                continue;
            }
            if !known_ids.contains(&name) {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    warn!(id = %name, "failed to remove orphan session dir: {e}");
                } else {
                    info!(id = %name, "removed orphan session dir");
                }
            }
        }
    }
}

/// Clean up a VM session: delete scratch.img, snapshot request counts, update status.
fn cleanup_session(
    session_dir: &Path,
    scratch_path: Option<&Path>,
    session_id: &str,
    index: &SessionIndex,
    web_db: Option<&std::sync::Mutex<WebDb>>,
) {
    if let Some(scratch) = scratch_path {
        if scratch.exists() {
            info!(path = %scratch.display(), "deleting scratch.img");
            if let Err(e) = std::fs::remove_file(scratch) {
                warn!("failed to delete scratch.img: {e}");
            }
        }
    }

    // Snapshot request counts.
    if let Some(db_lock) = web_db {
        if let Ok(db) = db_lock.lock() {
            if let Ok((total, allowed, denied)) = db.count_by_decision() {
                let _ = index.update_request_counts(
                    session_id,
                    total as u64,
                    allowed as u64,
                    denied as u64,
                );
            }
        }
    }

    let _ = index.update_status(session_id, "stopped", Some(&session::now_iso()));
}

/// Static CA keypair embedded at compile time.
const CA_KEY_PEM: &str = include_str!("../../../config/capsem-ca.key");
const CA_CERT_PEM: &str = include_str!("../../../config/capsem-ca.crt");

/// Create per-VM network state: load CA, network policy, and open web.db.
fn create_net_state(vm_id: &str) -> Result<VmNetworkState> {
    let ca = CertAuthority::load(CA_KEY_PEM, CA_CERT_PEM)
        .context("failed to load MITM CA")?;
    info!(vm_id, "loaded MITM CA");

    let policy = policy_config::load_merged_network_policy();
    info!(
        vm_id,
        "loaded network policy ({} rules)",
        policy.rules.len()
    );

    // Session directory: ~/.capsem/sessions/<vm_id>/
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let session_dir = PathBuf::from(home)
        .join(".capsem")
        .join("sessions")
        .join(vm_id);
    let db_path = session_dir.join("info.db");
    let web_db = WebDb::open(&db_path).context("failed to open telemetry db")?;
    info!(path = %db_path.display(), "opened telemetry db");

    Ok(VmNetworkState {
        policy: Arc::new(std::sync::RwLock::new(Arc::new(policy))),
        web_db: Arc::new(Mutex::new(web_db)),
        ca: Arc::new(ca),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
    })
}

/// Build config, create VM, start it, and return the VM + serial receiver + input fd + state machine.
///
/// If `scratch_disk_path` is provided, the scratch disk is attached as a second
/// block device (read-write) for the guest `/root` workspace.
fn boot_vm(
    assets: &Path,
    cmdline: &str,
    scratch_disk_path: Option<&Path>,
) -> Result<(VirtualMachine, broadcast::Receiver<Vec<u8>>, RawFd, HostStateMachine)> {
    let _span = info_span!("boot_vm").entered();
    let mut sm = HostStateMachine::new_host();

    let config = {
        let _span = debug_span!("config_build").entered();
        let mut builder = VmConfig::builder()
            .cpu_count(2)
            .ram_bytes(512 * 1024 * 1024)
            .kernel_path(assets.join("vmlinuz"))
            .kernel_cmdline(cmdline);

        if let Some(hash) = option_env!("VMLINUZ_HASH") {
            builder = builder.expected_kernel_hash(hash);
        }

        if assets.join("initrd.img").exists() {
            builder = builder.initrd_path(assets.join("initrd.img"));
            if let Some(hash) = option_env!("INITRD_HASH") {
                builder = builder.expected_initrd_hash(hash);
            }
        }

        if assets.join("rootfs.img").exists() {
            builder = builder.disk_path(assets.join("rootfs.img"));
            if let Some(hash) = option_env!("ROOTFS_HASH") {
                builder = builder.expected_disk_hash(hash);
            }
        }

        if let Some(scratch) = scratch_disk_path {
            builder = builder.scratch_disk_path(scratch);
        }

        builder.build().context("failed to build VmConfig")?
    };

    let (mut vm, rx, input_fd) = {
        let _span = debug_span!("vm_create").entered();
        VirtualMachine::create(&config).context("failed to create VM")?
    };

    {
        let _span = debug_span!("vm_start").entered();
        vm.start().context("failed to start VM")?;
    }

    sm.transition(HostState::Booting, "vm_started")?;

    Ok((vm, rx, input_fd, sm))
}

/// Forward serial console bytes to the Tauri frontend as events.
async fn serial_to_events(
    app_handle: tauri::AppHandle,
    mut rx: broadcast::Receiver<Vec<u8>>,
) {
    loop {
        match rx.recv().await {
            Ok(bytes) => {
                if let Err(e) = app_handle.emit("serial-output", &bytes) {
                    error!("failed to emit serial-output event: {e}");
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("serial broadcast channel closed");
                break;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                info!("serial receiver lagged by {n} messages");
            }
        }
    }
}

/// Forward vsock terminal data to the frontend with coalescing.
///
/// Reads raw bytes from the vsock fd in a blocking thread, then emits them
/// to the frontend. Coalesces output using `CoalesceBuffer` (8ms window,
/// 64KB cap) to prevent IPC saturation on high-throughput commands.
async fn vsock_terminal_to_events(app_handle: tauri::AppHandle, vsock_fd: RawFd) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

    // Blocking reader thread: vsock fd -> channel
    std::thread::spawn(move || {
        let mut file = match clone_fd(vsock_fd) {
            Ok(f) => f,
            Err(e) => {
                error!("vsock terminal: failed to clone fd: {e}");
                return;
            }
        };
        let mut buf = [0u8; 8192];
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
        // Wait for the first chunk.
        match rx.recv().await {
            Some(chunk) => { coalesce.push(&chunk); }
            None => break,
        }

        // Coalesce additional chunks within the time window or until size cap.
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(coalesce.window_ms());
        while !coalesce.is_full() {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(chunk)) => { coalesce.push(&chunk); }
                _ => break,
            }
        }

        coalesce.flush_to(|batch| {
            if let Err(e) = app_handle.emit("serial-output", batch) {
                error!("failed to emit vsock terminal data: {e}");
            }
        });
    }
}

/// Handle vsock control channel: read incoming messages, handle heartbeat.
/// Called AFTER the boot handshake (Ready/BootConfig/BootReady already consumed).
/// Validates each incoming message against the host state machine before processing.
async fn vsock_control_handler(app_handle: tauri::AppHandle, control_fd: RawFd) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuestToHost>(32);

    // Blocking reader thread for control messages.
    std::thread::spawn(move || {
        let mut file = match clone_fd(control_fd) {
            Ok(f) => f,
            Err(e) => {
                warn!("vsock control: failed to clone fd: {e}");
                return;
            }
        };
        loop {
            // Read length prefix.
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

    while let Some(msg) = rx.recv().await {
        // Validate incoming guest message against host state machine.
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
            other => {
                info!("vsock: unhandled control message: {other:?}");
            }
        }
    }
}

/// Set up vsock listeners and handle connections after VM boot.
///
/// Once vsock connects, the serial forwarding task is aborted since all
/// terminal I/O now flows through the vsock PTY bridge. After terminal
/// and control are established, continues accepting port 5002 (SNI proxy)
/// connections indefinitely, spawning each into a blocking thread.
async fn setup_vsock(
    app_handle: tauri::AppHandle,
    mut vsock_manager: VsockManager,
    serial_task: tauri::async_runtime::JoinHandle<()>,
) {
    // Wait for both terminal and control connections from the guest agent.
    let mut terminal_conn = None;
    let mut control_conn = None;

    while terminal_conn.is_none() || control_conn.is_none() {
        match vsock_manager.accept().await {
            Some(conn) => {
                info!(port = conn.port, fd = conn.fd, "vsock: accepted connection");
                match conn.port {
                    VSOCK_PORT_TERMINAL => terminal_conn = Some(conn),
                    VSOCK_PORT_CONTROL => control_conn = Some(conn),
                    VSOCK_PORT_SNI_PROXY => {
                        info!("vsock: SNI proxy connection before terminal/control ready, deferring");
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

    // Boot handshake: wait for Ready, send BootConfig, wait for BootReady.
    // Read first control message -- expect GuestToHost::Ready.
    match read_control_msg(control.fd) {
        Ok(GuestToHost::Ready { version }) => {
            info!("vsock: guest agent ready (version {version})");
            // Transition: VsockConnected -> Handshaking
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

    // Send boot config as individual messages.
    if let Err(e) = send_boot_config(control.fd, &[]) {
        warn!("vsock: failed to send boot config: {e}");
    }

    // Wait for BootReady.
    let boot_ready_deadline = Instant::now() + Duration::from_secs(5);
    let mut boot_ready_received = false;
    while Instant::now() < boot_ready_deadline {
        match read_control_msg(control.fd) {
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
    let mitm_config = {
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
            instance.net_state.as_ref().map(|ns| {
                Arc::new(MitmProxyConfig {
                    ca: Arc::clone(&ns.ca),
                    policy: Arc::clone(&ns.policy),
                    web_db: Arc::clone(&ns.web_db),
                    upstream_tls: Arc::clone(&ns.upstream_tls),
                })
            })
        } else {
            None
        }
    };

    // Emit structured state change to frontend.
    let _ = app_handle.emit("vm-state-changed", serde_json::json!({
        "state": "Running",
        "trigger": "boot_ready_received",
    }));
    let _ = app_handle.emit("terminal-source-changed", "vsock");

    // Spawn forwarding tasks.
    let handle1 = app_handle.clone();
    tokio::spawn(vsock_terminal_to_events(handle1, terminal.fd));
    tokio::spawn(vsock_control_handler(app_handle, control.fd));

    // Keep terminal/control connections alive.
    let _keep_terminal = terminal;
    let _keep_control = control;

    // Accept MITM proxy connections indefinitely on port 5002.
    if let Some(config) = mitm_config {
        info!("vsock: listening for MITM proxy connections on port 5002");
        loop {
            match vsock_manager.accept().await {
                Some(conn) if conn.port == VSOCK_PORT_SNI_PROXY => {
                    let fd = conn.fd;
                    let config = Arc::clone(&config);
                    tokio::spawn(async move {
                        let _conn = conn; // keep VsockConnection alive
                        mitm_proxy::handle_connection(fd, config).await;
                    });
                }
                Some(conn) => {
                    warn!(port = conn.port, "vsock: unexpected port after setup, ignoring");
                }
                None => {
                    info!("vsock: manager channel closed, stopping MITM proxy accept loop");
                    break;
                }
            }
        }
    } else {
        warn!("vsock: no network state, MITM proxy disabled");
        // Wait forever (connections are long-lived).
        std::future::pending::<()>().await;
    }
}

const CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// Read exactly `n` bytes from a raw fd, retrying on partial reads.
fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> std::io::Result<()> {
    let mut file = clone_fd(fd)?;
    let mut pos = 0;
    while pos < buf.len() {
        let n = file.read(&mut buf[pos..])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF"));
        }
        pos += n;
    }
    Ok(())
}

/// Write all bytes to a raw fd.
fn write_all_fd(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    let mut file = clone_fd(fd)?;
    file.write_all(data)?;
    Ok(())
}

/// Read one guest-to-host control message from an fd (blocking).
fn read_control_msg(fd: RawFd) -> Result<GuestToHost> {
    let mut len_buf = [0u8; 4];
    read_exact_fd(fd, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE as usize {
        anyhow::bail!("control frame too large ({len} bytes)");
    }
    let mut payload = vec![0u8; len];
    read_exact_fd(fd, &mut payload)?;
    decode_guest_msg(&payload)}

/// Write one host-to-guest control message to an fd.
fn write_control_msg(fd: RawFd, msg: &HostToGuest) -> Result<()> {
    let frame = encode_host_msg(msg)?;
    write_all_fd(fd, &frame)?;
    Ok(())
}

/// Send the boot configuration as individual vsock messages.
///
/// Sends BootConfig (clock), then SetEnv for each env var, FileWrite for each
/// boot file, and BootConfigDone to signal completion. Each message is its own
/// frame, eliminating the old single-frame size constraint.
///
/// Validates all env vars and file paths before sending. Invalid entries are
/// logged and skipped. Enforces allocation caps (MAX_BOOT_ENV_VARS,
/// MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES) to prevent unbounded allocations.
///
/// Env var priority: settings registry defaults < user.toml overrides < CLI --env flags.
fn send_boot_config(control_fd: RawFd, cli_env: &[(String, String)]) -> Result<()> {
    use capsem_core::capsem_proto::{
        validate_env_key, validate_env_value, validate_file_path,
        MAX_BOOT_ENV_VARS, MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES,
    };

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 1. Send BootConfig with clock.
    write_control_msg(control_fd, &HostToGuest::BootConfig { epoch_secs })?;

    // 2. Send metadata-driven env vars from settings registry.
    let guest_config = policy_config::load_merged_guest_config();
    let mut env_count: usize = 0;

    if let Some(env) = guest_config.env {
        for (key, value) in env {
            if env_count >= MAX_BOOT_ENV_VARS {
                warn!("boot env var cap reached ({MAX_BOOT_ENV_VARS}), skipping remaining");
                break;
            }
            if let Err(e) = validate_env_key(&key) {
                warn!("skipping invalid boot env var key: {e}");
                continue;
            }
            if let Err(e) = validate_env_value(&value) {
                warn!("skipping boot env var {key}: {e}");
                continue;
            }
            write_control_msg(control_fd, &HostToGuest::SetEnv { key, value })?;
            env_count += 1;
        }
    }

    // 3. CLI --env overrides (last wins).
    for (key, value) in cli_env {
        if env_count >= MAX_BOOT_ENV_VARS {
            warn!("boot env var cap reached ({MAX_BOOT_ENV_VARS}), skipping remaining CLI --env");
            break;
        }
        if let Err(e) = validate_env_key(key) {
            warn!("skipping invalid CLI --env key: {e}");
            continue;
        }
        if let Err(e) = validate_env_value(value) {
            warn!("skipping CLI --env {key}: {e}");
            continue;
        }
        write_control_msg(
            control_fd,
            &HostToGuest::SetEnv {
                key: key.clone(),
                value: value.clone(),
            },
        )?;
        env_count += 1;
    }

    // 4. Send each boot file (with caps).
    let mut file_count: usize = 0;
    let mut total_file_bytes: usize = 0;

    for file in guest_config.files.unwrap_or_default() {
        if file_count >= MAX_BOOT_FILES {
            warn!("boot file cap reached ({MAX_BOOT_FILES}), skipping remaining");
            break;
        }
        let data = file.content.into_bytes();
        if total_file_bytes + data.len() > MAX_BOOT_FILE_BYTES {
            warn!(
                "boot file bytes cap reached ({MAX_BOOT_FILE_BYTES}), skipping {}",
                file.path
            );
            continue;
        }
        if let Err(e) = validate_file_path(&file.path) {
            warn!("skipping invalid boot file path: {e}");
            continue;
        }
        total_file_bytes += data.len();
        file_count += 1;
        write_control_msg(
            control_fd,
            &HostToGuest::FileWrite {
                path: file.path,
                data,
                mode: file.mode,
            },
        )?;
    }

    // 5. Signal done.
    write_control_msg(control_fd, &HostToGuest::BootConfigDone)?;

    Ok(())
}

/// Parse `--env KEY=VALUE` pairs from CLI args, returning env pairs and remaining args.
///
/// CLI --env args are validated strictly: invalid keys or values cause an error
/// message and the pair is skipped (stricter than config file handling).
fn parse_env_args(args: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    use capsem_core::capsem_proto::{validate_env_key, validate_env_value};

    let mut env_pairs = Vec::new();
    let mut remaining = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--env" {
            if let Some(val) = iter.next() {
                if let Some((key, value)) = val.split_once('=') {
                    if let Err(e) = validate_env_key(key) {
                        eprintln!("capsem: --env rejected: {e}");
                        continue;
                    }
                    if let Err(e) = validate_env_value(value) {
                        eprintln!("capsem: --env {key} rejected: {e}");
                        continue;
                    }
                    env_pairs.push((key.to_string(), value.to_string()));
                } else {
                    eprintln!("capsem: --env value must be KEY=VALUE, got: {val}");
                }
            } else {
                eprintln!("capsem: --env requires a KEY=VALUE argument");
            }
        } else if let Some(rest) = arg.strip_prefix("--env=") {
            if let Some((key, value)) = rest.split_once('=') {
                if let Err(e) = validate_env_key(key) {
                    eprintln!("capsem: --env rejected: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(value) {
                    eprintln!("capsem: --env {key} rejected: {e}");
                    continue;
                }
                env_pairs.push((key.to_string(), value.to_string()));
            } else {
                eprintln!("capsem: --env value must be KEY=VALUE, got: {rest}");
            }
        } else {
            remaining.push(arg.clone());
        }
    }
    (env_pairs, remaining)
}

/// Start the VM in CLI mode and execute a command.
///
/// **Architecture & CFRunLoop:**
/// This function runs entirely on the main thread and uses synchronous blocking I/O
/// combined with manual `CFRunLoop` pumping. The Virtualization.framework (VZ) heavily
/// relies on GCD and the main thread's run loop to dispatch events, handle vsock
/// connections, and manage VM state transitions. If we block the main thread
/// (e.g., by waiting on a channel or reading from a socket without pumping the run loop),
/// VZ will deadlock and vsock connections will never arrive.
///
/// To solve this, `run_cli` uses `CFRunLoopRunInMode` with a short timeout (50ms)
/// to yield control back to VZ, allowing it to process events. We then check for
/// incoming messages or vsock connections using non-blocking/try_recv methods.
///
/// **Limitations:**
/// - Cannot use `tokio::main` or `async` on the main thread because tokio's reactor
///   does not pump `CFRunLoop`.
/// - Requires manual polling loops for control messages.
fn run_cli(command: &str, cli_env: &[(String, String)], session_index: &SessionIndex) -> Result<()> {
    // Tokio runtime for async MITM proxy handlers.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("failed to create tokio runtime")?;

    let assets = resolve_assets_dir()?;

    // Generate unique session ID.
    let cli_session_id = session::generate_session_id();
    eprintln!("[capsem] session: {cli_session_id}");

    // Create session directory and scratch disk for CLI mode.
    let vm_settings = policy_config::load_merged_vm_settings();
    let scratch_size = vm_settings.scratch_disk_size_gb.unwrap_or(8);
    let ram_bytes: u64 = 512 * 1024 * 1024;
    let cli_session_dir = session_dir_for(&cli_session_id);
    let scratch_path = cli_session_dir.as_ref().and_then(|d| {
        std::fs::create_dir_all(d).ok();
        let path = d.join("scratch.img");
        if let Err(e) = create_scratch_disk(&path, scratch_size) {
            warn!("failed to create scratch disk: {e}");
            return None;
        }
        info!(size_gb = scratch_size, "created scratch disk");
        Some(path)
    });

    // Record session in main.db.
    let record = SessionRecord {
        id: cli_session_id.clone(),
        mode: "cli".to_string(),
        command: Some(command.to_string()),
        status: "running".to_string(),
        created_at: session::now_iso(),
        stopped_at: None,
        scratch_disk_size_gb: scratch_size,
        ram_bytes,
        total_requests: 0,
        allowed_requests: 0,
        denied_requests: 0,
    };
    if let Err(e) = session_index.create_session(&record) {
        warn!("failed to record session: {e}");
    }

    let (vm, mut rx, _serial_input_fd, _sm) = boot_vm(
        &assets,
        "console=hvc0 ro loglevel=1",
        scratch_path.as_deref(),
    )?;

    // Set up vsock listeners (including SNI proxy port).
    let socket_devices = vm.socket_devices();
    let mut mgr = VsockManager::new(
        &socket_devices,
        &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL, VSOCK_PORT_SNI_PROXY],
    ).context("failed to set up vsock")?;

    // Create per-VM network state for MITM proxy.
    let net_state = create_net_state(&cli_session_id).ok();
    let mitm_config: Option<Arc<MitmProxyConfig>> = net_state.as_ref().map(|ns| {
        Arc::new(MitmProxyConfig {
            ca: Arc::clone(&ns.ca),
            policy: Arc::clone(&ns.policy),
            web_db: Arc::clone(&ns.web_db),
            upstream_tls: Arc::clone(&ns.upstream_tls),
        })
    });

    // Print serial boot logs to stderr in a background thread.
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(bytes) => {
                    let _ = std::io::stderr().write_all(&bytes);
                    let _ = std::io::stderr().flush();
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // Accept vsock connections with CFRunLoop pumping.
    // The VZ framework delivers connections via ObjC callbacks that require
    // CFRunLoop to be running on the main thread.
    let deadline = Instant::now() + CLI_TIMEOUT;
    let mut terminal_fd: Option<RawFd> = None;
    let mut control_fd: Option<RawFd> = None;
    let mut _conns = Vec::new(); // Keep connections alive.

    let setup_start = Instant::now();
    let mut warned_setup = false;

    while terminal_fd.is_none() || control_fd.is_none() {
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for vsock connections from guest agent");
        }
        if !warned_setup && setup_start.elapsed() > Duration::from_secs(30) {
            eprintln!("[capsem] warning: no vsock connections after 30s. Is the guest agent running?");
            warned_setup = true;
        }
        // Pump CFRunLoop to deliver ObjC callbacks.
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        // Check for accepted connections (non-blocking via try_recv on the channel).
        while let Ok(conn) = mgr.try_accept() {
            match conn.port {
                VSOCK_PORT_TERMINAL => terminal_fd = Some(conn.fd),
                VSOCK_PORT_CONTROL => control_fd = Some(conn.fd),
                VSOCK_PORT_SNI_PROXY => {
                    // Spawn MITM proxy handler on the tokio runtime.
                    if let Some(ref config) = mitm_config {
                        let fd = conn.fd;
                        let config = Arc::clone(config);
                        rt.spawn(async move {
                            let _conn = conn;
                            mitm_proxy::handle_connection(fd, config).await;
                        });
                        continue; // conn moved, don't push to _conns
                    }
                }
                _ => {}
            }
            _conns.push(conn);
        }
    }

    let terminal_fd = terminal_fd.unwrap();
    let control_fd = control_fd.unwrap();

    // Wait for Ready message from guest agent.
    let (ctrl_msg_tx, ctrl_msg_rx) = std::sync::mpsc::channel::<GuestToHost>();
    let ctrl_fd_reader = control_fd;
    std::thread::spawn(move || {
        loop {
            match read_control_msg(ctrl_fd_reader) {
                Ok(msg) => {
                    if ctrl_msg_tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for Ready, pumping CFRunLoop.
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for guest agent Ready");
        }
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        match ctrl_msg_rx.try_recv() {
            Ok(GuestToHost::Ready { version }) => {
                eprintln!("[capsem] guest agent ready (v{version})");
                break;
            }
            Ok(other) => {
                eprintln!("[capsem] unexpected control message before Ready: {other:?}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("control channel closed before Ready");
            }
        }
    }

    // Send boot config as individual messages.
    send_boot_config(control_fd, cli_env)?;

    // Wait for BootReady.
    let boot_ready_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() >= boot_ready_deadline {
            eprintln!("[capsem] BootReady not received within 5s, proceeding");
            break;
        }
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        match ctrl_msg_rx.try_recv() {
            Ok(GuestToHost::BootReady) => {
                eprintln!("[capsem] guest boot ready");
                break;
            }
            Ok(other) => {
                eprintln!("[capsem] control message during boot: {other:?}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("guest agent disconnected during boot handshake");
            }
        }
    }

    // Send Exec command.
    let exec_id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    write_control_msg(control_fd, &HostToGuest::Exec {
        id: exec_id,
        command: command.to_string(),
    })?;

    // Stream terminal output from vsock to stdout in a background thread.
    // Track whether the last byte written was a newline so we can add one
    // before exiting if needed.
    let last_was_newline = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let lwn = last_was_newline.clone();
    let terminal_reader = std::thread::spawn(move || {
        let mut file = match clone_fd(terminal_fd) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[capsem] terminal reader failed to clone fd: {e}");
                return;
            }
        };
        let mut buf = [0u8; 8192];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = std::io::stdout().write_all(&buf[..n]);
                    let _ = std::io::stdout().flush();
                    lwn.store(buf[n - 1] == b'\n', std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
    });

    // Wait for ExecDone, pumping CFRunLoop and accepting SNI proxy connections.
    let exit_code;
    let mut last_msg_time = Instant::now();
    let mut warned_exec = false;
    loop {
        if Instant::now() >= deadline {
            eprintln!("[capsem] timed out waiting for command completion");
            exit_code = 124; // Same as `timeout` command.
            break;
        }
        if !warned_exec && last_msg_time.elapsed() > Duration::from_secs(30) {
            eprintln!("[capsem] warning: no control messages (heartbeats) for 30s. Guest may be hung.");
            warned_exec = true;
        }
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        // Accept any incoming MITM proxy connections during exec.
        while let Ok(conn) = mgr.try_accept() {
            if conn.port == VSOCK_PORT_SNI_PROXY {
                if let Some(ref config) = mitm_config {
                    let fd = conn.fd;
                    let config = Arc::clone(config);
                    rt.spawn(async move {
                        let _conn = conn;
                        mitm_proxy::handle_connection(fd, config).await;
                    });
                }
            } else {
                _conns.push(conn);
            }
        }
        match ctrl_msg_rx.try_recv() {
            Ok(GuestToHost::ExecDone { id, exit_code: code }) if id == exec_id => {
                exit_code = code;
                break;
            }
            Ok(GuestToHost::Pong) => {
                last_msg_time = Instant::now();
                warned_exec = false;
            }
            Ok(other) => {
                last_msg_time = Instant::now();
                warned_exec = false;
                eprintln!("[capsem] control message during exec: {other:?}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                eprintln!("[capsem] control channel closed during exec");
                exit_code = 1;
                break;
            }
        }
    }

    // Stop VM and drop connections (closes vsock fds, unblocks the reader).
    let _ = vm.stop();
    drop(_conns);
    // Wait for terminal reader to drain remaining output.
    let _ = terminal_reader.join();

    // Clean up session: delete scratch.img, snapshot counts, update status.
    if let Some(ref dir) = cli_session_dir {
        let web_db_arc = net_state.as_ref().map(|ns| Arc::clone(&ns.web_db));
        let web_db_ref = web_db_arc.as_deref();
        cleanup_session(dir, scratch_path.as_deref(), &cli_session_id, session_index, web_db_ref);
    }

    // Ensure the host shell prompt starts on a fresh line.
    if !last_was_newline.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = std::io::stdout().write_all(b"\n");
        let _ = std::io::stdout().flush();
    }
    std::process::exit(exit_code);
}

/// Check for app updates using Tauri's updater plugin.
/// Uses a native dialog (not the WebView) since the webview gets replaced with
/// VZVirtualMachineView after VM boot.
async fn check_for_update(app: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    use tauri_plugin_dialog::DialogExt;

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            info!("updater not available: {e:#}");
            return;
        }
    };

    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => {
            info!("no update available");
            return;
        }
        Err(e) => {
            info!("update check failed: {e:#}");
            return;
        }
    };

    let current_version = app.package_info().version.to_string();
    let accepted = app
        .dialog()
        .message(format!(
            "Capsem {} is available (you have {}). Download and install?",
            update.version, current_version
        ))
        .title("Update Available")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
        .blocking_show();

    if accepted {
        if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
            error!("update failed: {e:#}");
        } else {
            app.restart();
        }
    }
}

fn main() {
    let cli_args: Vec<String> = std::env::args().skip(1).collect();

    let filter = match std::env::var("RUST_LOG") {
        Ok(_) => EnvFilter::from_default_env(),
        Err(_) => {
            let level = if cli_args.is_empty() { "debug" } else { "warn" };
            EnvFilter::new(format!("capsem={level},capsem_core={level}"))
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    // Open session index early (shared by CLI and GUI paths).
    let session_index = match sessions_dir() {
        Some(d) => {
            let _ = std::fs::create_dir_all(&d);
            match SessionIndex::open(&d.join("main.db")) {
                Ok(idx) => idx,
                Err(e) => {
                    eprintln!("capsem: failed to open session index: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            eprintln!("capsem: HOME not set, cannot create session index");
            std::process::exit(1);
        }
    };

    if !cli_args.is_empty() {
        cleanup_stale_sessions(&session_index);
        let (cli_env, remaining_args) = parse_env_args(&cli_args);
        if remaining_args.is_empty() {
            eprintln!("capsem: no command specified");
            std::process::exit(1);
        }
        let command = remaining_args.join(" ");
        if let Err(e) = run_cli(&command, &cli_env, &session_index) {
            eprintln!("capsem: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    info!("starting capsem");

    // Clean up stale sessions from previous runs.
    cleanup_stale_sessions(&session_index);

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(session_index))
        .setup(|app| {
            info!("tauri setup hook running");

            // Check for updates before booting the VM (the webview gets
            // replaced with VZVirtualMachineView after boot, so we use a
            // native dialog for the update prompt).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                check_for_update(handle).await;
            });

            let assets = match resolve_assets_dir() {
                Ok(a) => a,
                Err(e) => {
                    error!("asset resolution failed: {e:#}");
                    info!("continuing without VM (frontend-only mode)");
                    let _ = app.handle().emit("vm-state-changed", serde_json::json!({
                        "state": "Error",
                        "trigger": "assets_not_found",
                    }));
                    return Ok(());
                }
            };

            info!("assets directory: {}", assets.display());

            // Generate unique session ID for this boot.
            let gui_session_id = session::generate_session_id();
            info!(session_id = %gui_session_id, "starting new session");

            // Create session directory and scratch disk for GUI mode.
            let vm_settings = policy_config::load_merged_vm_settings();
            let scratch_size = vm_settings.scratch_disk_size_gb.unwrap_or(8);
            let ram_bytes: u64 = 512 * 1024 * 1024;
            let gui_session_dir = session_dir_for(&gui_session_id);
            let gui_scratch_path = gui_session_dir.as_ref().and_then(|d| {
                std::fs::create_dir_all(d).ok();
                let path = d.join("scratch.img");
                if let Err(e) = create_scratch_disk(&path, scratch_size) {
                    warn!("failed to create scratch disk: {e}");
                    return None;
                }
                info!(size_gb = scratch_size, "created scratch disk");
                Some(path)
            });

            // Record session in main.db.
            {
                let app_state = app.state::<AppState>();
                let idx = app_state.session_index.lock().unwrap();
                let record = SessionRecord {
                    id: gui_session_id.clone(),
                    mode: "gui".to_string(),
                    command: None,
                    status: "running".to_string(),
                    created_at: session::now_iso(),
                    stopped_at: None,
                    scratch_disk_size_gb: scratch_size,
                    ram_bytes,
                    total_requests: 0,
                    allowed_requests: 0,
                    denied_requests: 0,
                };
                if let Err(e) = idx.create_session(&record) {
                    warn!("failed to record session: {e}");
                }
                // Set active session ID.
                *app_state.active_session_id.lock().unwrap() = Some(gui_session_id.clone());
            }

            // Headless mode: hvc0 is primary console (routed to the frontend)
            match boot_vm(&assets, "console=hvc0 ro loglevel=1", gui_scratch_path.as_deref()) {
                Ok((vm, rx, input_fd, sm)) => {
                    info!("VM booted successfully");

                    // Register vsock listeners on the socket device (including SNI proxy port).
                    let vsock_manager = {
                        let socket_devices = vm.socket_devices();
                        match VsockManager::new(
                            &socket_devices,
                            &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL, VSOCK_PORT_SNI_PROXY],
                        ) {
                            Ok(mgr) => Some(mgr),
                            Err(e) => {
                                warn!("vsock setup failed: {e:#}, using serial-only mode");
                                None
                            }
                        }
                    };

                    // Create per-VM network state (policy + info.db).
                    let net_state = match create_net_state(&gui_session_id) {
                        Ok(ns) => Some(ns),
                        Err(e) => {
                            warn!("network state init failed: {e:#}, SNI proxy disabled");
                            None
                        }
                    };

                    // Store VM state.
                    {
                        let app_state = app.state::<AppState>();
                        let mut vms = app_state.vms.lock().unwrap();
                        vms.insert(gui_session_id.clone(), VmInstance {
                            vm,
                            serial_input_fd: input_fd,
                            vsock_terminal_fd: None,
                            vsock_control_fd: None,
                            net_state,
                            state_machine: sm,
                            scratch_disk_path: gui_scratch_path.clone(),
                        });
                    }

                    let handle = app.handle().clone();
                    // Serial forwarding for boot logs (aborted once vsock connects).
                    let serial_task = tauri::async_runtime::spawn(
                        serial_to_events(handle.clone(), rx),
                    );

                    // Spawn vsock connection handler if available.
                    if let Some(mgr) = vsock_manager {
                        tauri::async_runtime::spawn(
                            setup_vsock(handle.clone(), mgr, serial_task),
                        );
                    }

                    // Push initial state to frontend (Booting, not yet Running).
                    let _ = handle.emit("vm-state-changed", serde_json::json!({
                        "state": "Booting",
                        "trigger": "vm_started",
                    }));
                }
                Err(e) => {
                    error!("VM boot failed: {e:#}");
                    info!("continuing without VM (unsigned binary or missing entitlement)");
                    let _ = app.handle().emit("vm-state-changed", serde_json::json!({
                        "state": "Error",
                        "trigger": "boot_failed",
                    }));
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vm_status,
            commands::serial_input,
            commands::terminal_resize,
            commands::net_events,
            commands::get_guest_config,
            commands::get_network_policy,
            commands::set_guest_env,
            commands::remove_guest_env,
            commands::get_vm_state,
            commands::get_settings,
            commands::update_setting,
            commands::get_session_info,
            commands::get_session_history,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
