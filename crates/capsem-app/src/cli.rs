use std::io::{Read, Write};
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_core::{
    GuestToHost, HostState, HostToGuest, VirtioFsShare, VmState, VsockManager,
    VSOCK_PORT_CONTROL, VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
    create_virtiofs_session,
};
use capsem_core::mcp::gateway::{self, McpGatewayConfig};
use capsem_core::mcp::server_manager::McpServerManager;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy_config;
use capsem_core::session::{self, SessionIndex, SessionRecord};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::assets::{resolve_assets_dir, resolve_rootfs, create_asset_manager, rootfs_manifest_name};
use crate::boot::{boot_vm, clone_fd, read_control_msg, write_control_msg, send_boot_config};
use crate::logging::write_perf_log;
use crate::boot::create_net_state;
use crate::session_mgmt::{session_dir_for, open_session_db, cleanup_session, vacuum_session};
use crate::vsock_wiring::{wire_auto_snapshots, spawn_auto_snapshot_timer};

pub(crate) const CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// Parse `--env KEY=VALUE` pairs from CLI args, returning env pairs and remaining args.
///
/// CLI --env args are validated strictly: invalid keys or values cause an error
/// message and the pair is skipped (stricter than config file handling).
pub(crate) fn parse_env_args(args: &[String]) -> (Vec<(String, String)>, Vec<String>) {
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
pub(crate) fn run_cli(command: &str, cli_env: &[(String, String)], session_index: &SessionIndex, log_handle: Option<&capsem_core::log_layer::LogHandle>) -> Result<()> {
    // Tokio runtime for async MITM proxy handlers.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("failed to create tokio runtime")?;

    let assets = resolve_assets_dir()?;

    // Resolve rootfs: check bundled assets first, then ~/.capsem/assets/.
    // If missing, download it before booting.
    let rootfs_path = match resolve_rootfs(&assets) {
        Some(path) => Some(path),
        None => {
            eprintln!("[capsem] rootfs not found, downloading...");
            let mgr = create_asset_manager(&assets)?;
            let name = rootfs_manifest_name(&mgr)?;
            let _ = mgr.cleanup_unrecognized();
            let client = reqwest::Client::new();
            let downloaded = rt.block_on(mgr.download_asset(&name, &client, |p| {
                if p.total_bytes > 0 {
                    let pct = (p.bytes_downloaded as f64 / p.total_bytes as f64 * 100.0) as u32;
                    eprint!("\r[capsem] {}: {}% ({}/{} bytes)   ",
                        p.phase, pct, p.bytes_downloaded, p.total_bytes);
                } else {
                    eprint!("\r[capsem] {}: {} bytes   ", p.phase, p.bytes_downloaded);
                }
            }))?;
            eprintln!();
            Some(downloaded)
        }
    };

    // Generate unique session ID.
    let cli_session_id = session::generate_session_id();
    eprintln!("[capsem] session: {cli_session_id}");

    // Create session directory with VirtioFS overlay.
    let policies = policy_config::MergedPolicies::from_disk();
    let vm_settings = policies.vm;
    let cpu_count = vm_settings.cpu_count.unwrap_or(4);
    let ram_gb = vm_settings.ram_gb.unwrap_or(4);
    let ram_bytes: u64 = ram_gb as u64 * 1024 * 1024 * 1024;
    let cli_session_dir = session_dir_for(&cli_session_id);

    // Set up VirtioFS session directory (overlay upper + work + auto_snapshots).
    let virtiofs_shares: Vec<VirtioFsShare> = cli_session_dir
        .as_ref()
        .and_then(|d| {
            std::fs::create_dir_all(d).ok();
            if let Err(e) = create_virtiofs_session(d, 2) {
                warn!("failed to create VirtioFS session dir: {e}");
                return None;
            }
            info!("created VirtioFS session dir");
            Some(vec![VirtioFsShare {
                tag: "capsem".to_string(),
                host_path: d.clone(),
                read_only: false,
            }])
        })
        .unwrap_or_default();

    // Open per-VM log file for structured event capture.
    if let Some(ref dir) = cli_session_dir {
        if let Some(lh) = log_handle {
            if let Ok(f) = std::fs::File::create(dir.join("capsem.log")) {
                lh.set_vm_writer(f);
            }
        }
    }

    // Record session in main.db.
    let record = SessionRecord {
        id: cli_session_id.clone(),
        mode: "cli".to_string(),
        command: Some(command.to_string()),
        status: VmState::Running.to_string(),
        created_at: session::now_iso(),
        stopped_at: None,
        scratch_disk_size_gb: 0,
        ram_bytes,
        total_requests: 0,
        allowed_requests: 0,
        denied_requests: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_estimated_cost: 0.0,
        total_tool_calls: 0,
        total_mcp_calls: 0,
        total_file_events: 0,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: if virtiofs_shares.is_empty() { "block" } else { "virtiofs" }.to_string(),
        rootfs_hash: None,
        rootfs_version: None,
    };
    if let Err(e) = session_index.create_session(&record) {
        warn!("failed to record session: {e}");
    }

    let (vm, mut rx, _serial_input_fd, mut sm) = boot_vm(
        &assets,
        rootfs_path.as_deref(),
        "console=hvc0 ro loglevel=1 init_on_alloc=1 slab_nomerge page_alloc.shuffle=1",
        None, // no scratch disk in VirtioFS mode
        &virtiofs_shares,
        cpu_count,
        ram_bytes,
    )?;

    // Set up vsock listeners (including SNI proxy and MCP gateway ports).
    let socket_devices = vm.socket_devices();
    let mut mgr = VsockManager::new(
        &socket_devices,
        &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_MCP_GATEWAY],
    ).context("failed to set up vsock")?;

    // Open session DB (hard fail -- needed by file monitor, MCP, telemetry).
    let session_db = open_session_db(&cli_session_id)?;

    // Create per-VM network state for MITM proxy.
    let net_state = match create_net_state(&cli_session_id, Arc::clone(&session_db)) {
        Ok(ns) => Some(ns),
        Err(e) => {
            error!("MITM proxy disabled: {e:#}");
            None
        }
    };
    let mitm_config: Option<Arc<MitmProxyConfig>> = net_state.as_ref().map(|ns| {
        Arc::new(MitmProxyConfig {
            ca: Arc::clone(&ns.ca),
            policy: Arc::clone(&ns.policy),
            db: Arc::clone(&ns.db),
            upstream_tls: Arc::clone(&ns.upstream_tls),
            pricing: capsem_core::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(capsem_core::gateway::TraceState::new()),
        })
    });

    // Create MCP gateway config for vsock:5003 using pre-built policies.
    let (user_sf, corp_sf) = policy_config::load_settings_files();
    let user_mcp = user_sf.mcp.clone().unwrap_or_default();
    let corp_mcp = corp_sf.mcp.clone().unwrap_or_default();
    let mcp_servers = capsem_core::mcp::build_server_list(&user_mcp, &corp_mcp);
    let mcp_config: Option<Arc<McpGatewayConfig>> = {
        let http_client = reqwest::Client::builder()
            .user_agent("capsem-mcp/0.8")
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("reqwest client");
        Some(Arc::new(McpGatewayConfig {
            server_manager: tokio::sync::Mutex::new(McpServerManager::new(mcp_servers.clone(), http_client.clone())),
            db: Arc::clone(&session_db),
            policy: tokio::sync::RwLock::new(Arc::new(policies.mcp)),
            domain_policy: std::sync::RwLock::new(Arc::new(policies.domain)),
            http_client,
            auto_snapshots: None, // set after boot in VirtioFS mode
            workspace_dir: cli_session_dir.as_ref().map(|d| d.join("workspace")),
        }))
    };

    // Initialize MCP servers and run tool pinning (blocking in CLI mode).
    if let Some(ref config) = mcp_config {
        let config = Arc::clone(config);
        rt.block_on(async {
            let mut mgr = config.server_manager.lock().await;
            if let Err(e) = mgr.initialize_all().await {
                warn!("MCP server initialization failed: {e:#}");
            }
            // Tool cache pinning (detect rug pulls).
            let cache = capsem_core::mcp::load_tool_cache();
            let changes = capsem_core::mcp::detect_pin_changes(mgr.tool_catalog(), &cache);
            for change in &changes {
                match change {
                    capsem_core::mcp::PinChange::Changed { namespaced_name, .. } => {
                        warn!(tool = %namespaced_name, "MCP tool definition changed (possible rug pull)");
                    }
                    capsem_core::mcp::PinChange::New { namespaced_name } => {
                        info!(tool = %namespaced_name, "new MCP tool discovered");
                    }
                    capsem_core::mcp::PinChange::Removed { namespaced_name } => {
                        info!(tool = %namespaced_name, "MCP tool removed");
                    }
                }
            }
            let new_cache = capsem_core::mcp::build_cache_entries(mgr.tool_catalog(), &cache);
            if let Err(e) = capsem_core::mcp::save_tool_cache(&new_cache) {
                warn!("failed to save MCP tool cache: {e}");
            }
        });
    }

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
                VSOCK_PORT_MCP_GATEWAY => {
                    if let Some(ref config) = mcp_config {
                        let fd = conn.fd;
                        let config = Arc::clone(config);
                        rt.spawn(async move {
                            let _conn = conn;
                            gateway::serve_mcp_session(fd, config).await;
                        });
                        continue;
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
    let mut ctrl_fd_reader = match clone_fd(control_fd) {
        Ok(f) => f,
        Err(e) => anyhow::bail!("failed to clone control fd: {e}"),
    };
    std::thread::spawn(move || {
        loop {
            match read_control_msg(&mut ctrl_fd_reader) {
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
                let _ = sm.transition(HostState::VsockConnected, "vsock_ports_connected");
                let _ = sm.transition(HostState::Handshaking, "ready_received");
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

    let mut ctrl_fd_writer = clone_fd(control_fd)?;
    // Send boot config as individual messages.
    send_boot_config(&mut ctrl_fd_writer, cli_env)?;

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
                let _ = sm.transition(HostState::Running, "boot_ready_received");
                write_perf_log(&sm);
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

    // Start auto-snapshot scheduler and file monitor in VirtioFS mode.
    // _fs_monitor must outlive the session -- dropping it stops FSEvents.
    let mut _fs_monitor: Option<capsem_core::fs_monitor::FsMonitor> = None;
    if !virtiofs_shares.is_empty() {
        if let Some(ref dir) = cli_session_dir {
            // Wire auto-snapshot scheduler into MCP config.
            if let Some(ref config) = mcp_config {
                if let Some((scheduler, interval)) = rt.block_on(wire_auto_snapshots(config, dir)) {
                    spawn_auto_snapshot_timer(rt.handle(), scheduler, interval);
                }
            }

            // Start host file monitor (uses session_db directly, not gated on MITM proxy).
            // _fs_monitor must live until session ends -- dropping it stops the FSEvents watcher.
            let workspace = dir.join("workspace");
            _fs_monitor = match capsem_core::fs_monitor::FsMonitor::start(
                workspace.clone(),
                workspace.clone(),
                Arc::clone(&session_db),
            ) {
                Ok(monitor) => {
                    info!("host file monitor started");
                    Some(monitor)
                }
                Err(e) => {
                    warn!("failed to start host file monitor: {e}");
                    None
                }
            };

            info!("VirtioFS auto-snapshots and file monitor started");
        }
    }

    // Send Exec command.
    let exec_id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut exec_file = clone_fd(control_fd)?;
    write_control_msg(&mut exec_file, &HostToGuest::Exec {
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
        // Accept any incoming proxy connections during exec.
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
            } else if conn.port == VSOCK_PORT_MCP_GATEWAY {
                if let Some(ref config) = mcp_config {
                    let fd = conn.fd;
                    let config = Arc::clone(config);
                    rt.spawn(async move {
                        let _conn = conn;
                        gateway::serve_mcp_session(fd, config).await;
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
            Ok(GuestToHost::BootTiming { ref stages }) => {
                for s in stages {
                    eprintln!("[capsem] boot timing: {} {}ms", s.name, s.duration_ms);
                }
                let total: u64 = stages.iter().map(|s| s.duration_ms).sum();
                eprintln!("[capsem] boot timing total: {}ms", total);
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

    // Flush and close the per-VM log writer.
    if let Some(lh) = log_handle {
        lh.clear_vm_writer();
    }

    // Clean up session: delete VirtioFS dirs (or scratch.img for legacy), update status.
    if let Some(ref dir) = cli_session_dir {
        cleanup_session(dir, None, &cli_session_id, session_index, Some(session_db.as_ref()));
    }

    // Drop network state to close DbWriter (flushes WAL via checkpoint on drop).
    drop(net_state);

    // Vacuum and compress the session DB.
    if let Some(ref dir) = cli_session_dir {
        vacuum_session(&cli_session_id, session_index, dir);
    }

    // Ensure the host shell prompt starts on a fresh line.
    if !last_was_newline.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = std::io::stdout().write_all(b"\n");
        let _ = std::io::stdout().flush();
    }
    std::process::exit(exit_code);
}
