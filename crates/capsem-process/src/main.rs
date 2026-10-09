mod aggregator_driver;
mod cables;
mod helpers;
mod ipc;
mod job_store;
mod mcp_runtime;
mod metric_export;
mod private_names;
mod private_seats;
mod runtime_config;
mod terminal;
mod upstream_grant;
mod vsock;

use anyhow::{Context, Result};
use capsem_core::fs_monitor::FsMonitor;
use capsem_core::net::dns::{DnsAnswerCache, DnsResolver};
use capsem_core::{boot_vm, BootOptions, VirtioFsShare, VsockConnection};
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use clap::Parser;
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use job_store::JobStore;
use mcp_runtime::{GuestExposureTools, McpRuntime};
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

#[cfg(target_os = "linux")]
fn confine_linux_owner(args: &Args, session_dir: &Path) -> Result<()> {
    use capsem_foundation::unix::worker_sandbox::{Access, Policy, Role};

    let grant = |policy: Policy, path: &Path, access| -> Result<Policy> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("resolve sandbox grant {}", path.display()))?;
        Ok(policy.allow(canonical, access))
    };
    let mut policy = Policy::new(Role::VmOwner);
    policy = grant(policy, session_dir, Access::ReadWrite)?;
    policy = grant(policy, &args.assets_dir, Access::ReadOnly)?;
    policy = grant(policy, &args.rootfs, Access::ReadOnly)?;
    policy = grant(policy, &args.active_policy, Access::ReadOnly)?;
    for path in args.kernel.iter().chain(args.initrd.iter()) {
        policy = grant(policy, path, Access::ReadOnly)?;
    }
    let executable = std::env::current_exe().context("locate VM-owner executable")?;
    let router = executable.with_file_name("capsem-router");
    if router.exists() {
        policy = grant(policy, &router, Access::Executable)?;
    }
    for path in ["/lib", "/lib64", "/usr/lib", "/usr/lib64"] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant(policy, path, Access::ReadOnly)?;
        }
    }
    let linker_cache = Path::new("/etc/ld.so.cache");
    if linker_cache.exists() {
        policy = grant(policy, linker_cache, Access::ReadOnly)?;
    }
    for path in ["/lib64/ld-linux-x86-64.so.2", "/lib/ld-linux-aarch64.so.1"] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant(policy, path, Access::Executable)?;
        }
    }
    capsem_foundation::unix::worker_sandbox::confine(&policy)?;
    Ok(())
}

#[cfg(target_os = "linux")]
struct OwnerSandboxAttestation {
    direct_path: PathBuf,
    direct_file: std::fs::File,
    guest_path: PathBuf,
    guest_relative: Vec<u8>,
}

#[cfg(target_os = "linux")]
fn prepare_owner_sandbox_attestation(session_dir: &Path) -> Result<OwnerSandboxAttestation> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let create = |path: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("prepare owner sandbox attestation {}", path.display()))
    };
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let direct_path = session_dir.join(format!(".owner-sandbox-attestation-{nonce}"));
    let guest_relative = format!("workspace/.owner-metadata-attestation-{nonce}").into_bytes();
    let guest_path = session_dir
        .join(capsem_core::GUEST_SHARE_DIR)
        .join(std::ffi::OsStr::from_bytes(&guest_relative));
    let direct_file = create(&direct_path)?;
    drop(create(&guest_path)?);
    Ok(OwnerSandboxAttestation {
        direct_path,
        direct_file,
        guest_path,
        guest_relative,
    })
}

#[cfg(target_os = "linux")]
async fn attest_linux_owner(
    attestation: OwnerSandboxAttestation,
    service_socket: PathBuf,
    upstream_grants: Arc<upstream_grant::UpstreamGrantClient>,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let require_denied = |result: std::io::Result<()>, authority: &str| -> Result<()> {
        match result {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
            Err(error) => anyhow::bail!("VM owner sandbox returned {error} while denying {authority}"),
            Ok(()) => anyhow::bail!("VM owner sandbox retained authority to {authority}"),
        }
    };
    require_denied(
        std::fs::File::open("/etc/passwd").map(|_| ()),
        "read unrelated host files",
    )?;
    require_denied(
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)).map(|_| ()),
        "dial TCP sockets directly",
    )?;
    require_denied(
        std::os::unix::net::UnixStream::connect(service_socket).map(|_| ()),
        "dial control sockets by path",
    )?;
    require_denied(
        attestation
            .direct_file
            .set_permissions(std::fs::Permissions::from_mode(0o777)),
        "change host modes directly",
    )?;

    tokio::task::spawn_blocking(move || {
        capsem_core::GuestMetadataAuthority::set_mode(upstream_grants.as_ref(), &attestation.guest_relative, 0o701)
    })
    .await
    .context("join brokered metadata attestation")??;
    let mode = std::fs::metadata(&attestation.guest_path)?.mode() & 0o7777;
    anyhow::ensure!(mode == 0o701, "brokered metadata attestation returned mode {mode:o}");
    std::fs::remove_file(&attestation.guest_path)?;
    std::fs::remove_file(&attestation.direct_path)?;
    Ok(())
}

fn main() -> Result<()> {
    // SAFETY: process entry precedes argument parsing, telemetry, descriptor
    // owners and runtime threads. Broker grants will be named here explicitly.
    unsafe { capsem_foundation::unix::fd::close_inherited_descriptors()? };
    let upstream_socket =
        upstream_grant::adopt_inherited(std::io::stdin().as_fd()).context("adopt inherited upstream grant channel")?;
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
    // The kernel parent is the coordinator that launched this owner. Keep
    // that identity for every IPC check and die if it changes, so an orphan
    // cannot accept a later process that reuses the old parent's PID.
    let _owner_guards = capsem_guard::install(Some(controller.pid.get()), &session_dir.join("process.lock"))?
        .context("this session already has a running VM owner")?;

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let runtime_source = runtime_config::RuntimePolicySource::new(args.active_policy.clone());
    let runtime_config = runtime_source.load()?;
    let upstream_grants = {
        let _runtime = rt.enter();
        Arc::new(upstream_grant::UpstreamGrantClient::start(
            upstream_socket,
            runtime_config.active_policy_digest.clone(),
        )?)
    };

    info!(id = %args.id, "capsem-sandbox-process starting");
    let guest_dir = prepare_session_layout(&session_dir, args.scratch_disk_size_gb)?;
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
    let machine_identifier_path = session_dir.join("machine_identifier");
    let serial_log_path = session_dir.join("serial.log");
    let (vm, vsock_rx, sm) = boot_vm(BootOptions {
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

    let trace_id_for_loop = trace_id;
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
            trace_id_for_loop,
            session_dir_for_loop,
            shutdown_for_loop,
            runtime_source,
            runtime_config,
            upstream_grants,
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
    unsafe {
        core_foundation_sys::runloop::CFRunLoopRun();
    }
    #[cfg(not(target_os = "macos"))]
    rt.block_on(tokio::signal::ctrl_c())?;

    // A VM the hypervisor stopped on its own is not a clean exit: the
    // service keeps the session directory and reports the VM as exited
    // unexpectedly, which is what happened.
    if let Some(reason) = rt.block_on(async { vm_for_exit.lock().await.stop_reason() }) {
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
    trace_id: String,
    session_dir: std::path::PathBuf,
    shutdown: Arc<Mutex<Shutdown>>,
    runtime_source: runtime_config::RuntimePolicySource,
    runtime_config: runtime_config::RuntimePolicyConfig,
    upstream_grants: Arc<upstream_grant::UpstreamGrantClient>,
) -> Result<()> {
    let terminal_output = Arc::new(capsem_core::TerminalOutputQueue::new());

    // 1024 queued events: a guest resolving and fetching in parallel enqueues
    // several rows per request, and a full queue makes every producer sleep
    // in 5 ms steps on its reply path (see `DbWriter::send_with_backpressure`).
    let db = Arc::new(capsem_logger::DbWriter::open(&session_dir.join("session.db"), 1024)?);
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
    let job_store = Arc::new(JobStore {
        publisher: Arc::new(
            capsem_core::container::publish::Publisher::for_session(
                &session_dir,
                runtime_config.network.router.clone(),
            )?
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
    let seats = private_seats::bind(
        private_seats::Seats {
            id: &args.id,
            service_socket: args.service_socket.as_deref(),
            uds_path: &args.uds_path,
            run_dir: args.run_dir.as_deref(),
        },
        &job_store,
        ctrl_tx.clone(),
    )?;
    let private_service = Arc::new(
        capsem_core::service_uds::Client::connect(&seats.service_socket)
            .await
            .context("prepare private-name service channel")?,
    );
    let metric_service = if args.metric_broker {
        Some(
            tokio::net::UnixStream::connect(&seats.service_socket)
                .await
                .context("prepare metric service channel")?
                .into_std()
                .context("adopt metric service channel")?,
        )
    } else {
        None
    };
    // Held until the process exits: dropping it flushes the last measurements.
    let _metric_export = metric_export::install(&args.id, metric_service, args.metric_broker);
    let restored = job_store
        .publisher
        .restore(ctrl_tx.clone())
        .await
        .context("restore published ports")?;
    info!(restored, "restored published ports");
    let model_trace_state = Arc::new(std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new()));

    // Start host file monitor to record fs_events.
    // Opened once, by descriptor: the guest can swap its workspace for a host link.
    match capsem_core::session::open_workspace(&session_dir)
        .map_err(anyhow::Error::from)
        .and_then(|workspace| {
            capsem_core::fs_monitor::FsMonitor::start(
                workspace,
                Arc::clone(&db),
                Arc::clone(&security_rules),
                Arc::clone(&model_trace_state),
            )
        }) {
        Ok(monitor) => {
            info!("host file monitor started");
            shutdown.lock().await.fs_monitor = Some(monitor);
        }
        Err(e) => {
            error!(error = %e, "failed to start host file monitor");
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
    builtin_env.insert("CAPSEM_SESSION_DIR".into(), session_dir.to_string_lossy().to_string());
    let db_path = session_dir.join("session.db");
    builtin_env.insert("CAPSEM_SESSION_DB".into(), db_path.to_string_lossy().to_string());
    builtin_env.insert(
        "CAPSEM_ACTIVE_POLICY".into(),
        runtime_config.active_policy_path.to_string_lossy().to_string(),
    );
    let mcp_servers = runtime_config.mcp_servers(builtin_bin.as_deref(), builtin_env.clone());
    // Spawn the isolated MCP aggregator subprocess.
    let aggregator_client = spawn_mcp_aggregator(&mcp_servers, &session_dir, &args.id, &trace_id).await?;

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
                let prev = existing.iter().find(|e| e.namespaced_name == t.namespaced_name);
                capsem_core::mcp::ToolCacheEntry {
                    namespaced_name: t.namespaced_name.clone(),
                    original_name: t.original_name.clone(),
                    description: t.description.clone(),
                    server_name: t.server_name.clone(),
                    annotations: t.annotations.clone(),
                    pin_hash: pin_hash.clone(),
                    first_seen: prev.map(|p| p.first_seen.clone()).unwrap_or_else(|| now.clone()),
                    last_seen: now.clone(),
                    approved: prev.map(|p| p.approved && p.pin_hash == pin_hash).unwrap_or(false),
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
    let model_endpoints = Arc::new(std::sync::RwLock::new(Arc::new(runtime_config.model_endpoints.clone())));
    let mcp_inflight = Arc::new(tokio::sync::Semaphore::new(inflight_cap));
    let mcp_endpoint = Arc::new(
        capsem_core::net::mitm_proxy::McpEndpointState::new(
            aggregator_client.clone(),
            Arc::clone(&security_rules),
            Arc::clone(&plugin_policy),
            Arc::clone(&mcp_inflight),
            capsem_core::net::mitm_proxy::McpTimeouts::from_env(),
        )
        .with_builtin_ledger(Arc::clone(&db), capsem_core::mcp::builtin_server_names(&mcp_servers))
        .with_scoped_tools(Arc::new(GuestExposureTools::new(
            Arc::clone(&job_store.publisher),
            ctrl_tx.clone(),
        ))),
    );
    let mcp_runtime = Arc::new(McpRuntime {
        aggregator: aggregator_client,
        endpoint: Arc::clone(&mcp_endpoint),
        db: Arc::clone(&db),
        security_rules: Arc::clone(&security_rules),
        plugin_policy: Arc::clone(&plugin_policy),
        model_endpoints: Arc::clone(&model_endpoints),
    });

    let telemetry_deps = Arc::new(capsem_core::net::mitm_proxy::telemetry_hook::TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::clone(&model_trace_state),
        security_rules: Arc::clone(&security_rules),
        plugin_policy: Arc::clone(&plugin_policy),
    });
    let mitm_pipeline = capsem_core::net::mitm_proxy::make_production_pipeline(
        Arc::clone(&net_state.policy),
        Arc::clone(&telemetry_deps),
    );
    let mitm_config = Arc::new(capsem_core::net::mitm_proxy::MitmProxyConfig {
        ca: Arc::clone(&net_state.ca),
        server_tls: capsem_core::net::mitm_proxy::make_server_tls_config(&net_state.ca),
        policy: Arc::clone(&net_state.policy),
        model_endpoints,
        db: Arc::clone(&db),
        upstream_tls: Arc::clone(&net_state.upstream_tls),
        telemetry: telemetry_deps,
        pipeline: mitm_pipeline,
        mcp_endpoint: Some(mcp_endpoint),
        upstream_resolver: capsem_core::net::upstream_address::UpstreamResolver::disabled(),
        upstream_grants: Some(Arc::clone(&upstream_grants) as Arc<dyn capsem_core::net::mitm_proxy::TcpUpstreamGrants>),
    });

    // DNS handler shares the same security rule/plugin handles as MITM
    // so admin enforcement edits take effect across protocols at once.
    let dns_resolver = Arc::new(DnsResolver::with_grants(
        runtime_config.dns_upstreams.clone(),
        Arc::clone(&upstream_grants) as Arc<dyn capsem_core::net::dns::DnsUpstreamGrants>,
    ));
    // The private zone is the service's to answer, for this VM's networks.
    let private_names = Arc::new(private_names::ServicePrivateNames::new(
        private_service,
        args.id.clone(),
    ));
    let dns_handler = Arc::new(
        capsem_core::net::dns::DnsHandler::with_cache(
            Arc::clone(&net_state.policy),
            Arc::clone(&security_rules),
            Arc::clone(&plugin_policy),
            Arc::clone(&dns_resolver),
            Arc::new(DnsAnswerCache::default()),
        )
        .with_private_names(private_names),
    );

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
    let pty_log = match capsem_core::pty_log::PtyLog::open(&session_dir.join("pty.log")) {
        Ok(pl) => Some(Arc::new(pl)),
        Err(e) => {
            warn!(error = %e, "failed to open pty.log");
            None
        }
    };

    if uds_path.exists() {
        std::fs::remove_file(&uds_path)?;
    }
    let listener = UnixListener::bind(&uds_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o600))?;
    }
    let launched_path = uds_path.with_extension("launched");
    let mut launched = prepare_sentinel(&launched_path)?;
    let ready = prepare_sentinel(&uds_path.with_extension("ready"))?;

    #[cfg(target_os = "linux")]
    {
        let attestation = prepare_owner_sandbox_attestation(&session_dir)?;
        confine_linux_owner(&args, &session_dir).context("install Linux VM-owner confinement")?;
        attest_linux_owner(attestation, seats.service_socket.clone(), Arc::clone(&upstream_grants))
            .await
            .context("attest Linux VM-owner confinement")?;
    }

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
            mitm_config: mitm_config_clone,
            dns_handler: dns_handler_clone,
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
        let dns_resolver_c = Arc::clone(&dns_resolver);
        let upstream_grants_c = Arc::clone(&upstream_grants);

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
                dns_resolver_c,
                upstream_grants_c,
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
) -> Result<capsem_proto::mcp_aggregator::AggregatorClient> {
    use capsem_proto::mcp_aggregator::*;

    let (client, rx) = AggregatorClient::channel(64);

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

    let inflight = aggregator_driver::spawn(rx, child_stdin, child_stdout);

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
