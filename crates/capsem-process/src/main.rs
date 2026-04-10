use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use anyhow::{Result, Context};
use clap::Parser;
use capsem_core::{
    boot_vm, BootOptions, VirtioFsShare,
    VsockConnection,
};
use capsem_core::{read_control_msg, send_boot_config, write_control_msg};
use capsem_proto::{GuestToHost, HostToGuest};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
use tokio::sync::{broadcast, oneshot, mpsc};
use std::os::unix::io::RawFd;
use tracing::{info, error, warn, debug};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};
use std::io::{Read, Write};
use futures::{sink::SinkExt, stream::StreamExt};

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

struct JobStore {
    jobs: Mutex<HashMap<u64, oneshot::Sender<JobResult>>>,
    /// Currently active exec job ID and its captured output.
    active_exec: Mutex<Option<(u64, Vec<u8>)>>,
    /// Channel for snapshot ready signal.
    snapshot_ready: Mutex<Option<oneshot::Sender<()>>>,
}

#[cfg_attr(test, derive(Debug))]
enum JobResult {
    Exec { stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32 },
    WriteFile { success: bool, error: Option<String> },
    ReadFile { data: Option<Vec<u8>>, error: Option<String> },
    Error { message: String },
}

/// Orchestrates a guest quiescence sequence around a provided async operation.
/// 
/// 1. Sends `PrepareSnapshot` to the guest (sync + fsfreeze).
/// 2. Waits up to `timeout` for `SnapshotReady`.
/// 3. If successful, executes the provided operation.
/// 4. Sends `Unfreeze` to the guest regardless of operation success or timeout.
#[allow(dead_code)]
async fn with_quiescence<F, Fut>(
    ctrl_cmd_tx: &std::sync::mpsc::Sender<HostToGuest>,
    job_store: &Arc<JobStore>,
    timeout: std::time::Duration,
    op: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    // Prepare oneshot channel
    let (tx, rx) = tokio::sync::oneshot::channel();
    *job_store.snapshot_ready.lock().unwrap() = Some(tx);

    info!("Sending PrepareSnapshot to guest");
    ctrl_cmd_tx
        .send(HostToGuest::PrepareSnapshot)
        .map_err(|e| anyhow::anyhow!("failed to send PrepareSnapshot: {}", e))?;

    // Wait for SnapshotReady with timeout
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(_)) => {
            info!("Guest filesystem frozen successfully, running operation");
            let op_result = op().await;

            info!("Operation complete, sending Unfreeze to guest");
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze);
            op_result
        }
        Ok(Err(_)) => {
            // Channel closed without receiving
            warn!("SnapshotReady channel closed prematurely, aborting operation and unfreezing");
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze);
            anyhow::bail!("SnapshotReady channel closed prematurely");
        }
        Err(_) => {
            warn!("Timeout waiting for SnapshotReady, aborting operation and unfreezing");
            let _ = ctrl_cmd_tx.send(HostToGuest::Unfreeze);
            anyhow::bail!("timed out waiting for SnapshotReady");
        }
    }
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

    // -----------------------------------------------------------------------
    // JobStore
    // -----------------------------------------------------------------------

    #[test]
    fn job_store_insert_and_remove() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        };
        let (tx, _rx) = oneshot::channel::<JobResult>();
        store.jobs.lock().unwrap().insert(1, tx);
        assert!(store.jobs.lock().unwrap().contains_key(&1));
        let removed = store.jobs.lock().unwrap().remove(&1);
        assert!(removed.is_some());
        assert!(!store.jobs.lock().unwrap().contains_key(&1));
    }

    #[test]
    fn job_store_missing_id_returns_none() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        };
        let removed = store.jobs.lock().unwrap().remove(&999);
        assert!(removed.is_none());
    }

    #[test]
    fn job_store_concurrent_ids_unique() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        };
        for i in 0..100 {
            let (tx, _rx) = oneshot::channel::<JobResult>();
            store.jobs.lock().unwrap().insert(i, tx);
        }
        assert_eq!(store.jobs.lock().unwrap().len(), 100);
    }

    #[test]
    fn job_store_active_exec_set_and_clear() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        };
        assert!(store.active_exec.lock().unwrap().is_none());

        *store.active_exec.lock().unwrap() = Some((42, Vec::new()));
        assert!(store.active_exec.lock().unwrap().is_some());

        let (id, buf) = store.active_exec.lock().unwrap().as_ref().unwrap().clone();
        assert_eq!(id, 42);
        assert!(buf.is_empty());

        *store.active_exec.lock().unwrap() = None;
        assert!(store.active_exec.lock().unwrap().is_none());
    }

    #[test]
    fn job_store_active_exec_captures_data() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(Some((1, Vec::new()))),
            snapshot_ready: Mutex::new(None),
        };
        // Simulate output capture
        if let Some((_, ref mut captured)) = *store.active_exec.lock().unwrap() {
            captured.extend_from_slice(b"hello ");
            captured.extend_from_slice(b"world");
        }
        let captured = store.active_exec.lock().unwrap().as_ref().unwrap().1.clone();
        assert_eq!(captured, b"hello world");
    }

    #[test]
    fn job_store_overwrite_same_id() {
        let store = JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        };
        let (tx1, _rx1) = oneshot::channel::<JobResult>();
        let (tx2, _rx2) = oneshot::channel::<JobResult>();
        store.jobs.lock().unwrap().insert(1, tx1);
        // Overwriting drops the old sender
        store.jobs.lock().unwrap().insert(1, tx2);
        assert_eq!(store.jobs.lock().unwrap().len(), 1);
    }

    // -----------------------------------------------------------------------
    // JobResult variants
    // -----------------------------------------------------------------------

    #[test]
    fn job_result_exec_fields() {
        let r = JobResult::Exec {
            stdout: b"output".to_vec(),
            stderr: b"err".to_vec(),
            exit_code: 0,
        };
        match r {
            JobResult::Exec { stdout, stderr, exit_code } => {
                assert_eq!(stdout, b"output");
                assert_eq!(stderr, b"err");
                assert_eq!(exit_code, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_exec_nonzero_exit() {
        let r = JobResult::Exec {
            stdout: vec![],
            stderr: b"command not found".to_vec(),
            exit_code: 127,
        };
        match r {
            JobResult::Exec { exit_code, .. } => assert_eq!(exit_code, 127),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_write_file_success() {
        let r = JobResult::WriteFile { success: true, error: None };
        match r {
            JobResult::WriteFile { success, error } => {
                assert!(success);
                assert!(error.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_write_file_error() {
        let r = JobResult::WriteFile { success: false, error: Some("permission denied".into()) };
        match r {
            JobResult::WriteFile { success, error } => {
                assert!(!success);
                assert_eq!(error.unwrap(), "permission denied");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_read_file_with_data() {
        let r = JobResult::ReadFile { data: Some(b"contents".to_vec()), error: None };
        match r {
            JobResult::ReadFile { data, error } => {
                assert_eq!(data.unwrap(), b"contents");
                assert!(error.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_read_file_not_found() {
        let r = JobResult::ReadFile { data: None, error: Some("not found".into()) };
        match r {
            JobResult::ReadFile { data, error } => {
                assert!(data.is_none());
                assert_eq!(error.unwrap(), "not found");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_result_error() {
        let r = JobResult::Error { message: "internal failure".into() };
        match r {
            JobResult::Error { message } => assert_eq!(message, "internal failure"),
            _ => panic!("wrong variant"),
        }
    }

    // -----------------------------------------------------------------------
    // clone_fd
    // -----------------------------------------------------------------------

    #[test]
    fn clone_fd_valid_file() {
        use std::io::Write;
        use std::os::unix::io::AsRawFd;
        // Use a pipe as a valid FD source
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        let raw_write = write_fd.as_raw_fd();
        let _raw_read = read_fd.as_raw_fd();
        let mut cloned = clone_fd(raw_write).unwrap();
        cloned.write_all(b"test").unwrap();
        drop(read_fd);
        drop(write_fd);
    }

    #[test]
    fn clone_fd_invalid_fd_fails() {
        // -1 is universally an invalid file descriptor in POSIX.
        // This avoids multithreaded race conditions where a closed FD
        // is instantly reused by another test.
        let result = clone_fd(-1);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Job completion via oneshot (integration-unit)
    // -----------------------------------------------------------------------

    #[test]
    fn job_oneshot_send_receive() {
        let (tx, rx) = oneshot::channel::<JobResult>();
        tx.send(JobResult::Exec {
            stdout: b"hello".to_vec(),
            stderr: vec![],
            exit_code: 0,
        }).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(rx).unwrap();
        match result {
            JobResult::Exec { stdout, exit_code, .. } => {
                assert_eq!(stdout, b"hello");
                assert_eq!(exit_code, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn job_oneshot_dropped_sender() {
        let (tx, rx) = oneshot::channel::<JobResult>();
        drop(tx); // Simulate client disconnect

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(rx);
        assert!(result.is_err()); // RecvError
    }

    #[tokio::test]
    async fn quiescence_timeout_fires() {
        let job_store = Arc::new(JobStore {
            jobs: Mutex::new(HashMap::new()),
            active_exec: Mutex::new(None),
            snapshot_ready: Mutex::new(None),
        });
        let (tx, _rx) = std::sync::mpsc::channel();

        // 1. Never send SnapshotReady
        let start = std::time::Instant::now();
        let result = with_quiescence(&tx, &job_store, std::time::Duration::from_millis(100), || async {
            Ok(())
        }).await;

        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(elapsed.as_millis() >= 100);
    }
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

fn query_max_fs_event_id(db: &capsem_logger::DbWriter) -> i64 {
    db.reader().ok()
        .and_then(|r| r.query_raw("SELECT COALESCE(MAX(id),0) FROM fs_events").ok())
        .and_then(|json| {
            let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
            parsed["rows"].get(0)?.get(0)?.as_i64()
        })
        .unwrap_or(0)
}

async fn run_async_main_loop(
    args: Args,
    vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    _sm: capsem_core::host_state::HostStateMachine,
) -> Result<()> {
    let job_store = Arc::new(JobStore { 
        jobs: Mutex::new(HashMap::new()),
        active_exec: Mutex::new(None),
        snapshot_ready: Mutex::new(None),
    });
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

    let mcp_config = Arc::new(capsem_core::mcp::gateway::McpGatewayConfig {
        server_manager: tokio::sync::Mutex::new(capsem_core::mcp::server_manager::McpServerManager::new(
            mcp_servers,
            reqwest::Client::new(),
        )),
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

    let vm_ready = Arc::new(AtomicBool::new(false));

    let ctrl_tx_ipc = ctrl_tx.clone();
    let uds_path = args.uds_path.clone();
    let vm_id_ws = args.id.clone();
    let is_restore = args.checkpoint_path.is_some();
    let vm_for_vsock = Arc::clone(&vm);
    let vm_ready_vsock = Arc::clone(&vm_ready);
    let uds_path_vsock = uds_path.clone();
    tokio::spawn(async move {
        if let Err(e) = setup_vsock(VsockOptions {
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
                    ws.on_upgrade(move |socket| handle_terminal_socket(socket, ctrl_tx, term_rx))
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
            if let Err(e) = handle_ipc_connection(stream, tx_c, ipc_tx_pass, term_c, job_c, net_c, mcp_c, ready_c).await {
                error!("IPC error: {e:#}");
            }
        });
    }
}

pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    if fd == -1 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file descriptor -1"));
    }
    let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone()
}

struct VsockOptions {
    vm_id: String,
    vm: Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>,
    vsock_rx: mpsc::UnboundedReceiver<VsockConnection>,
    ipc_tx: broadcast::Sender<ProcessToService>,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    ctrl_rx: mpsc::Receiver<ServiceToProcess>,
    terminal_output: Arc<capsem_core::TerminalOutputQueue>,
    job_store: Arc<JobStore>,
    session_dir: PathBuf,
    cli_env: Vec<(String, String)>,
    guest_config: capsem_core::net::policy_config::GuestConfig,
    mitm_config: Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>,
    mcp_config: Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    net_state: Arc<capsem_core::SandboxNetworkState>,
    is_restore: bool,
    vm_ready: Arc<AtomicBool>,
    uds_path: PathBuf,
}

async fn setup_vsock(options: VsockOptions) -> Result<()> {
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

async fn handle_ipc_connection(
    stream: tokio::net::UnixStream,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    ipc_tx: broadcast::Sender<ProcessToService>,
    term_bcast_tx: broadcast::Sender<Vec<u8>>,
    job_store: Arc<JobStore>,
    net_state: Arc<capsem_core::SandboxNetworkState>,
    mcp_config: Arc<capsem_core::mcp::gateway::McpGatewayConfig>,
    vm_ready: Arc<AtomicBool>,
) -> Result<()> {
    let std_stream = stream.into_std()?;
    let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) = channel_from_std(std_stream)?;

    // Serialize all IPC writes through a single channel to prevent concurrent
    // sendmsg() interleaving that corrupts the data stream. tokio_unix_ipc's
    // Sender::send() writes header + payload as two separate syscalls with no
    // internal locking, so concurrent use from multiple tasks is unsafe.
    let (ipc_tx_out, mut ipc_rx_out) = mpsc::channel::<ProcessToService>(256);
    tokio::spawn(async move {
        while let Some(msg) = ipc_rx_out.recv().await {
            if tx.send(msg).await.is_err() { break; }
        }
    });

    while let Ok(msg) = rx.recv().await {
        match msg {
            ServiceToProcess::StartTerminalStream => {
                    info!("Starting terminal stream for connection");
                    let out_tx = ipc_tx_out.clone();
                    let mut term_rx = term_bcast_tx.subscribe();
                    tokio::spawn(async move {
                        while let Ok(data) = term_rx.recv().await {
                            if out_tx.send(ProcessToService::TerminalOutput { data }).await.is_err() { break; }
                        }
                    });

                    let out_tx2 = ipc_tx_out.clone();
                    let mut rx_c = ipc_tx.subscribe();
                    tokio::spawn(async move {
                        while let Ok(msg) = rx_c.recv().await {
                            if out_tx2.send(msg).await.is_err() { break; }
                        }
                    });
                }
                ServiceToProcess::Ping => {
                    if vm_ready.load(Ordering::Acquire) {
                        let _ = ipc_tx_out.send(ProcessToService::Pong).await;
                    } else {
                        debug!("Ping received but VM not ready, closing connection");
                        return Ok(());
                    }
                }
                ServiceToProcess::TerminalInput { data } => { let _ = ctrl_tx.send(ServiceToProcess::TerminalInput { data }).await; }
                ServiceToProcess::TerminalResize { cols, rows } => { let _ = ctrl_tx.send(ServiceToProcess::TerminalResize { cols, rows }).await; }
                ServiceToProcess::Exec { id, command } => {
                    let job_store = job_store.clone();
                    let ctrl_tx = ctrl_tx.clone();
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        info!(id, command, "Received Exec command via IPC");
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);

                        // Set as active exec to start capturing output
                        *job_store.active_exec.lock().unwrap() = Some((id, Vec::new()));

                        let _ = ctrl_tx.send(ServiceToProcess::Exec { id, command }).await;
                        match j_rx.await {
                            Ok(JobResult::Exec { stdout, stderr, exit_code }) => {
                                info!(id, exit_code, "Sending ExecResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ExecResult { id, stdout, stderr, exit_code }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending Exec error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ExecResult { id, stdout: vec![], stderr: message.into_bytes(), exit_code: -1 }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for Exec");
                            }
                        }
                    });
                }
                ServiceToProcess::WriteFile { id, path, data } => {
                    let job_store = job_store.clone();
                    let ctrl_tx = ctrl_tx.clone();
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        info!(id, path, len = data.len(), "Received WriteFile command via IPC");
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);
                        let _ = ctrl_tx.send(ServiceToProcess::WriteFile { id, path, data }).await;
                        match j_rx.await {
                            Ok(JobResult::WriteFile { success, error }) => {
                                info!(id, success, "Sending WriteFileResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::WriteFileResult { id, success, error }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending WriteFile error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::WriteFileResult { id, success: false, error: Some(message) }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for WriteFile");
                            }
                        }
                    });
                }
                ServiceToProcess::ReadFile { id, path } => {
                    let job_store = job_store.clone();
                    let ctrl_tx = ctrl_tx.clone();
                    let ipc_tx_out = ipc_tx_out.clone();
                    tokio::spawn(async move {
                        info!(id, path, "Received ReadFile command via IPC");
                        let (j_tx, j_rx) = oneshot::channel();
                        job_store.jobs.lock().unwrap().insert(id, j_tx);
                        let _ = ctrl_tx.send(ServiceToProcess::ReadFile { id, path }).await;
                        match j_rx.await {
                            Ok(JobResult::ReadFile { data, error }) => {
                                info!(id, success = data.is_some(), "Sending ReadFileResult back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ReadFileResult { id, data, error }).await;
                            }
                            Ok(JobResult::Error { message }) => {
                                error!(id, message, "Sending ReadFile error back via IPC");
                                let _ = ipc_tx_out.send(ProcessToService::ReadFileResult { id, data: None, error: Some(message) }).await;
                            }
                            _ => {
                                error!(id, "Job result channel closed for ReadFile");
                            }
                        }
                    });
                }
                ServiceToProcess::ReloadConfig => {
                    info!("Reloading policies from disk");
                    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();

                    let new_domain = Arc::new(capsem_core::net::policy_config::settings_to_domain_policy(&capsem_core::net::policy_config::resolve_settings(&user_sf, &corp_sf)));
                    let new_network = Arc::new(capsem_core::net::policy_config::build_network_policy(&capsem_core::net::policy_config::resolve_settings(&user_sf, &corp_sf)));

                    let user_mcp = user_sf.mcp.clone().unwrap_or_default();
                    let corp_mcp = corp_sf.mcp.clone().unwrap_or_default();
                    let new_mcp = Arc::new(user_mcp.to_policy(&corp_mcp));

                    *net_state.policy.write().unwrap() = new_network;
                    *mcp_config.domain_policy.write().unwrap() = Arc::clone(&new_domain);
                    *mcp_config.policy.write().await = new_mcp;

                    let _ = ipc_tx_out.send(ProcessToService::Pong).await;
                }
                ServiceToProcess::Shutdown => {
                    let _ = ctrl_tx.send(ServiceToProcess::Shutdown).await;
                    info!("Received Shutdown command, exiting IPC loop gracefully");
                    break;
                }
                ServiceToProcess::Suspend { checkpoint_path } => {
                    info!("Received Suspend command, forwarding to ctrl channel");
                    let _ = ctrl_tx.send(ServiceToProcess::Suspend { checkpoint_path }).await;
                }
                ServiceToProcess::PrepareSnapshot
                | ServiceToProcess::Unfreeze
                | ServiceToProcess::Resume => {
                    // These are sent directly by process internals (quiescence helper),
                    // not expected over IPC from service.
                    warn!("unexpected lifecycle IPC command received");
                }
        }
    }
    Ok(())
}

async fn handle_terminal_socket(
    ws: axum::extract::ws::WebSocket,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    mut term_rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
) {
    let (mut client_write, mut client_read) = ws.split();

    let mut rx_task = tokio::spawn(async move {
        while let Ok(data) = term_rx.recv().await {
            if client_write.send(axum::extract::ws::Message::Binary(data.into())).await.is_err() {
                break;
            }
        }
    });

    let ctrl_tx_c = ctrl_tx.clone();
    let mut tx_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = client_read.next().await {
            match msg {
                axum::extract::ws::Message::Binary(b) => {
                    let _ = ctrl_tx_c.send(ServiceToProcess::TerminalInput { data: b.to_vec() }).await;
                }
                axum::extract::ws::Message::Text(t) => {
                    if let Ok(resize) = serde_json::from_str::<serde_json::Value>(t.as_str()) {
                        if let (Some(cols), Some(rows)) = (
                            resize.get("cols").and_then(|v| v.as_u64()),
                            resize.get("rows").and_then(|v| v.as_u64())
                        ) {
                            let _ = ctrl_tx_c.send(ServiceToProcess::TerminalResize { 
                                cols: cols as u16, 
                                rows: rows as u16 
                            }).await;
                        }
                    }
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut rx_task => { tx_task.abort(); },
        _ = &mut tx_task => { rx_task.abort(); },
    }
}
