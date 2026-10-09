mod cables;
mod helpers;
mod ipc;
mod job_store;
mod mcp_runtime;
mod metric_export;
mod owner_sandbox;
mod private_seats;
mod proxy_mcp;
mod runtime_config;
mod terminal;
mod trace_hints;
mod vsock;

use anyhow::{Context, Result};
use capsem_core::fs_monitor::FsMonitor;
use capsem_core::net::upstream_grant::{adopt_inherited, UpstreamGrantClient};
use capsem_core::{prepare_vm, BootOptions, VirtioFsShare, VsockConnection};
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use clap::Parser;
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use job_store::JobStore;
use mcp_runtime::{GuestExposureTools, McpRuntime};
use owner_sandbox::{attest_owner, confine_owner, prepare_owner_sandbox_attestation};
use vsock::VsockOptions;

/// Owns the background-thread resources that MUST drain before the main
/// run loop stops. Populated by `run_async_main_loop` once DbWriter and
/// FsMonitor are constructed, drained by the SIGTERM handler before it
/// calls `CFRunLoopStop`. See /dev-rust-patterns, "Signal-driven explicit
/// cleanup".
#[derive(Default)]
pub(crate) struct Shutdown {
    publisher: Option<Arc<capsem_core::container::publish::Publisher>>,
    db: Option<Arc<DbWriter>>,
    fs_monitor: Option<FsMonitor>,
    proxy_mcp: Option<tokio::task::JoinHandle<()>>,
    proxy_trace_hints: Option<tokio::task::JoinHandle<()>>,
}

struct PreparedOwnerResources {
    seats: private_seats::Prepared,
    workspace: capsem_foundation::unix::contained::ContainedDir,
    metric_service: Option<std::os::unix::net::UnixStream>,
    ipc_listener: std::os::unix::net::UnixListener,
    launched: std::fs::File,
    ready: std::fs::File,
    pty_log: Option<Arc<capsem_core::pty_log::PtyLog>>,
    aggregator: capsem_proto::mcp_aggregator::AggregatorClient,
    mcp_servers: Vec<capsem_proto::mcp_contracts::McpServerDef>,
    builtin_bin: Option<PathBuf>,
    builtin_env: std::collections::HashMap<String, String>,
    ledger: PreparedLedger,
}

struct PreparedLedger {
    stream: std::os::unix::net::UnixStream,
    commitment: std::os::unix::net::UnixStream,
    grant: capsem_proto::ledger::LedgerChannelGrant,
}

impl PreparedLedger {
    fn from_grant(
        (stream, commitment, grant): (
            std::os::unix::net::UnixStream,
            std::os::unix::net::UnixStream,
            capsem_proto::ledger::LedgerChannelGrant,
        ),
    ) -> Self {
        Self {
            stream,
            commitment,
            grant,
        }
    }
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

pub(crate) async fn drain_background_owners(shutdown: &Arc<Mutex<Shutdown>>) {
    // Keep the lock through joining: a concurrent shutdown caller must not
    // stop the run loop while the first caller is still draining its owners.
    let mut guard = shutdown.lock().await;
    let mut owned = std::mem::take(&mut *guard);
    if let Some(proxy_mcp) = owned.proxy_mcp.take() {
        proxy_mcp.abort();
        let _ = proxy_mcp.await;
    }
    if let Some(proxy_trace_hints) = owned.proxy_trace_hints.take() {
        proxy_trace_hints.abort();
        let _ = proxy_trace_hints.await;
    }
    if let Some(publisher) = owned.publisher.take() {
        publisher.shutdown().await;
    }
    if let Err(error) = tokio::task::spawn_blocking(move || owned.drain_blocking()).await {
        error!(%error, "background owner drain failed");
    }
    drop(guard);
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    id: String,
    /// Trusted host-side identity; independent of guest environment overrides.
    #[arg(long)]
    vm_name: Option<String>,
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
    /// BLAKE3 hashes of the boot assets this VM must boot.
    ///
    /// Supplied by the service, which pins them: a new VM gets the installed
    /// runtime asset set, a persistent VM the set it was created with.
    #[arg(long)]
    expected_kernel_hash: String,
    #[arg(long)]
    expected_initrd_hash: String,
    #[arg(long)]
    expected_rootfs_hash: String,
    #[arg(long)]
    session_dir: PathBuf,
    #[arg(long)]
    active_policy: PathBuf,
    #[arg(long, default_value_t = 2)]
    cpus: u32,
    #[arg(long, default_value_t = 2048)]
    ram_mb: u64,
    #[arg(long, default_value_t = 16)]
    scratch_disk_size_gb: u32,
    #[arg(long)]
    uds_path: PathBuf,
    /// The service's run directory, so the terminal socket is derived from the
    /// same place the gateway derives it from.
    ///
    /// Passed rather than walked up from `uds_path`. That worked while the IPC
    /// socket was `{run_dir}/instances/{id}.sock` and broke the moment it was
    /// shortened to `/tmp/capsem-<uid>/<hash>.sock` -- which is precisely the long
    /// run directory the shortening exists for. Two levels up from the short
    /// form is `/tmp`, so the process bound `/tmp/instances/...` while the
    /// gateway dialled `{run_dir}/instances/...`.
    #[arg(long)]
    run_dir: Option<PathBuf>,
    /// The service's own socket, where this owner asks on a guest's behalf
    /// (private names and brokered metrics). Given by the service: it is not always
    /// `{run_dir}/service.sock`.
    #[arg(long)]
    service_socket: Option<PathBuf>,
    /// Export metrics through the service's generation-authenticated local
    /// broker. The service grants this without exposing a collector address.
    #[arg(long)]
    metric_broker: bool,
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
    let pid = u64::from(std::process::id());
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

const OWNER_CHECKPOINT_FILE: &str = "checkpoint.vzsave";

fn owner_checkpoint_path(session_dir: &Path) -> PathBuf {
    session_dir
        .join(capsem_core::session::OWNER_STATE_DIR)
        .join(OWNER_CHECKPOINT_FILE)
}

fn prepare_session_layout(session_dir: &Path, scratch_disk_size_gb: u32) -> Result<PathBuf> {
    capsem_core::create_virtiofs_session(session_dir, scratch_disk_size_gb)?;
    let guest_dir = capsem_core::guest_share_dir(session_dir);

    #[cfg(not(test))]
    {
        let rootfs_img = capsem_core::session::system_overlay_image_path(session_dir);
        let template_img = capsem_core::system_overlay_template_path_for_session(session_dir, scratch_disk_size_gb);
        match capsem_core::preformat_system_overlay_image_from_template_if_needed(
            &rootfs_img,
            &template_img,
            scratch_disk_size_gb,
        ) {
            Ok(true) => info!(
                path = %rootfs_img.display(),
                template = %template_img.display(),
                "cloned preformatted system overlay image"
            ),
            Ok(false) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => warn!(
                path = %rootfs_img.display(),
                error = %e,
                "mke2fs unavailable; guest will format system overlay at first boot"
            ),
            Err(e) => return Err(e.into()),
        }
    }

    Ok(guest_dir)
}

fn prepare_sentinel(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("prepare readiness sentinel {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn main() -> Result<()> {
    // SAFETY: process entry precedes argument parsing, telemetry, descriptor
    // owners and runtime threads. Broker grants will be named here explicitly.
    unsafe { capsem_foundation::unix::fd::close_inherited_descriptors()? };
    let upstream_socket =
        adopt_inherited(std::io::stdin().as_fd()).context("adopt inherited upstream grant channel")?;
    let _telemetry_guard = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-process",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "info",
    })?;
    let args = Args::parse();
    let controller = capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::parent_process_id().context("missing coordinator parent")?,
        uid: capsem_foundation::unix::process::current_uid(),
    };

    // Root span shared across the whole capsem-process run: every
    // subsequent log line inherits `vm_id` and `trace_id` as structured
    // fields in the JSON output. Guard is held until main returns.
    let trace_id = generate_trace_id();
    let root_span = tracing::info_span!("vm", vm_id = %args.id, trace_id = %trace_id);
    let _root_span_guard = root_span.enter();

    std::fs::create_dir_all(&args.session_dir)?;
    let mut session_dir = args.session_dir.clone();
    if let Ok(resolved) = session_dir.canonicalize() {
        session_dir = resolved;
    }
    let _owner_singleton = capsem_guard::Singleton::try_acquire(&session_dir.join("process.lock"))?
        .context("this session already has a running VM owner")?;

    // A current-thread runtime creates no worker before confinement. Tasks
    // queued during preparation start only after the sandbox is installed.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let runtime_source = runtime_config::RuntimePolicySource::new(args.active_policy.clone());
    let runtime_config = runtime_source.load()?;
    let upstream_grants = {
        let _runtime = rt.enter();
        Arc::new(UpstreamGrantClient::start(upstream_socket)?)
    };

    info!(id = %args.id, "capsem-sandbox-process starting");
    let guest_dir = prepare_session_layout(&session_dir, args.scratch_disk_size_gb)?;
    capsem_core::container::publish::Publisher::prepare_session(&session_dir)?;
    // The image share is attached to every session, read-only at the device:
    // a device cannot be added after boot, and an image is pulled only once
    // the VM runs (its owner admits the pull). It stays empty until the
    // service publishes an image's verified blobs into it, and a guest root
    // that remounts it read-write still cannot write it.
    let virtiofs_shares = vec![
        VirtioFsShare {
            tag: "capsem".into(),
            host_path: guest_dir,
            read_only: false,
            metadata_authority: Some(upstream_grants.clone()),
        },
        VirtioFsShare {
            tag: capsem_core::session::IMAGE_SHARE_TAG.into(),
            host_path: capsem_core::session::prepare_image_share(&session_dir)?,
            read_only: true,
            metadata_authority: None,
        },
    ];

    // Attach the system-overlay rootfs.img as a virtio-blk device (/dev/vdb in
    // the guest). capsem-init mounts it as the overlayfs upper directly --
    // native virtio-blk speaks block-device semantics and doesn't EIO under
    // writeback pressure across save_state/restore_state, unlike the prior
    // loop-on-VirtioFS path. The file lives in the host-only session
    // `system/` directory, outside the share, so the guest cannot swap it
    // for a link to a host file (`capsem_core::session::adopt_system_overlay`).
    let system_img = capsem_core::session::system_overlay_image_path(&session_dir);
    let owner_state = session_dir.join(capsem_core::session::OWNER_STATE_DIR);
    let machine_identifier_path = owner_state.join("machine_identifier");
    let legacy_machine_identifier = session_dir.join("machine_identifier");
    if legacy_machine_identifier.exists() && !machine_identifier_path.exists() {
        std::fs::rename(&legacy_machine_identifier, &machine_identifier_path)
            .context("move machine identifier into owner state")?;
    }
    let serial_log_path = session_dir.join("serial.log");
    drop(capsem_foundation::unix::fs::open_private_append_no_follow(
        &serial_log_path,
    )?);
    let pty_log = match capsem_core::pty_log::PtyLog::open(&session_dir.join("pty.log")) {
        Ok(log) => Some(Arc::new(log)),
        Err(error) => {
            warn!(%error, "failed to prepare pty.log");
            None
        }
    };
    let executable = std::env::current_exe().context("locate VM-owner executable")?;
    let aggregator_bin = resolve_mcp_aggregator_binary(&executable)?;
    let aggregator_stderr =
        capsem_foundation::unix::fs::open_private_append_no_follow(&aggregator_log_path(&session_dir))?;
    let builtin_bin = executable
        .parent()
        .map(|directory| directory.join("capsem-mcp-builtin"));
    let mut builtin_env = std::collections::HashMap::new();
    builtin_env.insert("CAPSEM_SESSION_DIR".into(), session_dir.to_string_lossy().to_string());
    builtin_env.insert(
        "CAPSEM_ACTIVE_POLICY".into(),
        runtime_config.active_policy_path.to_string_lossy().to_string(),
    );
    let mcp_servers = runtime_config.mcp_servers(builtin_bin.as_deref(), builtin_env.clone());
    // The aggregator is a separately supervised process. Spawn it while the
    // VM owner still has the one executable capability needed to create it;
    // parent-side driver tasks remain queued on the current-thread runtime.
    let aggregator = rt.block_on(spawn_mcp_aggregator(
        &mcp_servers,
        &session_dir,
        &args.id,
        &trace_id,
        aggregator_bin,
        aggregator_stderr,
    ))?;
    let prepared_vm = prepare_vm(BootOptions {
        assets: &args.assets_dir,
        kernel_override: args.kernel.as_deref(),
        initrd_override: args.initrd.as_deref(),
        rootfs_override: Some(&args.rootfs),
        cmdline: capsem_core::vm::config::KERNEL_CMDLINE,
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
        expected_asset_hashes: Some(capsem_assets::asset_manager::ExpectedAssetHashes {
            kernel: args.expected_kernel_hash.clone(),
            initrd: args.expected_initrd_hash.clone(),
            rootfs: args.expected_rootfs_hash.clone(),
        }),
    })?;

    let prepared_seats = private_seats::prepare(private_seats::Seats {
        id: &args.id,
        service_socket: args.service_socket.as_deref(),
        uds_path: &args.uds_path,
        run_dir: args.run_dir.as_deref(),
    })?;
    let metric_service = if args.metric_broker {
        Some(
            std::os::unix::net::UnixStream::connect(&prepared_seats.service_socket)
                .context("prepare metric service channel")?,
        )
    } else {
        None
    };
    if args.uds_path.exists() {
        std::fs::remove_file(&args.uds_path)?;
    }
    let ipc_listener = std::os::unix::net::UnixListener::bind(&args.uds_path)?;
    ipc_listener.set_nonblocking(true)?;
    std::fs::set_permissions(&args.uds_path, std::fs::Permissions::from_mode(0o600))?;
    let launched_path = args.uds_path.with_extension("launched");
    let launched = prepare_sentinel(&launched_path)?;
    let ready = prepare_sentinel(&args.uds_path.with_extension("ready"))?;
    // The grant response proves the supervised ledger worker has opened and
    // initialized session.db. Acquire it before confinement so the denial
    // attestation below always tests an existing ledger, then carry only the
    // connected descriptors across the boundary.
    let ledger = PreparedLedger::from_grant(
        rt.block_on(upstream_grants.open_ledger())
            .context("acquire supervised session ledger")?,
    );
    // Opening through the session root after confinement would require read
    // authority over session.db and every other sibling. Resolve the
    // guest-writable workspace before confinement, then carry only its
    // no-follow directory descriptor into the VM owner.
    let workspace = capsem_core::session::open_workspace(&session_dir).context("open workspace before confinement")?;
    let attestation = prepare_owner_sandbox_attestation(&session_dir)?;

    confine_owner(&args, &session_dir).context("install VM-owner confinement")?;
    rt.block_on(attest_owner(
        attestation,
        prepared_seats.service_socket.clone(),
        Arc::clone(&upstream_grants),
    ))
    .context("attest VM-owner confinement")?;
    // The parent watcher is the first worker thread and therefore inherits
    // the installed Landlock/Seatbelt and seccomp policy.
    capsem_guard::watch_parent_or_exit(Some(controller.pid.get()))?;

    let (vm, vsock_rx, sm) = prepared_vm.boot()?;

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

    let prepared_resources = PreparedOwnerResources {
        seats: prepared_seats,
        workspace,
        metric_service,
        ipc_listener,
        launched,
        ready,
        pty_log,
        aggregator,
        mcp_servers,
        builtin_bin,
        builtin_env,
        ledger,
    };

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

    let session_dir_for_loop = session_dir;
    let shutdown_for_loop = Arc::clone(&shutdown);
    let shutdown_for_loop_error = Arc::clone(&shutdown);
    let vm_for_signal = Arc::clone(&vm_arc);
    let vm_for_exit = Arc::clone(&vm_arc);
    rt.spawn(async move {
        if let Err(e) = Box::pin(run_async_main_loop(
            args,
            controller,
            vm_arc,
            vsock_rx,
            session_dir_for_loop,
            shutdown_for_loop,
            runtime_source,
            runtime_config,
            upstream_grants,
            prepared_resources,
        ))
        .await
        {
            error!(error = format!("{e:#}"), "async loop failed");
            drain_background_owners(&shutdown_for_loop_error).await;
            std::process::exit(1);
        }
    });

    // Signal-driven explicit cleanup. On SIGTERM/SIGINT, synchronously
    // stop the VM and drain the background-thread owners in the `Shutdown`
    // struct (FsMonitor -> DbWriter) BEFORE stopping the main run loop.
    // Without this, teardown relies on tokio-runtime-drop ordering and can
    // miss the service's 1s SIGKILL budget mid-checkpoint, leaving a dirty
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

        match vm_for_signal.lock().await.stop() {
            Ok(()) => tracing::info!(signal = signal_name, "VM stop requested for signal teardown"),
            Err(e) => tracing::warn!(signal = signal_name, error = %e, "VM stop failed during signal teardown"),
        }

        drain_background_owners(&shutdown_for_sig).await;
        tracing::warn!(signal = signal_name, "background owners drained, stopping run loop");

        #[cfg(target_os = "macos")]
        unsafe {
            core_foundation_sys::runloop::CFRunLoopStop(core_foundation_sys::runloop::CFRunLoopGetMain());
        }
        #[cfg(not(target_os = "macos"))]
        std::process::exit(0);
    });

    #[cfg(target_os = "macos")]
    let _runtime_thread = std::thread::Builder::new()
        .name("capsem-process-runtime".into())
        .spawn(move || rt.block_on(std::future::pending::<()>()))?;
    #[cfg(target_os = "macos")]
    unsafe {
        core_foundation_sys::runloop::CFRunLoopRun();
    }
    #[cfg(not(target_os = "macos"))]
    rt.block_on(tokio::signal::ctrl_c())?;

    // A VM the hypervisor stopped on its own is not a clean exit: the
    // service keeps the session directory and reports the VM as exited
    // unexpectedly, which is what happened.
    #[cfg(target_os = "macos")]
    let stop_reason = vm_for_exit.blocking_lock().stop_reason();
    #[cfg(not(target_os = "macos"))]
    let stop_reason = rt.block_on(async { vm_for_exit.lock().await.stop_reason() });
    if let Some(reason) = stop_reason {
        anyhow::bail!("the hypervisor stopped the VM: {reason}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_async_main_loop(
    args: Args,
    controller: capsem_foundation::unix::peer::PeerIdentity,
    vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    session_dir: std::path::PathBuf,
    shutdown: Arc<Mutex<Shutdown>>,
    runtime_source: runtime_config::RuntimePolicySource,
    runtime_config: runtime_config::RuntimePolicyConfig,
    upstream_grants: Arc<UpstreamGrantClient>,
    prepared: PreparedOwnerResources,
) -> Result<()> {
    let PreparedOwnerResources {
        seats: prepared_seats,
        workspace,
        metric_service,
        ipc_listener,
        mut launched,
        ready,
        pty_log,
        aggregator: aggregator_client,
        mcp_servers,
        builtin_bin,
        builtin_env,
        ledger,
    } = prepared;
    let terminal_output = Arc::new(capsem_core::TerminalOutputQueue::new());

    let ledger_path = session_dir.join("session.db");
    // 1024 queued events: a guest resolving and fetching in parallel enqueues
    // several rows per request, and a full queue makes every producer sleep
    // in 5 ms steps on its reply path (see `DbWriter::send_with_backpressure`).
    // The logical path is diagnostic identity only; the descriptor selects
    // the worker that exclusively owns and opens the database.
    let db = Arc::new(
        tokio::task::spawn_blocking(move || {
            capsem_logger::DbWriter::from_ledger_channel(
                ledger.stream,
                ledger.commitment,
                ledger.grant,
                &ledger_path,
                1024,
            )
        })
        .await
        .context("join supervised session ledger handshake")??,
    );
    // Register the DbWriter with the SIGTERM handler BEFORE any work that
    // produces writes. If the signal fires before the workspace monitor
    // starts, we still want a clean checkpoint.
    shutdown.lock().await.db = Some(Arc::clone(&db));

    let security_rule_ids = runtime_config
        .security_rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<Vec<_>>();
    info!(
        active_policy = %runtime_config.active_policy_path.display(),
        security_rule_count = security_rule_ids.len(),
        security_rule_ids = ?security_rule_ids,
        plugin_count = runtime_config.plugins.len(),
        dns_upstreams = ?runtime_config.dns_upstreams,
        "capsem-process loaded runtime policy"
    );
    let guest_config = capsem_core::net::policy_config::GuestConfig::default();
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(runtime_config.security_rules.clone())));
    let plugin_policy = Arc::new(std::sync::RwLock::new(Arc::new(runtime_config.plugins.clone())));
    let proxy_policy = capsem_core::net::proxy_engine::ProxyPolicyHandle::new(runtime_config.proxy_policy_snapshot());
    let job_store = Arc::new(JobStore {
        publisher: Arc::new(
            capsem_core::container::publish::Publisher::for_prepared_session(
                &session_dir,
                runtime_config.network.router.clone(),
            )?
            .with_listener_authority(upstream_grants.clone())
            .with_security(
                args.id.clone(),
                args.vm_name.clone().unwrap_or_else(|| args.id.clone()),
                Arc::new(capsem_core::security_engine::network::ledger::NetworkSecurity {
                    db: db.clone(),
                    rules: security_rules.clone(),
                    plugins: plugin_policy.clone(),
                }),
            ),
        ),
        ..JobStore::new()
    });
    shutdown.lock().await.publisher = Some(job_store.publisher.clone());
    let (ipc_tx, _) = broadcast::channel::<ProcessToService>(128);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<ServiceToProcess>(32);
    let seats = prepared_seats.activate(&job_store, ctrl_tx.clone())?;
    // Held until the process exits: dropping it flushes the last measurements.
    let _metric_export = metric_export::install(&args.id, metric_service, args.metric_broker);
    let restored = job_store
        .publisher
        .restore(ctrl_tx.clone())
        .await
        .context("restore published ports")?;
    info!(restored, "restored published ports");
    let model_trace_state = Arc::new(std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new()));

    let monitor = capsem_core::fs_monitor::FsMonitor::start(
        workspace,
        Arc::clone(&db),
        Arc::clone(&security_rules),
        Arc::clone(&model_trace_state),
    )
    .context("start host file monitor")?;
    info!("host file monitor started");
    shutdown.lock().await.fs_monitor = Some(monitor);

    let net_state = Arc::new(capsem_core::create_net_state_with_policy(
        &args.id,
        Arc::clone(&db),
        runtime_config.network.clone(),
    )?);
    let inflight_cap = capsem_core::mcp::resolve_inflight_cap();
    info!(inflight_cap, "MITM MCP endpoint in-flight handler cap");
    let mcp_inflight = Arc::new(tokio::sync::Semaphore::new(inflight_cap));
    let mcp_timeouts = capsem_core::net::mitm_proxy::McpTimeouts::from_env();
    let builtin_servers = capsem_core::mcp::builtin_server_names(&mcp_servers);
    let scoped_tools: Arc<dyn capsem_core::net::mitm_proxy::ScopedMcpTools> = Arc::new(GuestExposureTools::new(
        Arc::clone(&job_store.publisher),
        ctrl_tx.clone(),
    ));
    let mcp_endpoint = Arc::new(
        capsem_core::net::mitm_proxy::McpEndpointState::new(
            aggregator_client.clone(),
            Arc::clone(&security_rules),
            Arc::clone(&plugin_policy),
            Arc::clone(&mcp_inflight),
            mcp_timeouts.clone(),
        )
        .with_builtin_ledger(Arc::clone(&db), builtin_servers.clone())
        .with_scoped_tools(Arc::clone(&scoped_tools)),
    );
    let mcp_runtime = Arc::new(McpRuntime {
        aggregator: aggregator_client.clone(),
        endpoint: Arc::clone(&mcp_endpoint),
        db: Arc::clone(&db),
        security_rules: Arc::clone(&security_rules),
        plugin_policy: Arc::clone(&plugin_policy),
        proxy_policy: proxy_policy.clone(),
    });
    let (proxy_trace_hints, owner_trace_hints) = std::os::unix::net::UnixStream::pair()?;
    let owner_trace_state = Arc::clone(&model_trace_state);
    let proxy_trace_hints_task = tokio::spawn(async move {
        if let Err(error) = trace_hints::serve(owner_trace_hints, owner_trace_state).await {
            tracing::warn!(%error, "proxy trace-hint capability stopped");
        }
    });
    if let Err(error) = upstream_grants.attach_proxy_trace_hints(proxy_trace_hints.into()).await {
        proxy_trace_hints_task.abort();
        let _ = proxy_trace_hints_task.await;
        return Err(error.context("grant proxy trace-hint capability"));
    }
    shutdown.lock().await.proxy_trace_hints = Some(proxy_trace_hints_task);
    let (proxy_mcp, owner_mcp) = std::os::unix::net::UnixStream::pair()?;
    let proxy_mcp_task = tokio::spawn(async move {
        if let Err(error) = proxy_mcp::serve(
            owner_mcp,
            aggregator_client,
            scoped_tools,
            builtin_servers,
            inflight_cap,
            mcp_timeouts,
        )
        .await
        {
            tracing::warn!(%error, "proxy MCP capability stopped");
        }
    });
    if let Err(error) = upstream_grants.attach_proxy_mcp(proxy_mcp.into()).await {
        proxy_mcp_task.abort();
        let _ = proxy_mcp_task.await;
        return Err(error.context("grant proxy MCP capability"));
    }
    shutdown.lock().await.proxy_mcp = Some(proxy_mcp_task);

    let ipc_tx_clone = ipc_tx.clone();
    let job_store_clone = Arc::clone(&job_store);
    let terminal_output_clone = Arc::clone(&terminal_output);

    // Serial log is written by a thread attached inside the hypervisor's
    // boot() (before machine.start() spawns the reader), so no subscription
    // is needed here -- tokio::broadcast would race with VM resume and drop
    // the first ~100ms of post-resume output.

    let net_state_clone = Arc::clone(&net_state);
    let upstream_grants_for_vsock = Arc::clone(&upstream_grants);

    // Parse --env KEY=VALUE pairs for guest injection
    let cli_env: Vec<(String, String)> = args
        .env
        .iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();

    let vm_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let ctrl_tx_ipc = ctrl_tx.clone();
    let uds_path = args.uds_path.clone();
    let is_restore = args.checkpoint_path.is_some();
    let vm_for_vsock = Arc::clone(&vm);
    let vm_ready_vsock = Arc::clone(&vm_ready);
    let db_for_vsock = Arc::clone(&db);
    let shutdown_for_vsock = Arc::clone(&shutdown);
    let shutdown_for_vsock_error = Arc::clone(&shutdown);
    let listener = tokio::net::UnixListener::from_std(ipc_listener)?;
    seats.start();
    use std::io::Write as _;
    launched.write_all(b"launched\n")?;
    launched.sync_data()?;
    info!(socket = %uds_path.display(), "listening for IPC (mode 0600)");

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
            upstream_grants: upstream_grants_for_vsock,
            security_rules: Arc::clone(&security_rules),
            plugin_policy: Arc::clone(&plugin_policy),
            _net_state: net_state_clone,
            is_restore,
            vm_ready: vm_ready_vsock,
            ready,
            db: db_for_vsock,
            pty_log,
            shutdown: shutdown_for_vsock,
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
            error!(error = format!("{e:#}"), "vsock failed");
            drain_background_owners(&shutdown_for_vsock_error).await;
            std::process::exit(1);
        }
    });

    // Terminal relay: fan-out broadcast + ring buffer so a newly-attached
    // terminal stream sees the shell's startup banner (printed before it joined).
    let term_relay = terminal::TerminalRelay::new(1024);
    let term_c_bcast = Arc::clone(&terminal_output);
    let term_relay_pump = Arc::clone(&term_relay);
    tokio::spawn(async move {
        while let Some(data) = term_c_bcast.poll().await {
            term_relay_pump.publish(data);
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
        let ready_c = Arc::clone(&vm_ready);

        tokio::spawn(async move {
            if let Err(e) = ipc::handle_ipc_connection(
                stream,
                controller,
                tx_c,
                ipc_tx_pass,
                term_c,
                job_c,
                net_c,
                mcp_c,
                runtime_source_c,
                builtin_bin_c,
                builtin_env_c,
                ready_c,
            )
            .await
            {
                error!(error = format!("{e:#}"), "IPC error");
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
    servers: &[capsem_proto::mcp_contracts::McpServerDef],
    session_dir: &Path,
    vm_id: &str,
    trace_id: &str,
    aggregator_bin: PathBuf,
    stderr_file: std::fs::File,
) -> Result<capsem_proto::mcp_aggregator::AggregatorClient> {
    use capsem_proto::mcp_aggregator::*;

    let (client, rx) = AggregatorClient::channel(64);

    let log_path = aggregator_log_path(session_dir);

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
    for (k, v) in capsem_foundation::telemetry::child_trace_env(vm_id) {
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
    let child_stdout = child.stdout.take().unwrap();

    // Send server definitions as the first frame.
    let defs_vec = servers.to_vec();
    write_frame(&mut child_stdin, &defs_vec).await?;

    let inflight = capsem_core::mcp::aggregator_driver::spawn(rx, child_stdin, child_stdout);

    // Monitor child process.
    tokio::spawn(async move {
        info!("aggregator monitor task started");
        match child.wait().await {
            Ok(status) => info!(status = %status, "aggregator subprocess exited"),
            Err(e) => error!(error = %e, "failed to wait on aggregator"),
        }
        inflight.close();
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
    anyhow::bail!("required MCP aggregator binary capsem-mcp-aggregator is missing; searched: {searched}")
}

#[cfg(test)]
mod tests;
