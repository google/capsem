mod helpers;
mod ipc;
mod job_store;
mod mcp_runtime;
mod metrics_debug;
mod pty_log;
mod terminal;
mod vsock;

use anyhow::{Context, Result};
use capsem_core::fs_monitor::FsMonitor;
use capsem_core::{boot_vm, BootOptions, VirtioFsShare, VsockConnection};
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use helpers::query_max_fs_event_id;
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
    expected_kernel_hash: Option<String>,
    #[arg(long)]
    expected_initrd_hash: Option<String>,
    #[arg(long)]
    expected_rootfs_hash: Option<String>,
    #[arg(long)]
    session_dir: PathBuf,
    #[arg(long, default_value_t = 2)]
    cpus: u32,
    #[arg(long, default_value_t = 2048)]
    ram_mb: u64,
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

const AGGREGATOR_PARENT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "RUST_LOG",
    "RUST_BACKTRACE",
    "CAPSEM_METRICS_DEBUG_INTERVAL_SECS",
];

fn process_kernel_cmdline() -> String {
    let append = if cfg!(debug_assertions) {
        std::env::var("CAPSEM_DEV_KERNEL_CMDLINE_APPEND").ok()
    } else {
        None
    };
    process_kernel_cmdline_with_append(append.as_deref())
}

fn process_kernel_cmdline_with_append(append: Option<&str>) -> String {
    #[cfg(target_arch = "x86_64")]
    let base = "console=ttyS0 root=/dev/vda ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1 capsem.storage=virtiofs";
    #[cfg(target_arch = "aarch64")]
    let base = "console=hvc0 root=/dev/vda ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1 capsem.storage=virtiofs";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let base = "console=hvc0 root=/dev/vda ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1 capsem.storage=virtiofs";

    match append.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!("{base} {extra}"),
        None => base.to_string(),
    }
}

fn aggregator_parent_env_from<F>(lookup: F) -> std::collections::HashMap<String, String>
where
    F: Fn(&str) -> Option<String>,
{
    AGGREGATOR_PARENT_ENV_ALLOWLIST
        .iter()
        .filter_map(|key| lookup(key).map(|value| ((*key).to_string(), value)))
        .collect()
}

fn aggregator_child_env(vm_id: &str, trace_id: &str) -> std::collections::HashMap<String, String> {
    let mut env = aggregator_parent_env_from(|key| std::env::var(key).ok());
    for (k, v) in capsem_core::telemetry::child_trace_env(vm_id) {
        env.insert(k, v);
    }
    for key in [
        capsem_core::telemetry::CAPSEM_SESSION_ID_ENV,
        capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV,
        capsem_core::telemetry::CAPSEM_PROFILE_REVISION_ENV,
        capsem_core::telemetry::CAPSEM_USER_ID_ENV,
    ] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    env.insert("CAPSEM_TRACE_ID".to_string(), trace_id.to_string());
    env
}

fn main() -> Result<()> {
    if capsem_core::build_info::maybe_print_json_and_exit("capsem-process")? {
        return Ok(());
    }
    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-process",
        sink: capsem_core::telemetry::LogSink::Stderr,
        default_filter: "info",
    })?;
    let _metrics_debug_guard = metrics_debug::MetricsDebugGuard::maybe_start();
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

    capsem_core::create_virtiofs_session(&session_dir, 2)?;
    let guest_dir = capsem_core::guest_share_dir(&session_dir);
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
    let kernel_cmdline = process_kernel_cmdline();
    let (vm, vsock_rx, sm) = boot_vm(BootOptions {
        assets: &args.assets_dir,
        kernel_override: args.kernel.as_deref(),
        initrd_override: args.initrd.as_deref(),
        rootfs_override: Some(&args.rootfs),
        expected_kernel_hash: args.expected_kernel_hash.as_deref(),
        expected_initrd_hash: args.expected_initrd_hash.as_deref(),
        expected_rootfs_hash: args.expected_rootfs_hash.as_deref(),
        cmdline: &kernel_cmdline,
        system_overlay_disk: Some(&system_img),
        virtiofs_shares: &virtiofs_shares,
        cpu_count: args.cpus,
        ram_bytes: args.ram_mb * 1024 * 1024,
        checkpoint_path: args.checkpoint_path.clone().map(|p| {
            if p.is_absolute() {
                p
            } else {
                session_dir.join(p)
            }
        }),
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
    let (signal_exit_tx, _signal_exit_rx) = tokio::sync::oneshot::channel::<()>();
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
        let _ = signal_exit_tx.send(());

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
    {
        let _ = rt.block_on(_signal_exit_rx);
    }

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

    let runtime_rule_matches = mcp_runtime::RuntimeRuleMatchAccumulator::default();
    let runtime_policy = mcp_runtime::load_runtime_policy_state_with_runtime_rules_and_recorder(
        &session_dir,
        None,
        Some(runtime_rule_matches.clone()),
    );
    if let Ok(env_profile_id) = std::env::var(capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV) {
        if env_profile_id != runtime_policy.profile_id {
            warn!(
                env_profile_id,
                effective_profile_id = %runtime_policy.profile_id,
                "process telemetry profile identity differed from attached vm-effective settings"
            );
        }
    }
    let user_id = capsem_core::telemetry::host_user_id();
    db.write(capsem_logger::WriteOp::TelemetryIdentity(
        capsem_logger::TelemetryIdentity {
            timestamp: std::time::SystemTime::now(),
            vm_id: args.id.clone(),
            profile_id: runtime_policy.profile_id.clone(),
            user_id: user_id.clone(),
        },
    ))
    .await;
    info!(
        vm_id = %args.id,
        profile_id = %runtime_policy.profile_id,
        user_id = %user_id,
        "session telemetry identity attached"
    );

    // Start host file monitor to record fs_events.
    let workspace_dir = session_dir.join("workspace");
    match capsem_core::fs_monitor::FsMonitor::start(
        workspace_dir.clone(),
        workspace_dir.clone(),
        Arc::clone(&db),
    ) {
        Ok(monitor) => {
            info!("host file monitor started");
            shutdown.lock().await.fs_monitor = Some(monitor);
        }
        Err(e) => {
            error!("failed to start host file monitor: {e}");
        }
    }

    let guest_config = runtime_policy.guest_config.clone();

    let net_state = Arc::new(capsem_core::create_net_state(&args.id, Arc::clone(&db))?);
    // Locate the builtin MCP server binary next to our own binary.
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("capsem-mcp-builtin")));
    let mcp_servers = mcp_runtime::build_servers_with_builtin(
        &runtime_policy.mcp_user,
        &runtime_policy.mcp_corp,
        builtin_bin.as_deref(),
        &session_dir,
        &runtime_policy.domain_policy,
    );
    let snap_auto_max = runtime_policy.snapshot_auto_max;
    let snap_manual_max = runtime_policy.snapshot_manual_max;
    let snap_interval = runtime_policy.snapshot_interval_secs;

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
        let db_snap = Arc::clone(&db);
        tokio::spawn(async move {
            let mut s = sched.lock().await;
            if let Ok(slot) = s.take_snapshot() {
                let stop_id = query_max_fs_event_id(&db_snap);
                db_snap
                    .write(capsem_logger::WriteOp::SnapshotEvent(
                        capsem_logger::SnapshotEvent {
                            timestamp: slot.timestamp,
                            slot: slot.slot,
                            origin: "auto".into(),
                            name: None,
                            files_count: slot.files_count,
                            start_fs_event_id: 0,
                            stop_fs_event_id: stop_id,
                            trace_id: capsem_core::telemetry::ambient_capsem_trace_id(),
                        },
                    ))
                    .await;
            }
        });
    }

    // Spawn the isolated MCP aggregator subprocess.
    let aggregator_client =
        spawn_mcp_aggregator(&mcp_servers, &session_dir, &args.id, &trace_id).await?;

    // Persist the aggregator's discovered tool catalog to the cache file
    // for runtime diagnostics and policy reload accounting.
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
    let mcp_policy = Arc::new(tokio::sync::RwLock::new(Arc::new(
        runtime_policy.mcp_policy.clone(),
    )));
    let mcp_domain_policy = Arc::new(std::sync::RwLock::new(Arc::new(
        runtime_policy.domain_policy.clone(),
    )));
    let runtime_security_engine = Arc::new(
        capsem_core::net::mitm_proxy::RuntimeSecurityEngineSlot::new(
            runtime_policy.security_engine.clone(),
        ),
    );
    let mcp_inflight = Arc::new(tokio::sync::Semaphore::new(inflight_cap));
    let mcp_endpoint = Arc::new(capsem_core::net::mitm_proxy::McpEndpointState::new(
        aggregator_client.clone(),
        Arc::clone(&mcp_policy),
        Arc::clone(&runtime_security_engine),
        Arc::clone(&mcp_inflight),
        capsem_core::net::mitm_proxy::McpTimeouts::from_env(),
    ));
    let mcp_runtime = Arc::new(McpRuntime {
        aggregator: aggregator_client,
        policy: Arc::clone(&mcp_policy),
        domain_policy: Arc::clone(&mcp_domain_policy),
        security_engine: Arc::clone(&runtime_security_engine),
        rule_matches: runtime_rule_matches,
        session_dir: session_dir.clone(),
        builtin_binary: builtin_bin,
    });

    let telemetry_deps = Arc::new(
        capsem_core::net::mitm_proxy::telemetry_hook::TelemetryDeps {
            db: Arc::clone(&db),
            pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
            trace_state: Arc::new(std::sync::Mutex::new(
                capsem_core::net::ai_traffic::TraceState::new(),
            )),
        },
    );
    let mitm_pipeline =
        capsem_core::net::mitm_proxy::make_production_pipeline(Arc::clone(&telemetry_deps));
    let mitm_config = Arc::new(capsem_core::net::mitm_proxy::MitmProxyConfig {
        ca: Arc::clone(&net_state.ca),
        db: Arc::clone(&db),
        upstream_tls: Arc::clone(&net_state.upstream_tls),
        telemetry: telemetry_deps,
        pipeline: mitm_pipeline,
        security_engine: runtime_security_engine,
        mcp_endpoint: Some(mcp_endpoint),
    });

    let dns_handler = Arc::new(capsem_core::net::dns::DnsHandler::with_default_resolver());

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
            })
            .await;
            match result {
                Ok(Ok(slot)) => {
                    let stop_id = query_max_fs_event_id(&db_clone);
                    db_clone
                        .write(capsem_logger::WriteOp::SnapshotEvent(
                            capsem_logger::SnapshotEvent {
                                timestamp: slot.timestamp,
                                slot: slot.slot,
                                origin: "auto".into(),
                                name: None,
                                files_count: slot.files_count,
                                start_fs_event_id: last_stop,
                                stop_fs_event_id: stop_id,
                                trace_id: capsem_core::telemetry::ambient_capsem_trace_id(),
                            },
                        ))
                        .await;
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
    let vm_id_for_vsock = args.id.clone();
    let session_dir_for_vsock = session_dir.clone();
    let pty_log = match pty_log::PtyLog::open(&session_dir.join("pty.log")) {
        Ok(pl) => Some(Arc::new(pl)),
        Err(e) => {
            warn!("failed to open pty.log: {e}");
            None
        }
    };
    tokio::spawn(async move {
        if let Err(e) = vsock::setup_vsock(VsockOptions {
            vm_id: vm_id_for_vsock,
            vm: vm_for_vsock,
            vsock_rx,
            ipc_tx: ipc_tx_clone,
            _ctrl_tx: ctrl_tx,
            ctrl_rx,
            terminal_output: terminal_output_clone,
            job_store: job_store_clone,
            session_dir: session_dir_for_vsock,
            cli_env,
            guest_config,
            mitm_config: mitm_config_clone,
            dns_handler: dns_handler_clone,
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
        let mcp_c = Arc::clone(&mcp_runtime);
        let db_c = Arc::clone(&db);
        let ready_c = Arc::clone(&vm_ready);
        let vm_c = Arc::clone(&vm);
        let vm_id_c = vm_id_ws.clone();
        let resource_metrics = ipc::ResourceMetricsContext {
            configured_vcpus: args.cpus,
            configured_ram_mb: args.ram_mb,
        };

        tokio::spawn(async move {
            if let Err(e) = ipc::handle_ipc_connection(
                stream,
                tx_c,
                ipc_tx_pass,
                term_c,
                job_c,
                mcp_c,
                db_c,
                ready_c,
                vm_c,
                vm_id_c,
                resource_metrics,
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
///
/// If the aggregator binary is not found (dev builds), falls back to an in-process
/// mock that returns empty results.
async fn spawn_mcp_aggregator(
    servers: &[capsem_core::mcp::types::McpServerDef],
    session_dir: &Path,
    vm_id: &str,
    trace_id: &str,
) -> Result<capsem_core::mcp::aggregator::AggregatorClient> {
    use capsem_core::mcp::aggregator::*;
    use std::collections::HashMap;

    let (client, mut rx) = AggregatorClient::channel(64);

    // Find the aggregator binary next to our own binary.
    let exe_path = std::env::current_exe()?;
    let bin_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
    let aggregator_bin = bin_dir.join("capsem-mcp-aggregator");

    if !aggregator_bin.exists() {
        // Dev fallback: no aggregator binary. Return a client with an empty mock driver.
        info!(
            "aggregator binary not found at {}, using empty stub",
            aggregator_bin.display()
        );
        tokio::spawn(async move {
            while let Some((req, _enqueued_at, resp_tx)) = rx.recv().await {
                let body = match req.method {
                    AggregatorMethod::ListServers => AggregatorResult::Servers { servers: vec![] },
                    AggregatorMethod::ListTools => AggregatorResult::Tools { tools: vec![] },
                    AggregatorMethod::ListResources => {
                        AggregatorResult::Resources { resources: vec![] }
                    }
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
                capsem_core::try_send!(
                    "aggregator_response",
                    resp_tx.send(AggregatorResponse { id: req.id, body })
                );
            }
        });
        return Ok(client);
    }

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
    cmd.env_clear();
    // W4: include CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE.
    // Keep PATH/RUST_LOG/RUST_BACKTRACE as the explicit execution/logging
    // surface; config override paths and ambient provider tokens do not cross
    // into the aggregator.
    for (k, v) in aggregator_child_env(vm_id, trace_id) {
        cmd.env(k, v);
    }
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
    struct PendingAggregatorRequest {
        tx: tokio::sync::oneshot::Sender<AggregatorResponse>,
        method_kind: &'static str,
        tool_kind: &'static str,
    }

    let pending: Arc<tokio::sync::Mutex<HashMap<u64, PendingAggregatorRequest>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Reader task: reads msgpack frames from subprocess stdout and routes to pending callers.
    let pending_reader = Arc::clone(&pending);
    tokio::spawn(async move {
        info!("aggregator reader task started");
        loop {
            let read_started = std::time::Instant::now();
            match read_frame_payload(&mut child_stdout).await {
                Ok(Some(payload)) => {
                    let decode_started = std::time::Instant::now();
                    let resp = match decode_frame_payload::<AggregatorResponse>(&payload) {
                        Ok(resp) => resp,
                        Err(e) => {
                            error!(error = %e, "failed to decode aggregator response frame");
                            continue;
                        }
                    };
                    let mut map = pending_reader.lock().await;
                    if let Some(pending) = map.remove(&resp.id) {
                        let result = match &resp.body {
                            AggregatorResult::Error { .. } => "error",
                            _ => "ok",
                        };
                        record_aggregator_client_stage_metric(
                            read_started,
                            "response_frame_read",
                            pending.method_kind,
                            pending.tool_kind,
                            result,
                        );
                        record_aggregator_client_stage_metric(
                            decode_started,
                            "response_msgpack_decode",
                            pending.method_kind,
                            pending.tool_kind,
                            result,
                        );
                        let route_started = std::time::Instant::now();
                        capsem_core::try_send!("aggregator_oneshot", pending.tx.send(resp));
                        record_aggregator_client_stage_metric(
                            route_started,
                            "response_route",
                            pending.method_kind,
                            pending.tool_kind,
                            result,
                        );
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
        while let Some((req, enqueued_at, resp_tx)) = rx.recv().await {
            let method_kind = req.method.metric_label();
            let tool_kind = req.method.tool_kind_label();
            record_aggregator_client_stage_metric(
                enqueued_at,
                "driver_queue_wait",
                method_kind,
                tool_kind,
                "ok",
            );
            {
                let mut map = pending_writer.lock().await;
                map.insert(
                    req.id,
                    PendingAggregatorRequest {
                        tx: resp_tx,
                        method_kind,
                        tool_kind,
                    },
                );
            }
            let encode_started = std::time::Instant::now();
            let payload = match encode_frame_payload(&req) {
                Ok(payload) => payload,
                Err(e) => {
                    record_aggregator_client_stage_metric(
                        encode_started,
                        "request_msgpack_encode",
                        method_kind,
                        tool_kind,
                        "error",
                    );
                    error!(error = %e, "failed to encode aggregator request frame");
                    continue;
                }
            };
            record_aggregator_client_stage_metric(
                encode_started,
                "request_msgpack_encode",
                method_kind,
                tool_kind,
                "ok",
            );
            let write_started = std::time::Instant::now();
            if let Err(e) = write_frame_payload(&mut child_stdin, &payload).await {
                record_aggregator_client_stage_metric(
                    write_started,
                    "request_frame_write",
                    method_kind,
                    tool_kind,
                    "error",
                );
                error!(error = %e, "failed to write aggregator request frame");
                info!("aggregator writer task ending due to write error");
                break;
            }
            record_aggregator_client_stage_metric(
                write_started,
                "request_frame_write",
                method_kind,
                tool_kind,
                "ok",
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
            "--uds-path",
            "/tmp/vm.sock",
        ])
        .unwrap();
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
            "--uds-path",
            "/u",
        ])
        .unwrap();
        assert_eq!(args.ram_mb, 2048);
    }

    #[test]
    fn args_custom_cpus_and_ram() {
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
            "--uds-path",
            "/u",
            "--cpus",
            "8",
            "--ram-mb",
            "16384",
        ])
        .unwrap();
        assert_eq!(args.cpus, 8);
        assert_eq!(args.ram_mb, 16384);
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
    fn process_kernel_cmdline_has_root_disk_and_arch_console() {
        let cmdline = process_kernel_cmdline_with_append(None);
        assert!(cmdline.contains(" root=/dev/vda "));
        assert!(cmdline.contains(" capsem.storage=virtiofs"));
        #[cfg(target_arch = "x86_64")]
        assert!(cmdline.starts_with("console=ttyS0 "));
        #[cfg(target_arch = "aarch64")]
        assert!(cmdline.starts_with("console=hvc0 "));
    }

    #[test]
    fn process_kernel_cmdline_can_append_dev_diagnostics() {
        let cmdline = process_kernel_cmdline_with_append(Some(" ignore_loglevel loglevel=7 "));
        assert!(cmdline.ends_with("ignore_loglevel loglevel=7"));
        assert!(cmdline.contains(" capsem.storage=virtiofs "));
    }

    #[test]
    fn aggregator_parent_env_allows_execution_and_logging_only() {
        let mut source = std::collections::HashMap::new();
        source.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        source.insert("RUST_LOG".to_string(), "capsem=debug".to_string());
        source.insert("RUST_BACKTRACE".to_string(), "1".to_string());
        source.insert(
            "CAPSEM_METRICS_DEBUG_INTERVAL_SECS".to_string(),
            "2".to_string(),
        );
        source.insert("CAPSEM_HOME".to_string(), "/tmp/capsem-home".to_string());
        source.insert(
            "CAPSEM_SERVICE_SETTINGS".to_string(),
            "/tmp/service.toml".to_string(),
        );
        source.insert(
            "CAPSEM_TEST_UPSTREAM_OVERRIDES".to_string(),
            "leak".to_string(),
        );
        source.insert("OPENAI_API_KEY".to_string(), "secret".to_string());

        let env = aggregator_parent_env_from(|key| source.get(key).cloned());

        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin:/bin"));
        assert_eq!(
            env.get("RUST_LOG").map(String::as_str),
            Some("capsem=debug")
        );
        assert_eq!(env.get("RUST_BACKTRACE").map(String::as_str), Some("1"));
        assert_eq!(
            env.get("CAPSEM_METRICS_DEBUG_INTERVAL_SECS")
                .map(String::as_str),
            Some("2")
        );
        assert!(!env.contains_key("CAPSEM_USER_CONFIG"));
        assert!(!env.contains_key("CAPSEM_CORP_CONFIG"));
        assert!(!env.contains_key("CAPSEM_TEST_UPSTREAM_OVERRIDES"));
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn aggregator_child_env_preserves_runtime_identity() {
        let _guard = ENV_LOCK.lock().unwrap();
        let keys = [
            capsem_core::telemetry::CAPSEM_SESSION_ID_ENV,
            capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV,
            capsem_core::telemetry::CAPSEM_PROFILE_REVISION_ENV,
            capsem_core::telemetry::CAPSEM_USER_ID_ENV,
        ];
        let previous: Vec<(&str, Option<String>)> = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        std::env::set_var(capsem_core::telemetry::CAPSEM_SESSION_ID_ENV, "session-1");
        std::env::set_var(capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV, "coding");
        std::env::set_var(
            capsem_core::telemetry::CAPSEM_PROFILE_REVISION_ENV,
            "2026.0522.1",
        );
        std::env::set_var(capsem_core::telemetry::CAPSEM_USER_ID_ENV, "sansa");

        let env = aggregator_child_env("vm-1", "trace-1");

        for (key, value) in previous {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }

        assert_eq!(
            env.get(capsem_core::telemetry::CAPSEM_SESSION_ID_ENV)
                .map(String::as_str),
            Some("session-1")
        );
        assert_eq!(
            env.get(capsem_core::telemetry::CAPSEM_PROFILE_ID_ENV)
                .map(String::as_str),
            Some("coding")
        );
        assert_eq!(
            env.get(capsem_core::telemetry::CAPSEM_PROFILE_REVISION_ENV)
                .map(String::as_str),
            Some("2026.0522.1")
        );
        assert_eq!(
            env.get(capsem_core::telemetry::CAPSEM_USER_ID_ENV)
                .map(String::as_str),
            Some("sansa")
        );
    }
}
