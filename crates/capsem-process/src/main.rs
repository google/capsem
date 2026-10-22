mod aggregator;
mod cables;
mod helpers;
mod ipc;
mod job_store;
mod mcp_runtime;
mod metric_export;
mod owner;
mod owner_sandbox;
mod private_seats;
mod proxy_mcp;
mod runtime_config;
mod trace_hints;
mod vsock;

use anyhow::{Context, Result};
use capsem_core::fs_monitor::FsMonitor;
use capsem_core::net::upstream_grant::{adopt_inherited, UpstreamGrantClient};
use capsem_core::{prepare_vm, BootOptions, VirtioFsShare, VsockConnection};
use capsem_foundation::unix::process::ensure_open_file_limit;
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use capsem_proto::NETWORK_PROCESS_OPEN_FILE_LIMIT;
use clap::Parser;
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use aggregator::connect_mcp_aggregator;
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
    terminal_bridge: Option<vsock::TerminalBridge>,
    pty_log: Option<capsem_core::pty_log::PtyRecorder>,
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
    pty_log: Option<capsem_core::pty_log::PtyLog>,
    serial_log: capsem_foundation::telemetry::SharedLogWriter,
    checkpoint: Arc<vsock::SuspendFiles>,
    terminal_output: Arc<capsem_core::terminal::SessionTerminal>,
    aggregator: capsem_proto::mcp_aggregator::AggregatorClient,
    mcp_servers: Vec<capsem_proto::mcp_contracts::McpServerDef>,
    builtin_bin: Option<PathBuf>,
    builtin_env: std::collections::HashMap<String, String>,
    ledger: PreparedLedger,
}

struct PreparedLedger {
    stream: std::os::unix::net::UnixStream,
    grant: capsem_proto::ledger::LedgerChannelGrant,
}

impl PreparedLedger {
    fn from_grant((stream, grant): (std::os::unix::net::UnixStream, capsem_proto::ledger::LedgerChannelGrant)) -> Self {
        Self { stream, grant }
    }
}

pub(crate) async fn drain_background_owners(shutdown: &Arc<Mutex<Shutdown>>) {
    // Keep the lock through joining: a concurrent shutdown caller must not
    // stop the run loop while the first caller is still draining its owners.
    let mut guard = shutdown.lock().await;
    let Shutdown {
        terminal_bridge,
        pty_log,
        publisher,
        db,
        fs_monitor,
        proxy_mcp,
        proxy_trace_hints,
    } = std::mem::take(&mut *guard);
    // Stop accepting terminal bytes and flush the final coalesced batch
    // before closing the transcript producer shared by input and output.
    if let Some(terminal) = terminal_bridge {
        if let Err(error) = terminal.shutdown().await {
            error!(%error, "terminal bridge drain failed");
        }
    }
    if let Some(recorder) = pty_log {
        if let Err(error) = recorder.shutdown().await {
            warn!(%error, "terminal transcript drain incomplete");
        }
    }
    if let Some(proxy_mcp) = proxy_mcp {
        proxy_mcp.abort();
        let _ = proxy_mcp.await;
    }
    if let Some(proxy_trace_hints) = proxy_trace_hints {
        proxy_trace_hints.abort();
        let _ = proxy_trace_hints.await;
    }
    if let Some(publisher) = publisher {
        publisher.shutdown().await;
    }
    if let Err(error) = tokio::task::spawn_blocking(move || {
        // FsMonitor's final events must reach DbWriter before checkpointing.
        if let Some(fs_monitor) = fs_monitor {
            fs_monitor.shutdown_and_join();
        }
        if let Some(db) = db {
            db.shutdown_blocking();
        }
    })
    .await
    {
        error!(%error, "background owner drain failed");
    }
    drop(guard);
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    id: String,
    /// Original service parent; required by owner admission before VM boot.
    #[arg(long)]
    parent_pid: Option<u32>,
    /// Original service-issued spawn UUID, checked against host-only metadata.
    #[arg(long)]
    spawn_generation: Option<String>,
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
    // Clap handles help/version without service-issued runtime capabilities.
    // Normal startup still requires its upstream grant before creating owners.
    let args = Args::parse();
    ensure_open_file_limit(NETWORK_PROCESS_OPEN_FILE_LIMIT)?;
    let upstream_socket =
        adopt_inherited(std::io::stdin().as_fd()).context("adopt inherited upstream grant channel")?;
    let _telemetry_guard = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-process",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "info",
    })?;
    let controller = capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::parent_process_id().context("missing coordinator parent")?,
        uid: capsem_foundation::unix::process::current_uid(),
    };
    // Declared before VM/runtime resources, so normal teardown releases the
    // generation lifetime fence last. SIGKILL still needs native exit proof.
    let _spawn_owner = owner::enroll(&args)?;

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
    if let Some(path) = &args.checkpoint_path {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            session_dir.join(path)
        };
        anyhow::ensure!(
            path == owner_checkpoint_path(&session_dir),
            "checkpoint must belong to this session owner"
        );
    }
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
    let logs = capsem_core::session::prepare_session_logs(&session_dir).context("prepare private session logs")?;
    let serial_log = logs.open_serial().context("prepare shared console log")?;
    let pty_log = match logs.open_pty() {
        Ok(log) => Some(log),
        Err(error) => {
            warn!(%error, "failed to prepare pty.log");
            None
        }
    };
    let executable = std::env::current_exe().context("locate VM-owner executable")?;
    let builtin_bin = executable
        .parent()
        .map(|directory| directory.join("capsem-mcp-builtin"))
        .filter(|binary| binary.is_file());
    let builtin_env = std::collections::HashMap::new();
    let mcp_servers = runtime_config.mcp_servers(builtin_bin.as_deref(), builtin_env.clone());
    // Native MCP children and their privileged capabilities stay service-owned.
    let aggregator = rt.block_on(connect_mcp_aggregator(&mcp_servers, &upstream_grants))?;
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
        serial_log_path: None,
        expected_asset_hashes: Some(capsem_assets::asset_manager::ExpectedAssetHashes {
            kernel: args.expected_kernel_hash.clone(),
            initrd: args.expected_initrd_hash.clone(),
            rootfs: args.expected_rootfs_hash.clone(),
        }),
    })?
    .with_serial_log(serial_log.clone());

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
    let checkpoint = Arc::new(vsock::SuspendFiles::prepare(&session_dir).context("prepare checkpoint handles")?);
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

    let terminal_output = if args.checkpoint_path.is_some() {
        checkpoint.load_terminal().context("restore terminal before VM boot")?
    } else {
        capsem_core::terminal::SessionTerminal::new(capsem_proto::TERMINAL_SUBSCRIBER_UPDATES)
    };
    let (vm, vsock_rx, sm) = prepared_vm.boot()?;

    // Delete checkpoint file if we just restored from it, so we don't accidentally suspend on normal shutdown
    if args.checkpoint_path.is_some() {
        checkpoint
            .consume_restored()
            .context("consume restored checkpoint pair")?;
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
        serial_log,
        checkpoint,
        terminal_output,
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
        serial_log,
        checkpoint,
        terminal_output,
        aggregator: aggregator_client,
        mcp_servers,
        builtin_bin,
        builtin_env,
        ledger,
    } = prepared;
    // This runs after owner confinement. The storage worker must inherit
    // that policy rather than being started while preparing file handles.
    let pty_log = pty_log.and_then(|log| match capsem_core::pty_log::PtyRecorder::new(log) {
        Ok(recorder) => Some(recorder),
        Err(error) => {
            warn!(%error, "terminal transcript worker could not start");
            None
        }
    });
    shutdown.lock().await.pty_log = pty_log.clone();

    let ledger_path = session_dir.join("session.db");
    // The config-owned queue covers the supported concurrent network burst.
    // A full queue makes every producer sleep in 5 ms steps on its reply path
    // (see `DbWriter::send_with_backpressure`).
    // The logical path is diagnostic identity only; the descriptor selects
    // the worker that exclusively owns and opens the database.
    let db = Arc::new(
        tokio::task::spawn_blocking(move || {
            capsem_logger::DbWriter::from_ledger_channel(
                ledger.stream,
                ledger.grant,
                &ledger_path,
                capsem_proto::LEDGER_PRODUCER_QUEUE_CAPACITY,
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
            serial_log,
            checkpoint,
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

    let session_terminal = Arc::clone(&terminal_output);

    loop {
        let (stream, _) = listener.accept().await?;
        let tx_c = ctrl_tx_ipc.clone();
        let ipc_tx_pass = ipc_tx.clone();
        let term_c = Arc::clone(&session_terminal);
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

#[cfg(test)]
mod tests;
