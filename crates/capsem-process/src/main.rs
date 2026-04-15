mod helpers;
mod ipc;
mod job_store;
mod terminal;
mod vsock;

use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use clap::Parser;
use capsem_core::{
    boot_vm, BootOptions, VirtioFsShare,
    VsockConnection,
};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

use helpers::query_max_fs_event_id;
use job_store::JobStore;
use vsock::VsockOptions;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)] id: String,
    #[arg(long)] assets_dir: PathBuf,
    #[arg(long)] rootfs: PathBuf,
    #[arg(long)] session_dir: PathBuf,
    #[arg(long, default_value_t = 2)] cpus: u32,
    #[arg(long, default_value_t = 2048)] ram_mb: u64,
    #[arg(long)] uds_path: PathBuf,
    #[arg(long)] checkpoint_path: Option<PathBuf>,
    /// Environment variables to inject into guest (repeatable: --env KEY=VALUE)
    #[arg(long = "env")]
    env: Vec<String>,
}

fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_writer(std::io::stderr))
        .init();
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    info!(id = %args.id, "capsem-sandbox-process starting");

    std::fs::create_dir_all(&args.session_dir)?;
    capsem_core::create_virtiofs_session(&args.session_dir, 2)?;
    let guest_dir = capsem_core::guest_share_dir(&args.session_dir);
    let virtiofs_shares = vec![VirtioFsShare { tag: "capsem".into(), host_path: guest_dir, read_only: false }];

    let (vm, vsock_rx, sm) = boot_vm(BootOptions {
        assets: &args.assets_dir,
        rootfs_override: Some(&args.rootfs),
        cmdline: "console=hvc0 ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1",
        scratch_disk_path: None,
        virtiofs_shares: &virtiofs_shares,
        cpu_count: args.cpus,
        ram_bytes: args.ram_mb * 1024 * 1024,
        checkpoint_path: args
            .checkpoint_path
            .clone()
            .map(|p| if p.is_absolute() { p } else { args.session_dir.join(p) }),
    })?;

    // Delete checkpoint file if we just restored from it, so we don't accidentally suspend on normal shutdown
    if let Some(cp) = &args.checkpoint_path {
        let full_path = if std::path::Path::new(cp).is_absolute() {
            std::path::PathBuf::from(cp)
        } else {
            args.session_dir.join(cp)
        };
        let _ = std::fs::remove_file(full_path);
    }

    let vm_arc = Arc::new(tokio::sync::Mutex::new(vm));

    // Emit boot timeline state transitions for process.log.
    for t in sm.history() {
        info!(
            category = "boot_timeline",
            from = %t.from, to = %t.to,
            trigger = %t.trigger,
            duration_ms = t.duration_in_from.as_millis() as u64,
            "state transition"
        );
    }

    rt.spawn(async move {
        if let Err(e) = run_async_main_loop(args, vm_arc, vsock_rx, sm).await {
            error!("async loop failed: {e:#}");
            std::process::exit(1);
        }
    });

    #[cfg(target_os = "macos")]
    unsafe { core_foundation_sys::runloop::CFRunLoopRun(); }
    #[cfg(not(target_os = "macos"))]
    rt.block_on(tokio::signal::ctrl_c())?;

    Ok(())
}

async fn run_async_main_loop(
    args: Args,
    vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    _sm: capsem_core::host_state::HostStateMachine,
) -> Result<()> {
    let job_store = Arc::new(JobStore::new());
    let (ipc_tx, _) = broadcast::channel::<ProcessToService>(128);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<ServiceToProcess>(32);
    let terminal_output = Arc::new(capsem_core::TerminalOutputQueue::new());

    let db = Arc::new(capsem_logger::DbWriter::open(&args.session_dir.join("session.db"), 256)?);

    // Start host file monitor to record fs_events.
    // _fs_monitor must live until the process exits to keep the watcher alive.
    let workspace_dir = args.session_dir.join("workspace");
    let _fs_monitor = match capsem_core::fs_monitor::FsMonitor::start(
        workspace_dir.clone(),
        workspace_dir.clone(),
        Arc::clone(&db),
    ) {
        Ok(monitor) => {
            info!("host file monitor started");
            Some(monitor)
        }
        Err(e) => {
            error!("failed to start host file monitor: {e}");
            None
        }
    };

    // Load settings files once and derive everything from them.
    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();
    let merged = capsem_core::net::policy_config::MergedPolicies::from_files(&user_sf, &corp_sf);
    let snap_settings = capsem_core::net::policy_config::resolve_settings(&user_sf, &corp_sf);
    let guest_config = merged.guest.clone();

    let net_state = Arc::new(capsem_core::create_net_state_with_policy(
        &args.id,
        Arc::clone(&db),
        merged.network.clone(),
    )?);
    let mcp_servers = capsem_core::mcp::build_server_list(
        &user_sf.mcp.clone().unwrap_or_default(),
        &corp_sf.mcp.clone().unwrap_or_default(),
    );
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
        args.session_dir.clone(),
        snap_auto_max,
        snap_manual_max,
        std::time::Duration::from_secs(snap_interval),
    );
    let scheduler = Arc::new(tokio::sync::Mutex::new(scheduler));

    // Defer initial snapshot to background -- workspace is empty at boot, no need to block.
    {
        let sched = Arc::clone(&scheduler);
        let db_snap = Arc::clone(&db);
        tokio::spawn(async move {
            let mut s = sched.lock().await;
            if let Ok(slot) = s.take_snapshot() {
                let stop_id = query_max_fs_event_id(&db_snap);
                db_snap.write(capsem_logger::WriteOp::SnapshotEvent(
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
        });
    }

    // Spawn the isolated MCP aggregator subprocess.
    let aggregator_client = spawn_mcp_aggregator(&mcp_servers).await?;

    let mcp_config = Arc::new(capsem_core::mcp::gateway::McpGatewayConfig {
        aggregator: aggregator_client,
        db: Arc::clone(&db),
        policy: tokio::sync::RwLock::new(Arc::new(merged.mcp)),
        domain_policy: std::sync::RwLock::new(Arc::new(merged.domain)),
        http_client: reqwest::Client::new(),
        auto_snapshots: Some(Arc::clone(&scheduler)),
        workspace_dir: Some(args.session_dir.join("workspace")),
    });

    let mitm_config = Arc::new(capsem_core::net::mitm_proxy::MitmProxyConfig {
        ca: Arc::clone(&net_state.ca),
        policy: Arc::clone(&net_state.policy),
        db: Arc::clone(&db),
        upstream_tls: Arc::clone(&net_state.upstream_tls),
        pricing: capsem_core::net::ai_traffic::pricing::PricingTable::load(),
        trace_state: std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new()),
    });

    let db_clone = Arc::clone(&db);
    let sched_clone = Arc::clone(&scheduler);
    let initial_stop = query_max_fs_event_id(&db_clone);
    tokio::spawn(async move {
        let mut last_stop = initial_stop;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(snap_interval));
        tick.tick().await;
        loop {
            tick.tick().await;
            let sched = Arc::clone(&sched_clone);
            let result = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    let mut s = sched.lock().await;
                    s.take_snapshot()
                })
            }).await;
            match result {
                Ok(Ok(slot)) => {
                    let stop_id = query_max_fs_event_id(&db_clone);
                    db_clone.write(capsem_logger::WriteOp::SnapshotEvent(
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

    let ipc_tx_clone = ipc_tx.clone();
    let job_store_clone = Arc::clone(&job_store);
    let terminal_output_clone = Arc::clone(&terminal_output);

    // Spawn serial log reader
    let mut rx = {
        let v = vm.lock().await;
        v.serial().subscribe()
    };
    let log_path = args.session_dir.join("serial.log");
    tokio::spawn(async move {
        use std::io::Write;
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .ok();
        while let Ok(data) = rx.recv().await {
            if let Some(ref mut f) = log_file {
                let _ = f.write_all(&data);
                let _ = f.flush();
            }
        }
    });

    let session_dir = args.session_dir.clone();
    let net_state_clone = Arc::clone(&net_state);
    let mitm_config_clone = Arc::clone(&mitm_config);
    let mcp_config_clone = Arc::clone(&mcp_config);

    // Parse --env KEY=VALUE pairs for guest injection
    let cli_env: Vec<(String, String)> = args.env.iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();

    let vm_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let ctrl_tx_ipc = ctrl_tx.clone();
    let uds_path = args.uds_path.clone();
    let vm_id_ws = args.id.clone();
    let is_restore = args.checkpoint_path.is_some();
    let vm_for_vsock = Arc::clone(&vm);
    let vm_ready_vsock = Arc::clone(&vm_ready);
    let uds_path_vsock = uds_path.clone();
    tokio::spawn(async move {
        if let Err(e) = vsock::setup_vsock(VsockOptions {
            vm_id: args.id.clone(),
            vm: vm_for_vsock,
            vsock_rx,
            ipc_tx: ipc_tx_clone,
            ctrl_tx,
            ctrl_rx,
            terminal_output: terminal_output_clone,
            job_store: job_store_clone,
            session_dir,
            cli_env,
            guest_config,
            mitm_config: mitm_config_clone,
            mcp_config: mcp_config_clone,
            net_state: net_state_clone,
            is_restore,
            vm_ready: vm_ready_vsock,
            uds_path: uds_path_vsock,
        }).await {
            error!("vsock failed: {e:#}");
        }
    });

    if uds_path.exists() { std::fs::remove_file(&uds_path)?; }
    let listener = UnixListener::bind(&uds_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!(socket = %uds_path.display(), "listening for IPC (mode 0600)");

    let ws_sock_path = uds_path.with_file_name(format!("{}-ws.sock", vm_id_ws));
    if ws_sock_path.exists() { std::fs::remove_file(&ws_sock_path)?; }
    let ws_listener = tokio::net::UnixListener::bind(&ws_sock_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ws_sock_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!(socket = %ws_sock_path.display(), "listening for terminal WS (mode 0600)");

    // We use a broadcast channel to fan out terminal output to multiple WS connections
    let (term_bcast_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(1024);
    let term_c_bcast = Arc::clone(&terminal_output);
    let term_bcast_tx_clone = term_bcast_tx.clone();
    tokio::spawn(async move {
        while let Some(data) = term_c_bcast.poll().await {
            let _ = term_bcast_tx_clone.send(data);
        }
    });

    let ctrl_tx_ws = ctrl_tx_ipc.clone();
    let term_bcast_tx_app = term_bcast_tx.clone();

    let ws_app = axum::Router::new()
        .route("/terminal", axum::routing::get(
            move |ws: axum::extract::ws::WebSocketUpgrade| {
                let ctrl_tx = ctrl_tx_ws.clone();
                let term_rx = term_bcast_tx_app.subscribe();
                async move {
                    ws.on_upgrade(move |socket| terminal::handle_terminal_socket(socket, ctrl_tx, term_rx))
                }
            }
        ));

    tokio::spawn(async move {
        if let Err(e) = axum::serve(ws_listener, ws_app).await {
            error!("WS server error: {}", e);
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let tx_c = ctrl_tx_ipc.clone();
        let ipc_tx_pass = ipc_tx.clone();
        let term_c = term_bcast_tx.clone();
        let job_c = Arc::clone(&job_store);
        let net_c = Arc::clone(&net_state);
        let mcp_c = Arc::clone(&mcp_config);
        let ready_c = Arc::clone(&vm_ready);

        tokio::spawn(async move {
            if let Err(e) = ipc::handle_ipc_connection(stream, tx_c, ipc_tx_pass, term_c, job_c, net_c, mcp_c, ready_c).await {
                error!("IPC error: {e:#}");
            }
        });
    }
}

/// Spawn the isolated MCP aggregator subprocess and return a client handle.
///
/// The subprocess manages connections to external MCP servers. It communicates
/// via NDJSON on stdin/stdout. Server definitions are sent as the first line.
///
/// If the aggregator binary is not found (dev builds), falls back to an in-process
/// mock that returns empty results.
async fn spawn_mcp_aggregator(
    servers: &[capsem_core::mcp::types::McpServerDef],
) -> Result<capsem_core::mcp::aggregator::AggregatorClient> {
    use std::collections::HashMap;
    use capsem_core::mcp::aggregator::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (client, mut rx) = AggregatorClient::channel(64);

    // Find the aggregator binary next to our own binary.
    let exe_path = std::env::current_exe()?;
    let bin_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
    let aggregator_bin = bin_dir.join("capsem-mcp-aggregator");

    if !aggregator_bin.exists() {
        // Dev fallback: no aggregator binary. Return a client with an empty mock driver.
        info!("aggregator binary not found at {}, using empty stub", aggregator_bin.display());
        tokio::spawn(async move {
            while let Some((req, resp_tx)) = rx.recv().await {
                let body = match req.method {
                    AggregatorMethod::ListServers => AggregatorResult::Servers { servers: vec![] },
                    AggregatorMethod::ListTools => AggregatorResult::Tools { tools: vec![] },
                    AggregatorMethod::ListResources => AggregatorResult::Resources { resources: vec![] },
                    AggregatorMethod::ListPrompts => AggregatorResult::Prompts { prompts: vec![] },
                    AggregatorMethod::CallTool { name, .. } => AggregatorResult::Error {
                        error: format!("aggregator not available: {name}"),
                    },
                    AggregatorMethod::ReadResource { uri, .. } => AggregatorResult::Error {
                        error: format!("aggregator not available: {uri}"),
                    },
                    AggregatorMethod::GetPrompt { name, .. } => AggregatorResult::Error {
                        error: format!("aggregator not available: {name}"),
                    },
                    AggregatorMethod::Refresh { .. } | AggregatorMethod::Shutdown => {
                        AggregatorResult::Ok { ok: true }
                    }
                };
                let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
            }
        });
        return Ok(client);
    }

    info!(bin = %aggregator_bin.display(), servers = servers.len(), "spawning MCP aggregator");

    let mut child = tokio::process::Command::new(&aggregator_bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;

    let mut child_stdin = child.stdin.take().unwrap();
    let child_stdout = child.stdout.take().unwrap();

    // Send server definitions as the first line.
    let defs_json = serde_json::to_string(servers)?;
    child_stdin.write_all(defs_json.as_bytes()).await?;
    child_stdin.write_all(b"\n").await?;
    child_stdin.flush().await?;

    // Background driver: reads from client channel, writes to subprocess stdin,
    // reads responses from subprocess stdout, routes back to callers.
    let pending: Arc<tokio::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<AggregatorResponse>>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Reader task: reads NDJSON from subprocess stdout and routes to pending callers.
    let pending_reader = Arc::clone(&pending);
    let mut reader = BufReader::new(child_stdout);
    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    info!("aggregator stdout closed");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    match serde_json::from_str::<AggregatorResponse>(trimmed) {
                        Ok(resp) => {
                            let mut map = pending_reader.lock().await;
                            if let Some(tx) = map.remove(&resp.id) {
                                let _ = tx.send(resp);
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "failed to parse aggregator response");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "failed to read from aggregator");
                    break;
                }
            }
        }
    });

    // Writer task: reads from client channel, writes to subprocess stdin.
    let pending_writer = Arc::clone(&pending);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            {
                let mut map = pending_writer.lock().await;
                map.insert(req.id, resp_tx);
            }
            let mut req_json = match serde_json::to_string(&req) {
                Ok(j) => j,
                Err(e) => {
                    error!(error = %e, "failed to serialize aggregator request");
                    continue;
                }
            };
            req_json.push('\n');
            if let Err(e) = child_stdin.write_all(req_json.as_bytes()).await {
                error!(error = %e, "failed to write to aggregator");
                break;
            }
            if let Err(e) = child_stdin.flush().await {
                error!(error = %e, "failed to flush aggregator stdin");
                break;
            }
        }
    });

    // Monitor child process.
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) => info!(status = %status, "aggregator subprocess exited"),
            Err(e) => error!(error = %e, "failed to wait on aggregator"),
        }
    });

    Ok(client)
}


#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // -----------------------------------------------------------------------
    // Args parsing
    // -----------------------------------------------------------------------

    #[test]
    fn args_parses_all_required() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "test-vm",
            "--assets-dir", "/tmp/assets",
            "--rootfs", "/tmp/rootfs.img",
            "--session-dir", "/tmp/session",
            "--uds-path", "/tmp/vm.sock",
        ]).unwrap();
        assert_eq!(args.id, "test-vm");
        assert_eq!(args.assets_dir, PathBuf::from("/tmp/assets"));
        assert_eq!(args.rootfs, PathBuf::from("/tmp/rootfs.img"));
        assert_eq!(args.session_dir, PathBuf::from("/tmp/session"));
        assert_eq!(args.uds_path, PathBuf::from("/tmp/vm.sock"));
    }

    #[test]
    fn args_default_cpus() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
        ]).unwrap();
        assert_eq!(args.cpus, 2);
    }

    #[test]
    fn args_default_ram_mb() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
        ]).unwrap();
        assert_eq!(args.ram_mb, 2048);
    }

    #[test]
    fn args_custom_cpus_and_ram() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
            "--cpus", "8", "--ram-mb", "16384",
        ]).unwrap();
        assert_eq!(args.cpus, 8);
        assert_eq!(args.ram_mb, 16384);
    }

    #[test]
    fn args_missing_required_id_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_missing_required_assets_dir_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_invalid_cpus_type_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
            "--cpus", "not-a-number",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_checkpoint_path_optional() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
        ]).unwrap();
        assert!(args.checkpoint_path.is_none());
    }

    #[test]
    fn args_checkpoint_path_set() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
            "--checkpoint-path", "/tmp/cp.vzsave",
        ]).unwrap();
        assert_eq!(args.checkpoint_path.unwrap(), PathBuf::from("/tmp/cp.vzsave"));
    }

    #[test]
    fn args_env_vars_parsed() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id", "vm", "--assets-dir", "/a", "--rootfs", "/r",
            "--session-dir", "/s", "--uds-path", "/u",
            "--env", "FOO=bar", "--env", "BAZ=qux",
        ]).unwrap();
        assert_eq!(args.env, vec!["FOO=bar", "BAZ=qux"]);
    }

    // -----------------------------------------------------------------------
    // CLI env parsing (used in run_async_main_loop)
    // -----------------------------------------------------------------------

    #[test]
    fn cli_env_parsing_valid() {
        let env = vec!["FOO=bar".to_string(), "BAZ=qux=extra".to_string()];
        let parsed: Vec<(String, String)> = env.iter()
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        assert_eq!(parsed, vec![
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux=extra".to_string()),
        ]);
    }

    #[test]
    fn cli_env_parsing_no_equals_skipped() {
        let env = vec!["NOEQ".to_string(), "GOOD=val".to_string()];
        let parsed: Vec<(String, String)> = env.iter()
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ("GOOD".to_string(), "val".to_string()));
    }

    #[test]
    fn cli_env_parsing_empty_value() {
        let env = vec!["KEY=".to_string()];
        let parsed: Vec<(String, String)> = env.iter()
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        assert_eq!(parsed, vec![("KEY".to_string(), "".to_string())]);
    }
}
