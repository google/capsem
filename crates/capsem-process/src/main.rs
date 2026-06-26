mod helpers;
mod ipc;
mod job_store;
mod mcp_runtime;
mod pty_log;
mod runtime_config;
mod terminal;
mod vsock;

use anyhow::{Context, Result};
use capsem_core::fs_monitor::FsMonitor;
use capsem_core::net::dns::{DnsAnswerCache, DnsResolver};
use capsem_core::{boot_vm, BootOptions, VirtioFsShare, VsockConnection};
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use job_store::JobStore;
use mcp_runtime::McpRuntime;
use vsock::VsockOptions;

/// Owns the background-thread resources that MUST drain before the main
/// run loop stops. Populated by `run_async_main_loop` once DbWriter and
/// FsMonitor are constructed, drained by the SIGTERM handler before it
/// calls `CFRunLoopStop`. See the sprint doc at
/// `sprints/explicit-shutdown-cleanup/` and /dev-rust-patterns
/// "Signal-driven explicit cleanup".
#[derive(Default)]
struct Shutdown {
    db: Option<Arc<DbWriter>>,
    fs_monitor: Option<FsMonitor>,
}

impl Shutdown {
    /// Drain in order: fs_events fan into DbWriter, so FsMonitor must
    /// finish its final flush before the DbWriter runs its checkpoint.
    /// Blocking — caller should run this from `spawn_blocking`.
    fn drain_blocking(&mut self) {
        if let Some(fs_monitor) = self.fs_monitor.take() {
            fs_monitor.shutdown_and_join();
        }
        if let Some(db) = self.db.take() {
            db.shutdown_blocking();
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    id: String,
    #[arg(long)]
    assets_dir: PathBuf,
    #[arg(long)]
    rootfs: PathBuf,
    /// Explicit kernel path (overrides assets_dir/vmlinuz)
    #[arg(long)]
    kernel: Option<PathBuf>,
    /// Explicit initrd path (overrides assets_dir/initrd.img)
    #[arg(long)]
    initrd: Option<PathBuf>,
    #[arg(long)]
    session_dir: PathBuf,
    #[arg(long)]
    active_profile: PathBuf,
    #[arg(long, default_value_t = 2)]
    cpus: u32,
    #[arg(long, default_value_t = 2048)]
    ram_mb: u64,
    #[arg(long, default_value_t = 16)]
    scratch_disk_size_gb: u32,
    #[arg(long)]
    uds_path: PathBuf,
    #[arg(long)]
    checkpoint_path: Option<PathBuf>,
    /// Environment variables to inject into guest (repeatable: --env KEY=VALUE)
    #[arg(long = "env")]
    env: Vec<String>,
}

/// Generate a short (16-hex-char) correlation id for this
/// capsem-process's lifetime. Propagated to capsem-mcp-aggregator via
/// `CAPSEM_TRACE_ID` so all three host-side processes
/// (service -> process -> aggregator) share a `trace_id` field on
/// every log line, making cross-process correlation grep-able.
///
/// Not cryptographic -- it just needs enough entropy to disambiguate
/// concurrent processes and rapid-fire restarts on the same host.
fn generate_trace_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    // FxHash-style mixer -- cheap, deterministic, plenty of bit churn
    // for the "probably unique within this host" bar we need here.
    static MIX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let bump = MIX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mixed = nanos
        .wrapping_mul(0x9e3779b97f4a7c15)
        .wrapping_add(pid)
        .wrapping_add(bump.wrapping_mul(0x94d049bb133111eb));
    format!("{mixed:016x}")
}

/// Path to the aggregator's dedicated stderr log within a VM's session
/// directory. Kept out of `process.log` so the parent's JSON tracing
/// stream isn't polluted by child text tracing (the two used to mix
/// because `Stdio::inherit()` forwarded the aggregator's stderr
/// straight into the parent's log sink).
fn aggregator_log_path(session_dir: &Path) -> PathBuf {
    session_dir.join("mcp-aggregator.stderr.log")
}

fn prepare_session_layout(session_dir: &Path, scratch_disk_size_gb: u32) -> Result<PathBuf> {
    capsem_core::create_virtiofs_session(session_dir, scratch_disk_size_gb)?;
    Ok(capsem_core::guest_share_dir(session_dir))
}

fn main() -> Result<()> {
    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-process",
        sink: capsem_core::telemetry::LogSink::Stderr,
        default_filter: "info",
    })?;
    let args = Args::parse();

    // Root span shared across the whole capsem-process run: every
    // subsequent log line inherits `vm_id` and `trace_id` as structured
    // fields in the JSON output. Guard is held until main returns.
    let trace_id = generate_trace_id();
    let root_span = tracing::info_span!("vm", vm_id = %args.id, trace_id = %trace_id);
    let _root_span_guard = root_span.enter();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    info!(id = %args.id, "capsem-sandbox-process starting");

    std::fs::create_dir_all(&args.session_dir)?;
    let mut session_dir = args.session_dir.clone();
    if let Ok(resolved) = session_dir.canonicalize() {
        session_dir = resolved;
    }

    let guest_dir = prepare_session_layout(&session_dir, args.scratch_disk_size_gb)?;
    let virtiofs_shares = vec![VirtioFsShare {
        tag: "capsem".into(),
        host_path: guest_dir.clone(),
        read_only: false,
    }];

    // Attach the system-overlay rootfs.img as a virtio-blk device (/dev/vdb in
    // the guest). capsem-init mounts it as the overlayfs upper directly --
    // native virtio-blk speaks block-device semantics and doesn't EIO under
    // writeback pressure across save_state/restore_state, unlike the prior
    // loop-on-VirtioFS path. The file lives in the VirtioFS share so the
    // host can introspect it while the VM is stopped, but the guest only
    // opens it via virtio-blk, never through the share.
    let system_img = guest_dir.join("system").join("rootfs.img");
    let machine_identifier_path = session_dir.join("machine_identifier");
    let serial_log_path = session_dir.join("serial.log");
    let (vm, vsock_rx, sm) = boot_vm(BootOptions {
        assets: &args.assets_dir,
        kernel_override: args.kernel.as_deref(),
        initrd_override: args.initrd.as_deref(),
        rootfs_override: Some(&args.rootfs),
        cmdline: "console=hvc0 ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1",
        system_overlay_disk: Some(&system_img),
        virtiofs_shares: &virtiofs_shares,
        cpu_count: args.cpus,
        ram_bytes: args.ram_mb * 1024 * 1024,
        checkpoint_path: args
            .checkpoint_path
            .clone()
            .map(|p| if p.is_absolute() { p } else { session_dir.join(p) }),
        machine_identifier_path: Some(&machine_identifier_path),
        serial_log_path: Some(&serial_log_path),
    })?;

    // Delete checkpoint file if we just restored from it, so we don't accidentally suspend on normal shutdown
    if let Some(cp) = &args.checkpoint_path {
        let full_path = if std::path::Path::new(cp).is_absolute() {
            std::path::PathBuf::from(cp)
        } else {
            session_dir.join(cp)
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

    let shutdown: Arc<Mutex<Shutdown>> = Arc::new(Mutex::new(Shutdown::default()));

    let trace_id_for_loop = trace_id.clone();
    let session_dir_for_loop = session_dir.clone();
    let shutdown_for_loop = Arc::clone(&shutdown);
    rt.spawn(async move {
        if let Err(e) = run_async_main_loop(
            args,
            vm_arc,
            vsock_rx,
            sm,
            trace_id_for_loop,
            session_dir_for_loop,
            shutdown_for_loop,
        )
        .await
        {
            error!("async loop failed: {e:#}");
            std::process::exit(1);
        }
    });

    // Signal-driven explicit cleanup. On SIGTERM/SIGINT, synchronously
    // drain the background-thread owners in the `Shutdown` struct
    // (FsMonitor -> DbWriter) BEFORE stopping the main run loop. Without
    // this, teardown relies on tokio-runtime-drop ordering and can miss
    // the service's 1s SIGKILL budget mid-checkpoint, leaving a dirty
    // `session.db-wal`. See /dev-rust-patterns "Signal-driven explicit
    // cleanup for background-thread owners".
    let shutdown_for_sig = Arc::clone(&shutdown);
    rt.spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let signal_name = tokio::select! {
            _ = sigterm.recv() => "SIGTERM",
            _ = sigint.recv() => "SIGINT",
        };
        tracing::warn!(
            signal = signal_name,
            "capsem-process received signal, draining background owners"
        );

        // Take the Shutdown struct out from under the async mutex so we
        // can hand it to `spawn_blocking` for the synchronous join. The
        // join itself blocks on thread handles, which we must not do on
        // a tokio worker.
        let mut owned = {
            let mut guard = shutdown_for_sig.lock().await;
            std::mem::take(&mut *guard)
        };
        let _ = tokio::task::spawn_blocking(move || {
            owned.drain_blocking();
        })
        .await;
        tracing::warn!(
            signal = signal_name,
            "background owners drained, stopping run loop"
        );

        #[cfg(target_os = "macos")]
        unsafe {
            core_foundation_sys::runloop::CFRunLoopStop(
                core_foundation_sys::runloop::CFRunLoopGetMain(),
            );
        }
    });

    #[cfg(target_os = "macos")]
    unsafe {
        core_foundation_sys::runloop::CFRunLoopRun();
    }
    #[cfg(not(target_os = "macos"))]
    rt.block_on(tokio::signal::ctrl_c())?;

    Ok(())
}

async fn run_async_main_loop(
    args: Args,
    vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    _sm: capsem_core::host_state::HostStateMachine,
    trace_id: String,
    session_dir: std::path::PathBuf,
    shutdown: Arc<Mutex<Shutdown>>,
) -> Result<()> {
    let job_store = Arc::new(JobStore::new());
    let (ipc_tx, _) = broadcast::channel::<ProcessToService>(128);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<ServiceToProcess>(32);
    let terminal_output = Arc::new(capsem_core::TerminalOutputQueue::new());

    let db = Arc::new(capsem_logger::DbWriter::open(
        &session_dir.join("session.db"),
        256,
    )?);
    // Register the DbWriter with the SIGTERM handler BEFORE any work that
    // produces writes. If the signal fires before the workspace monitor
    // starts, we still want a clean checkpoint.
    shutdown.lock().await.db = Some(Arc::clone(&db));

    let runtime_source = runtime_config::RuntimeProfileSource::new(args.active_profile.clone());
    let runtime_config = runtime_source.load()?;
    let security_rule_ids = runtime_config
        .security_rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<Vec<_>>();
    info!(
        profile_id = %runtime_config.profile_id,
        active_profile = %runtime_config.active_profile_path.display(),
        security_rule_count = security_rule_ids.len(),
        security_rule_ids = ?security_rule_ids,
        plugin_count = runtime_config.plugins.len(),
        dns_upstreams = ?runtime_config.dns_upstreams,
        "capsem-process loaded profile runtime config"
    );
    let guest_config = capsem_core::net::policy_config::GuestConfig::default();
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(
        runtime_config.security_rules.clone(),
    )));
    let plugin_policy = Arc::new(std::sync::RwLock::new(runtime_config.plugins.clone()));
    let model_trace_state = Arc::new(std::sync::Mutex::new(
        capsem_core::net::ai_traffic::TraceState::new(),
    ));

    // Start host file monitor to record fs_events.
    let workspace_dir = capsem_core::guest_share_dir(&session_dir).join("workspace");
    match capsem_core::fs_monitor::FsMonitor::start(
        workspace_dir.clone(),
        workspace_dir.clone(),
        Arc::clone(&db),
        Arc::clone(&security_rules),
        Arc::clone(&model_trace_state),
    ) {
        Ok(monitor) => {
            info!("host file monitor started");
            shutdown.lock().await.fs_monitor = Some(monitor);
        }
        Err(e) => {
            error!("failed to start host file monitor: {e}");
        }
    }

    let net_state = Arc::new(capsem_core::create_net_state_with_policy(
        &args.id,
        Arc::clone(&db),
        runtime_config.network.clone(),
    )?);
    // Locate the builtin MCP server binary next to our own binary.
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("capsem-mcp-builtin")));
    let mut builtin_env = std::collections::HashMap::new();
    builtin_env.insert(
        "CAPSEM_SESSION_DIR".into(),
        session_dir.to_string_lossy().to_string(),
    );
    let db_path = session_dir.join("session.db");
    builtin_env.insert(
        "CAPSEM_SESSION_DB".into(),
        db_path.to_string_lossy().to_string(),
    );
    builtin_env.insert(
        "CAPSEM_ACTIVE_PROFILE".into(),
        runtime_config
            .active_profile_path
            .to_string_lossy()
            .to_string(),
    );
    let mcp_servers = runtime_config.mcp_servers(builtin_bin.as_deref(), builtin_env.clone());
    let snap_auto_max = 10usize;
    let snap_manual_max = 12usize;
    let snap_interval = 300u64;

    let scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session_dir.clone(),
        snap_auto_max,
        snap_manual_max,
        std::time::Duration::from_secs(snap_interval),
    );
    let scheduler = Arc::new(tokio::sync::Mutex::new(scheduler));

    // Defer initial snapshot to background -- workspace is empty at boot, no need to block.
    {
        let sched = Arc::clone(&scheduler);
        tokio::spawn(async move {
            let mut s = sched.lock().await;
            if let Ok(slot) = s.take_snapshot() {
                info!(
                    slot = slot.slot,
                    files_count = slot.files_count,
                    origin = "auto",
                    "auto snapshot captured"
                );
            }
        });
    }

    // Spawn the isolated MCP aggregator subprocess.
    let aggregator_client =
        spawn_mcp_aggregator(&mcp_servers, &session_dir, &args.id, &trace_id).await?;

    // Persist the aggregator's discovered tool catalog to the cache file
    // so the service's GET /mcp/tools endpoint can serve it.
    if let Ok(tools) = aggregator_client.list_tools().await {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        // Merge with existing cache to preserve approval state.
        let existing = capsem_core::mcp::load_tool_cache();
        let cache_entries: Vec<capsem_core::mcp::ToolCacheEntry> = tools
            .iter()
            .map(|t| {
                let pin_hash = capsem_core::mcp::compute_tool_hash(t);
                let prev = existing
                    .iter()
                    .find(|e| e.namespaced_name == t.namespaced_name);
                capsem_core::mcp::ToolCacheEntry {
                    namespaced_name: t.namespaced_name.clone(),
                    original_name: t.original_name.clone(),
                    description: t.description.clone(),
                    server_name: t.server_name.clone(),
                    annotations: t.annotations.clone(),
                    pin_hash: pin_hash.clone(),
                    first_seen: prev
                        .map(|p| p.first_seen.clone())
                        .unwrap_or_else(|| now.clone()),
                    last_seen: now.clone(),
                    approved: prev
                        .map(|p| p.approved && p.pin_hash == pin_hash)
                        .unwrap_or(false),
                }
            })
            .collect();
        if let Err(e) = capsem_core::mcp::save_tool_cache(&cache_entries) {
            warn!(error = %e, "failed to write tool cache");
        } else {
            info!(tools = cache_entries.len(), "wrote tool cache");
        }
    }

    let inflight_cap = capsem_core::mcp::resolve_inflight_cap();
    info!(inflight_cap, "MITM MCP endpoint in-flight handler cap");
    let model_endpoints = Arc::new(std::sync::RwLock::new(Arc::new(
        runtime_config.model_endpoints.clone(),
    )));
    let mcp_inflight = Arc::new(tokio::sync::Semaphore::new(inflight_cap));
    let mcp_endpoint = Arc::new(capsem_core::net::mitm_proxy::McpEndpointState::new(
        aggregator_client.clone(),
        Arc::clone(&security_rules),
        Arc::clone(&plugin_policy),
        Arc::clone(&mcp_inflight),
        capsem_core::net::mitm_proxy::McpTimeouts::from_env(),
    ));
    let mcp_runtime = Arc::new(McpRuntime {
        aggregator: aggregator_client,
        endpoint: Arc::clone(&mcp_endpoint),
        db: Arc::clone(&db),
        security_rules: Arc::clone(&security_rules),
        plugin_policy: Arc::clone(&plugin_policy),
        model_endpoints: Arc::clone(&model_endpoints),
    });

    let telemetry_deps = Arc::new(
        capsem_core::net::mitm_proxy::telemetry_hook::TelemetryDeps {
            db: Arc::clone(&db),
            pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
            trace_state: Arc::clone(&model_trace_state),
            security_rules: Arc::clone(&security_rules),
            plugin_policy: Arc::clone(&plugin_policy),
        },
    );
    let mitm_pipeline = capsem_core::net::mitm_proxy::make_production_pipeline(
        Arc::clone(&net_state.policy),
        Arc::clone(&telemetry_deps),
    );
    let mitm_config = Arc::new(capsem_core::net::mitm_proxy::MitmProxyConfig {
        ca: Arc::clone(&net_state.ca),
        policy: Arc::clone(&net_state.policy),
        model_endpoints,
        db: Arc::clone(&db),
        upstream_tls: Arc::clone(&net_state.upstream_tls),
        telemetry: telemetry_deps,
        pipeline: mitm_pipeline,
        mcp_endpoint: Some(mcp_endpoint),
    });

    // DNS handler shares the same security rule/plugin handles as MITM
    // so admin enforcement edits take effect across protocols at once.
    let dns_resolver = if runtime_config.dns_upstreams.is_empty() {
        DnsResolver::new()
    } else {
        DnsResolver::with_upstreams(runtime_config.dns_upstreams.clone())
    };
    let dns_handler = Arc::new(capsem_core::net::dns::DnsHandler::with_cache(
        Arc::clone(&net_state.policy),
        Arc::clone(&security_rules),
        Arc::clone(&plugin_policy),
        Arc::new(dns_resolver),
        Arc::new(DnsAnswerCache::default()),
    ));

    let sched_clone = Arc::clone(&scheduler);
    tokio::spawn(async move {
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
            })
            .await;
            match result {
                Ok(Ok(slot)) => {
                    info!(
                        slot = slot.slot,
                        files_count = slot.files_count,
                        origin = "auto",
                        "auto snapshot captured"
                    );
                }
                Ok(Err(e)) => tracing::warn!("auto-snapshot failed: {e}"),
                Err(e) => tracing::warn!("auto-snapshot task panicked: {e}"),
            }
        }
    });

    let ipc_tx_clone = ipc_tx.clone();
    let job_store_clone = Arc::clone(&job_store);
    let terminal_output_clone = Arc::clone(&terminal_output);

    // Serial log is written by a thread attached inside the hypervisor's
    // boot() (before machine.start() spawns the reader), so no subscription
    // is needed here -- tokio::broadcast would race with VM resume and drop
    // the first ~100ms of post-resume output.

    let net_state_clone = Arc::clone(&net_state);
    let mitm_config_clone = Arc::clone(&mitm_config);
    let dns_handler_clone = Arc::clone(&dns_handler);

    // Parse --env KEY=VALUE pairs for guest injection
    let cli_env: Vec<(String, String)> = args
        .env
        .iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();

    let vm_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let ctrl_tx_ipc = ctrl_tx.clone();
    let uds_path = args.uds_path.clone();
    let vm_id_ws = args.id.clone();
    let is_restore = args.checkpoint_path.is_some();
    let vm_for_vsock = Arc::clone(&vm);
    let vm_ready_vsock = Arc::clone(&vm_ready);
    let uds_path_vsock = uds_path.clone();
    let db_for_vsock = Arc::clone(&db);
    let pty_log = match pty_log::PtyLog::open(&session_dir.join("pty.log")) {
        Ok(pl) => Some(Arc::new(pl)),
        Err(e) => {
            warn!("failed to open pty.log: {e}");
            None
        }
    };
    tokio::spawn(async move {
        if let Err(e) = vsock::setup_vsock(VsockOptions {
            vm_id: args.id.clone(),
            vm: vm_for_vsock,
            vsock_rx,
            ipc_tx: ipc_tx_clone,
            _ctrl_tx: ctrl_tx,
            ctrl_rx,
            terminal_output: terminal_output_clone,
            job_store: job_store_clone,
            session_dir: session_dir.clone(),
            cli_env,
            guest_config,
            mitm_config: mitm_config_clone,
            dns_handler: dns_handler_clone,
            security_rules: Arc::clone(&security_rules),
            plugin_policy: Arc::clone(&plugin_policy),
            _net_state: net_state_clone,
            is_restore,
            vm_ready: vm_ready_vsock,
            uds_path: uds_path_vsock,
            db: db_for_vsock,
            pty_log,
        })
        .await
        {
            // Handshake or other vsock setup failed. Without an explicit
            // exit, capsem-process keeps running with no .ready sentinel
            // and no working control channel -- the service sees no exit,
            // polls .ready for 30s, and every command times out. Exiting
            // here lets the service's child-exit handler clean up the
            // instance promptly so the caller (test, CLI, MCP) sees the
            // failure in <1s instead of 30s.
            error!("vsock failed: {e:#}");
            std::process::exit(1);
        }
    });

    if uds_path.exists() {
        std::fs::remove_file(&uds_path)?;
    }
    let listener = UnixListener::bind(&uds_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!(socket = %uds_path.display(), "listening for IPC (mode 0600)");

    let ws_sock_path = uds_path.with_file_name(format!("{}-ws.sock", vm_id_ws));
    if ws_sock_path.exists() {
        std::fs::remove_file(&ws_sock_path)?;
    }
    let ws_listener = tokio::net::UnixListener::bind(&ws_sock_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ws_sock_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!(socket = %ws_sock_path.display(), "listening for terminal WS (mode 0600)");

    // Terminal relay: fan-out broadcast + ring buffer so a newly-connecting
    // WS client sees the shell's startup banner (printed before it joined).
    let term_relay = terminal::TerminalRelay::new(1024);
    let term_c_bcast = Arc::clone(&terminal_output);
    let term_relay_pump = Arc::clone(&term_relay);
    tokio::spawn(async move {
        while let Some(data) = term_c_bcast.poll().await {
            term_relay_pump.publish(data);
        }
    });

    let ctrl_tx_ws = ctrl_tx_ipc.clone();
    let term_relay_app = Arc::clone(&term_relay);

    let ws_app = axum::Router::new().route(
        "/terminal",
        axum::routing::get(move |ws: axum::extract::ws::WebSocketUpgrade| {
            let ctrl_tx = ctrl_tx_ws.clone();
            let (replay, term_rx) = term_relay_app.subscribe();
            async move {
                ws.on_upgrade(move |socket| {
                    terminal::handle_terminal_socket(socket, ctrl_tx, replay, term_rx)
                })
            }
        }),
    );

    tokio::spawn(async move {
        if let Err(e) = axum::serve(ws_listener, ws_app).await {
            error!("WS server error: {}", e);
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let tx_c = ctrl_tx_ipc.clone();
        let ipc_tx_pass = ipc_tx.clone();
        let term_c = Arc::clone(&term_relay);
        let job_c = Arc::clone(&job_store);
        let net_c = Arc::clone(&net_state);
        let mcp_c = Arc::clone(&mcp_runtime);
        let runtime_source_c = runtime_source.clone();
        let builtin_bin_c = builtin_bin.clone();
        let builtin_env_c = builtin_env.clone();
        let sched_c = Arc::clone(&scheduler);
        let ready_c = Arc::clone(&vm_ready);

        tokio::spawn(async move {
            if let Err(e) = ipc::handle_ipc_connection(
                stream,
                tx_c,
                ipc_tx_pass,
                term_c,
                job_c,
                net_c,
                mcp_c,
                runtime_source_c,
                builtin_bin_c,
                builtin_env_c,
                sched_c,
                ready_c,
            )
            .await
            {
                error!("IPC error: {e:#}");
            }
        });
    }
}

/// Spawn the isolated MCP aggregator subprocess and return a client handle.
///
/// The subprocess manages connections to external MCP servers. It communicates
/// via length-prefixed MessagePack frames on stdin/stdout.
///
/// Frame format: [4 bytes big-endian payload length] [N bytes msgpack]
async fn spawn_mcp_aggregator(
    servers: &[capsem_core::mcp::types::McpServerDef],
    session_dir: &Path,
    vm_id: &str,
    trace_id: &str,
) -> Result<capsem_core::mcp::aggregator::AggregatorClient> {
    use capsem_core::mcp::aggregator::*;
    use std::collections::HashMap;

    let (client, mut rx) = AggregatorClient::channel(64);

    let exe_path = std::env::current_exe()?;
    let aggregator_bin = resolve_mcp_aggregator_binary(&exe_path)?;

    // Dedicated stderr log for the aggregator -- keeps its JSON tracing
    // stream out of the parent's process.log. 0o600 to match the
    // project's sensitive-log permissions policy (see
    // /dev-rust-patterns lesson 14).
    let log_path = aggregator_log_path(session_dir);
    let stderr_file = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&log_path)
                .with_context(|| format!("failed to open {}", log_path.display()))?
        }
        #[cfg(not(unix))]
        {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .with_context(|| format!("failed to open {}", log_path.display()))?
        }
    };

    info!(
        bin = %aggregator_bin.display(),
        servers = servers.len(),
        log = %log_path.display(),
        "spawning MCP aggregator"
    );

    let mut cmd = tokio::process::Command::new(&aggregator_bin);
    // W4: include CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE.
    // Caller already has `trace_id` from the root span; we re-derive via
    // child_trace_env so the aggregator inherits this process's parent
    // traceparent verbatim instead of getting a freshly-synthesized one.
    for (k, v) in capsem_core::telemetry::child_trace_env(vm_id) {
        cmd.env(k, v);
    }
    // Keep the pre-W4 CAPSEM_TRACE_ID override path so callers that
    // pass an explicit trace_id (the root span's value) still win over
    // the env-derived id. Belt-and-suspenders for the aggregator's
    // structured root span.
    cmd.env("CAPSEM_TRACE_ID", trace_id);
    let mut child = cmd
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--lock-path")
        .arg(session_dir.join("mcp-aggregator.lock"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()?;

    let mut child_stdin = child.stdin.take().unwrap();
    let mut child_stdout = child.stdout.take().unwrap();

    // Send server definitions as the first frame.
    let defs_vec = servers.to_vec();
    write_frame(&mut child_stdin, &defs_vec).await?;

    // Background driver: reads from client channel, writes to subprocess stdin,
    // reads responses from subprocess stdout, routes back to callers.
    let pending: Arc<
        tokio::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<AggregatorResponse>>>,
    > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Reader task: reads msgpack frames from subprocess stdout and routes to pending callers.
    let pending_reader = Arc::clone(&pending);
    tokio::spawn(async move {
        info!("aggregator reader task started");
        loop {
            match read_frame::<_, AggregatorResponse>(&mut child_stdout).await {
                Ok(Some(resp)) => {
                    let mut map = pending_reader.lock().await;
                    if let Some(tx) = map.remove(&resp.id) {
                        capsem_core::try_send!("aggregator_oneshot", tx.send(resp));
                    }
                }
                Ok(None) => {
                    info!("aggregator stdout closed (EOF)");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "failed to read aggregator response frame");
                    break;
                }
            }
        }
        info!("aggregator reader task ending");
    });

    // Writer task: reads from client channel, writes msgpack frames to subprocess stdin.
    let pending_writer = Arc::clone(&pending);
    tokio::spawn(async move {
        info!("aggregator writer task started");
        while let Some((req, resp_tx)) = rx.recv().await {
            {
                let mut map = pending_writer.lock().await;
                map.insert(req.id, resp_tx);
            }
            if let Err(e) = write_frame(&mut child_stdin, &req).await {
                error!(error = %e, "failed to write aggregator request frame");
                info!("aggregator writer task ending due to write error");
                break;
            }
        }
        info!("aggregator writer task ending (channel closed or break)");
    });

    // Monitor child process.
    tokio::spawn(async move {
        info!("aggregator monitor task started");
        match child.wait().await {
            Ok(status) => info!(status = %status, "aggregator subprocess exited"),
            Err(e) => error!(error = %e, "failed to wait on aggregator"),
        }
    });

    Ok(client)
}

fn resolve_mcp_aggregator_binary(exe_path: &Path) -> Result<PathBuf> {
    let bin_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
    let mut candidates = vec![bin_dir.join("capsem-mcp-aggregator")];
    if bin_dir.file_name().and_then(|name| name.to_str()) == Some("deps") {
        if let Some(target_debug) = bin_dir.parent() {
            candidates.push(target_debug.join("capsem-mcp-aggregator"));
        }
    }

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    let searched = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "required MCP aggregator binary capsem-mcp-aggregator is missing; searched: {searched}"
    )
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
            "--id",
            "test-vm",
            "--assets-dir",
            "/tmp/assets",
            "--rootfs",
            "/tmp/rootfs.img",
            "--session-dir",
            "/tmp/session",
            "--active-profile",
            "/tmp/config/profiles/code",
            "--uds-path",
            "/tmp/vm.sock",
        ])
        .unwrap();
        assert_eq!(args.id, "test-vm");
        assert_eq!(args.assets_dir, PathBuf::from("/tmp/assets"));
        assert_eq!(args.rootfs, PathBuf::from("/tmp/rootfs.img"));
        assert_eq!(args.session_dir, PathBuf::from("/tmp/session"));
        assert_eq!(
            args.active_profile,
            PathBuf::from("/tmp/config/profiles/code")
        );
        assert_eq!(args.uds_path, PathBuf::from("/tmp/vm.sock"));
    }

    #[test]
    fn args_default_cpus() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ])
        .unwrap();
        assert_eq!(args.cpus, 2);
    }

    #[test]
    fn args_default_ram_mb() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ])
        .unwrap();
        assert_eq!(args.ram_mb, 2048);
    }

    #[test]
    fn args_default_scratch_disk_size_gb() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ])
        .unwrap();
        assert_eq!(args.scratch_disk_size_gb, 16);
    }

    #[test]
    fn args_custom_cpus_ram_and_scratch_disk_size() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
            "--cpus",
            "8",
            "--ram-mb",
            "16384",
            "--scratch-disk-size-gb",
            "64",
        ])
        .unwrap();
        assert_eq!(args.cpus, 8);
        assert_eq!(args.ram_mb, 16384);
        assert_eq!(args.scratch_disk_size_gb, 64);
    }

    #[test]
    fn prepare_session_layout_uses_requested_scratch_disk_size() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("session");

        let guest_dir = prepare_session_layout(&session_dir, 64).unwrap();

        assert_eq!(guest_dir, session_dir.join("guest"));
        let rootfs_img = guest_dir.join("system/rootfs.img");
        let metadata = std::fs::metadata(&rootfs_img).unwrap();
        assert_eq!(metadata.len(), 64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn args_missing_required_id_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_missing_required_assets_dir_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_missing_required_active_profile_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--uds-path",
            "/u",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_invalid_cpus_type_fails() {
        let result = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
            "--cpus",
            "not-a-number",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn args_checkpoint_path_optional() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
        ])
        .unwrap();
        assert!(args.checkpoint_path.is_none());
    }

    #[test]
    fn args_checkpoint_path_set() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
            "--checkpoint-path",
            "/tmp/cp.vzsave",
        ])
        .unwrap();
        assert_eq!(
            args.checkpoint_path.unwrap(),
            PathBuf::from("/tmp/cp.vzsave")
        );
    }

    #[test]
    fn args_env_vars_parsed() {
        let args = Args::try_parse_from([
            "capsem-process",
            "--id",
            "vm",
            "--assets-dir",
            "/a",
            "--rootfs",
            "/r",
            "--session-dir",
            "/s",
            "--active-profile",
            "/profiles/code",
            "--uds-path",
            "/u",
            "--env",
            "FOO=bar",
            "--env",
            "BAZ=qux",
        ])
        .unwrap();
        assert_eq!(args.env, vec!["FOO=bar", "BAZ=qux"]);
    }

    // -----------------------------------------------------------------------
    // CLI env parsing (used in run_async_main_loop)
    // -----------------------------------------------------------------------

    #[test]
    fn cli_env_parsing_valid() {
        let env = ["FOO=bar".to_string(), "BAZ=qux=extra".to_string()];
        let parsed: Vec<(String, String)> = env
            .iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect();
        assert_eq!(
            parsed,
            vec![
                ("FOO".to_string(), "bar".to_string()),
                ("BAZ".to_string(), "qux=extra".to_string()),
            ]
        );
    }

    #[test]
    fn cli_env_parsing_no_equals_skipped() {
        let env = ["NOEQ".to_string(), "GOOD=val".to_string()];
        let parsed: Vec<(String, String)> = env
            .iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ("GOOD".to_string(), "val".to_string()));
    }

    #[test]
    fn cli_env_parsing_empty_value() {
        let env = ["KEY=".to_string()];
        let parsed: Vec<(String, String)> = env
            .iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect();
        assert_eq!(parsed, vec![("KEY".to_string(), "".to_string())]);
    }

    // -----------------------------------------------------------------------
    // trace_id generation: stitches together the three host-side processes
    // (capsem-service, capsem-process, capsem-mcp-aggregator) so
    // per-VM logs can be correlated across the process.log + the new
    // mcp-aggregator.stderr.log streams.
    // -----------------------------------------------------------------------

    #[test]
    fn generate_trace_id_is_16_hex_chars() {
        let id = generate_trace_id();
        assert_eq!(id.len(), 16);
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex character in trace_id: {id}"
        );
    }

    #[test]
    fn generate_trace_id_is_unique_across_calls() {
        // Not cryptographic -- but within a process, rapid successive
        // calls must not collide, otherwise correlation is useless.
        use std::collections::HashSet;
        let ids: HashSet<String> = (0..64).map(|_| generate_trace_id()).collect();
        assert_eq!(ids.len(), 64, "trace_id collisions: {ids:?}");
    }

    #[test]
    fn aggregator_log_path_lives_in_session_dir() {
        let session = PathBuf::from("/tmp/some-session");
        let log = aggregator_log_path(&session);
        assert_eq!(log, session.join("mcp-aggregator.stderr.log"));
    }

    #[test]
    fn missing_mcp_aggregator_fails_loud_instead_of_empty_stub() {
        let dir = tempfile::tempdir().unwrap();
        let fake_exe = dir.path().join("capsem-process");
        let error = resolve_mcp_aggregator_binary(&fake_exe)
            .expect_err("missing aggregator binary must not resolve");
        assert!(
            error.to_string().contains("capsem-mcp-aggregator"),
            "error should name the missing component: {error:#}"
        );
    }

    #[test]
    fn mcp_aggregator_resolver_supports_cargo_test_deps_layout() {
        let dir = tempfile::tempdir().unwrap();
        let deps = dir.path().join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let aggregator = dir.path().join("capsem-mcp-aggregator");
        std::fs::write(&aggregator, "").unwrap();

        let resolved = resolve_mcp_aggregator_binary(&deps.join("capsem-process-test")).unwrap();
        assert_eq!(resolved, aggregator);
    }
}
