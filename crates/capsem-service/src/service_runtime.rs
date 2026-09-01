use super::*;

pub(super) async fn run_service() -> Result<()> {
    let args = Args::parse();

    let mut run_dir = capsem_foundation::paths::capsem_run_dir();
    let _ = std::fs::create_dir_all(&run_dir);
    if let Ok(resolved) = run_dir.canonicalize() {
        run_dir = resolved;
    }

    let _telemetry_guard = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-service",
        sink: capsem_foundation::telemetry::LogSink::File {
            path: run_dir.join("service.log"),
        },
        default_filter: "info",
    })?;
    capsem_foundation::telemetry::install_panic_logger("capsem-service");
    let service_launch_span = tracing::info_span!(
        target: "capsem.launch",
        capsem_foundation::telemetry::LAUNCH_SERVICE_SPAN,
        status = tracing::field::Empty,
    );

    service_launch_span.in_scope(|| info!("capsem-service starting up"));
    info!(args = ?args, run_dir = %run_dir.display(), "environment initialized");

    // Optional parent-watch. Symmetric with the companion (tray/gateway)
    // reaper: if the test harness that spawned us dies abruptly, bail
    // rather than linger. Only armed when --parent-pid is passed.
    if let Some(ppid) = args.parent_pid {
        match capsem_guard::watch_parent_or_exit(Some(ppid)) {
            Ok(()) => {}
            Err(e) => {
                info!(parent_pid = ppid, error = %e, "parent watch not armed; exiting 0");
                return Ok(());
            }
        }
    }

    let instances_dir = run_dir.join("instances");
    let sessions_dir = run_dir.join("sessions");
    let persistent_dir = run_dir.join("persistent");
    let _ = std::fs::create_dir_all(&instances_dir);
    let _ = std::fs::create_dir_all(&sessions_dir);
    let _ = std::fs::create_dir_all(&persistent_dir);

    let service_sock = args.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    // Self-idempotent startup. Four parallel `capsem-service --uds-path X`
    // invocations must converge on exactly one running service.
    //
    //   1. Fast probe without locking: if someone matching our version is
    //      already serving, exit 0 (happy path for tests and re-runs).
    //   2. Take an flock next to the socket for the critical section:
    //      probe again (double-check), remove any stale socket, bind.
    //      Drop the lock the moment bind() succeeds so peers waiting for
    //      the lock can fast-probe us on their next iteration.
    //   3. Version mismatch refuses to start (do not auto-kill -- destructive).
    //
    // On crash the flock releases automatically (fd close), so failed
    // startups never wedge subsequent ones.
    let current_version = env!("CARGO_PKG_VERSION");
    let probe_timeout = std::time::Duration::from_millis(500);

    // Fast path: someone else already serves a compatible version.
    if service_sock.exists() {
        if let Ok(Some(running)) = startup::probe_running_version(&service_sock, probe_timeout).await {
            if running == current_version {
                info!(
                    socket = %service_sock.display(),
                    version = %running,
                    "compatible capsem-service already running; exiting 0"
                );
                return Ok(());
            }
            eprintln!(
                "capsem-service {} is already running at {}, but this binary is {}.\n\
                 Stop the running service before starting a new one.",
                running,
                service_sock.display(),
                current_version
            );
            return Err(anyhow::anyhow!(
                "version mismatch with running service (running: {}, this: {})",
                running,
                current_version
            ));
        }
    }

    let lock_path = service_sock.with_extension("lock");
    let startup_lock = match startup::StartupLock::acquire(&lock_path, std::time::Duration::from_secs(30))? {
        Some(lock) => lock,
        None => {
            return Err(anyhow::anyhow!(
                "another capsem-service startup holds {} after 30s; aborting",
                lock_path.display()
            ));
        }
    };

    // Under lock: double-check a peer didn't finish starting while we waited.
    if service_sock.exists() {
        match startup::probe_running_version(&service_sock, probe_timeout).await {
            Ok(Some(running)) if running == current_version => {
                info!(
                    socket = %service_sock.display(),
                    version = %running,
                    "peer starter won the race; exiting 0"
                );
                return Ok(());
            }
            Ok(Some(running)) => {
                return Err(anyhow::anyhow!(
                    "version mismatch with running service (running: {}, this: {})",
                    running,
                    current_version
                ));
            }
            Ok(None) => {
                info!(socket = %service_sock.display(), "removing stale socket");
                let _ = std::fs::remove_file(&service_sock);
            }
            Err(e) => {
                warn!(error = %e, socket = %service_sock.display(),
                    "probe failed under lock; removing socket and continuing");
                let _ = std::fs::remove_file(&service_sock);
            }
        }
    }
    // Keep `startup_lock` alive until after UnixListener::bind below. Released
    // where we explicitly drop it, right after bind succeeds.
    let startup_lock_guard = startup_lock;

    let process_binary = args
        .process_binary
        .unwrap_or_else(|| PathBuf::from("cache/target/cargo/debug/capsem-process"));
    let assets_base_dir = args
        .assets_dir
        .unwrap_or_else(|| run_dir.parent().unwrap().join("assets"));

    // Load v2 manifest if available. In dev mode (no manifest or v1), use None.
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest_path = if assets_base_dir.join("manifest.json").exists() {
        Some(assets_base_dir.join("manifest.json"))
    } else if assets_base_dir.parent().unwrap().join("manifest.json").exists() {
        Some(assets_base_dir.parent().unwrap().join("manifest.json"))
    } else {
        None
    };

    let manifest = manifest_path.and_then(|path| {
        let content = std::fs::read_to_string(&path).ok()?;
        match capsem_assets::asset_manager::ManifestV2::from_json(&content) {
            Ok(m) => {
                info!(asset_version = %m.assets.current, "loaded manifest");
                Some(Arc::new(m))
            }
            Err(e) => {
                warn!(error = %e, "failed to parse manifest");
                None
            }
        }
    });

    let registry_path = run_dir.join("persistent_registry.json");
    let persistent_registry = PersistentRegistry::load(registry_path);
    info!(
        persistent_vms = persistent_registry.data.vms.len(),
        "loaded persistent VM registry"
    );

    match capsem_core::credential_broker::hydrate_credential_runtime_cache_from_durable_store() {
        Ok(count) => {
            info!(
                component = "credential_store",
                status = "ready",
                loaded_count = count,
                "credential broker runtime cache hydrated"
            );
        }
        Err(error) => {
            warn!(
                component = "credential_store",
                status = "degraded",
                error = %error,
                "credential broker runtime cache hydration failed"
            );
        }
    }

    // Clean up stale assets (legacy v*/ dirs, unreferenced hash-named files).
    // Preserve every filename referenced by the profile catalog or by saved VM
    // boot pins so cleanup cannot strand a valid profile or persistent VM.
    if let Some(ref m) = manifest {
        match ProfileCatalog::load_default() {
            Ok(catalog) => {
                let mut preserve = profile_catalog_asset_filenames(&catalog);
                preserve.extend(persistent_registry_asset_filenames(&persistent_registry));
                match capsem_assets::asset_manager::cleanup_unused_assets_preserving(&assets_base_dir, m, preserve) {
                    Ok(removed) if !removed.is_empty() => {
                        info!(count = removed.len(), "cleaned up stale assets");
                    }
                    Err(e) => warn!(error = %e, "asset cleanup failed"),
                    _ => {}
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "profile catalog unavailable; skipping asset cleanup"
                );
            }
        }
    }

    let magika_session = magika::Session::builder()
        .with_inter_threads(1)
        .with_intra_threads(1)
        .build()
        .expect("failed to init magika file-type detection");

    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let asset_reconcile = load_asset_reconcile_state(&asset_status_path);
    let profile_summary_cache = build_profile_summary_cache()
        .map_err(|AppError(_, message)| anyhow!("failed to build profile summary cache: {message}"))?;
    let profile_cache =
        build_profile_cache().map_err(|AppError(_, message)| anyhow!("failed to build profile cache: {message}"))?;
    prewarm_system_overlay_templates(&run_dir, &profile_cache);
    prewarm_vm_asset_hash_cache(&assets_base_dir, manifest.as_deref(), &current_version);
    let profile_rule_cache = build_profile_rule_cache(None)
        .map_err(|AppError(_, message)| anyhow!("failed to build profile rule cache: {message}"))?;
    let profile_mcp_default_cache = build_profile_mcp_default_cache(None)
        .map_err(|AppError(_, message)| anyhow!("failed to build profile MCP default cache: {message}"))?;
    let profile_plugin_policy_cache = build_profile_plugin_policy_cache(None)
        .map_err(|AppError(_, message)| anyhow!("failed to build profile plugin cache: {message}"))?;
    let profile_mutation_db = ServiceState::open_profile_mutation_db_handle(&run_dir)?;
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(persistent_registry),
        process_binary: process_binary.clone(),
        assets_dir: assets_base_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: RwLock::new(manifest),
        current_version,
        asset_reconcile: Mutex::new(asset_reconcile),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: Mutex::new(magika_session),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(profile_summary_cache),
        profile_cache: Mutex::new(profile_cache),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: Mutex::new(profile_rule_cache),
        profile_mcp_default_cache: Mutex::new(profile_mcp_default_cache),
        profile_plugin_policy_cache: Mutex::new(profile_plugin_policy_cache),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db,
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        list_response_cache: Mutex::new(None),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
        update_lock: tokio::sync::Mutex::new(()),
        update_restart: tokio::sync::Notify::new(),
        #[cfg(test)]
        _test_tempdir: None,
    });
    hydrate_startup_route_caches(&state).map_err(|AppError(_, message)| anyhow!("{message}"))?;
    state.hydrate_session_db_handles();
    state.reconcile_persistent_defunct_from_logs();

    asset_background::start_startup_ensure(Arc::clone(&state));

    // Reap capsem-process orphans from any prior service run sharing this
    // run_dir. A previous service that crashed (SIGKILL) or was killed by
    // tests left its per-VM processes alive; they still reference our
    // run_dir via --session-dir and will never die on their own. Do this
    // BEFORE stale-socket removal so the orphans get a chance to clean up
    // their own sockets on SIGTERM.
    reap_orphan_capsem_processes(&run_dir);

    // Check for running instances to reattach
    info!("scanning for existing sandboxes in {}", instances_dir.display());
    if let Ok(entries) = std::fs::read_dir(&instances_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "sock" {
                    // Stale socket from previous run, remove it
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    // Periodic cleanup of stale instances (replaces per-handler calls).
    {
        let state_for_cleanup = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                state_for_cleanup.cleanup_stale_instances();
            }
        });
    }

    let app = build_service_router(Arc::clone(&state));

    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = match service_launch_span.in_scope(|| UnixListener::bind(&service_sock).context("failed to bind UDS")) {
        Ok(uds) => {
            service_launch_span.record("status", "ok");
            uds
        }
        Err(error) => {
            service_launch_span.record("status", "error");
            return Err(error);
        }
    };
    // We hold the socket, so we are the service a reaper should kill. Claim the
    // pidfile before releasing the startup lock: from the moment a peer can
    // fast-probe us and exit 0, `$run_dir/service.pid` must already name us and
    // not the peer, whose guard would otherwise erase our pid on its way out.
    //
    // Without a pidfile, every cleanup targeting `$run_dir/service.pid` no-ops
    // silently -- indistinguishable from success -- and the asset gate left a
    // service (and its tray) behind on every run.
    let _pidfile_guard = ServicePidfile::claim(run_dir.join("service.pid"));

    // Socket is bound; release the startup lock so any peer starter still in
    // its flock wait can fast-probe us and exit 0.
    drop(startup_lock_guard);

    if should_start_automatic_update_loop(args.parent_pid) {
        let state_for_updates = Arc::clone(&state);
        // Announced before the first sleep. A proof that waits for this loop to
        // say something had no way to tell "not started" from "started and
        // quiet", and spent a release cycle on the difference.
        info!(
            initial_delay_secs =
                automatic_update_delay(AUTOMATIC_UPDATE_INITIAL_DELAY_ENV, AUTOMATIC_UPDATE_INITIAL_DELAY_SECS,)
                    .as_secs(),
            poll_secs = automatic_update_delay(AUTOMATIC_UPDATE_POLL_ENV, AUTOMATIC_UPDATE_POLL_SECS,).as_secs(),
            "automatic release polling started"
        );
        tokio::spawn(async move {
            run_automatic_update_loop(state_for_updates).await;
        });
    } else {
        info!(
            parent_pid = args.parent_pid,
            "automatic release polling is disabled for the bounded test service"
        );
    }

    // Spawn companion processes (gateway + tray) in the background so the UDS
    // starts accepting immediately. The previous .await here delayed accept()
    // by up to 5s on every startup while polling gateway.token into existence
    // -- fatal under parallel test load. Companions are stateless and can come
    // up after the service is already serving clients.
    struct CompanionManager {
        children: Vec<tokio::process::Child>,
        spawn_task: Option<tokio::task::JoinHandle<()>>,
    }
    let companions = Arc::new(std::sync::Mutex::new(CompanionManager {
        children: Vec::new(),
        spawn_task: None,
    }));
    let companions_for_spawn = Arc::clone(&companions);
    let service_sock_for_spawn = service_sock.clone();
    let run_dir_for_spawn = run_dir.clone();
    let gateway_binary = args.gateway_binary;
    let gateway_port = args.gateway_port;
    let tray_binary = args.tray_binary;

    let spawn_task = tokio::spawn(async move {
        let spawned = spawn_companions(
            &service_sock_for_spawn,
            &run_dir_for_spawn,
            gateway_binary,
            gateway_port,
            tray_binary,
        )
        .await;
        companions_for_spawn.lock().unwrap().children.extend(spawned);
    });
    companions.lock().unwrap().spawn_task = Some(spawn_task);

    let shutdown_state = state.clone();
    let companions_for_shutdown = Arc::clone(&companions);
    axum::serve(uds, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = shutdown_signal() => {}
                _ = shutdown_state.update_restart.notified() => {
                    info!("service restart requested after binary update");
                }
            }
            info!("service shutting down, killing companions and VM processes");
            // Companions FIRST. kill_all_vm_processes has an unconditional
            // 500ms SIGTERM grace sleep; if companion-kill ran after it, a
            // downstream `_ensure-service` (which itself sleeps 500ms before
            // spawning the next service) would race with companion exit and
            // the new gateway would fail to bind :19222.

            // Scoped so the MutexGuard is definitely dropped before the
            // awaits below; relying on `drop(manager)` alone was fragile
            // enough that the compiler's Send analysis tripped once the
            // surrounding future gained other Send requirements.
            let children = {
                let mut manager = companions_for_shutdown.lock().unwrap();
                if let Some(task) = manager.spawn_task.take() {
                    task.abort();
                }
                std::mem::take(&mut manager.children)
            };

            info!(count = children.len(), "killing companions");
            for mut child in children {
                info!(pid = child.id(), "killing companion process");
                let _ = child.kill().await;
            }
            info!("killing all VM processes");
            kill_all_vm_processes(&shutdown_state);
            info!("shutdown complete");
        })
        .await
        .context("server error")?;

    Ok(())
}

/// Parse `ps -ax -o pid=,command=` output and return the PIDs of every
/// `capsem-process` instance whose `--session-dir` lives inside `run_dir`.
///
/// A SIGKILL to capsem-service (crash, OOM, `svc.proc.kill()` in recovery
/// tests) does not propagate to children, so every per-VM `capsem-process`
/// it spawned becomes an orphan with its `--session-dir` still pointing
/// under the dead service's run_dir. When a replacement service starts on
/// the same run_dir it must reap these orphans or the host accumulates
/// wedged Apple VZ instances and leaked vsock ports.
///
/// Matches on the `--session-dir <run_dir>/` prefix because the spawn-side
/// always writes the absolute session dir as `<run_dir>/sessions/<id>` or
/// `<run_dir>/persistent/<id>`. Pure -- no side effects -- so the matching
/// is unit-testable without spawning real processes.
pub(super) fn find_orphan_capsem_pids(ps_output: &str, run_dir: &std::path::Path) -> Vec<i32> {
    let run_dir_str = run_dir.display().to_string();
    let marker = format!("--session-dir {run_dir_str}");
    let mut pids = Vec::new();
    for line in ps_output.lines() {
        let line = line.trim_start();
        if !line.contains("capsem-process") {
            continue;
        }
        if !line.contains(&marker) {
            continue;
        }
        let Some((pid_str, _)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if let Ok(pid) = pid_str.parse::<i32>() {
            pids.push(pid);
        }
    }
    pids
}

/// Reap `capsem-process` orphans from a prior service run that shared this
/// run_dir. See [`find_orphan_capsem_pids`] for the why; this wrapper reads the
/// shared syscall-backed process table, applies the match, and escalates
/// SIGTERM -> 2s poll -> SIGKILL. An unreadable table is reported; no matching
/// process is ordinary and silent.
pub(super) fn reap_orphan_capsem_processes(run_dir: &std::path::Path) {
    let table = match capsem_core::proctable::running_processes() {
        Ok(table) => table,
        // Loudly. This used to shell out to `ps` and return silently when the
        // spawn failed, which is indistinguishable from finding no orphans --
        // and under a sandbox, where setuid `ps` cannot be exec'd, that was
        // every single time.
        Err(error) => {
            tracing::warn!(
                error = %error,
                "cannot read the process table; orphaned capsem-process children \
                 of a previous service run will not be reaped"
            );
            return;
        }
    };
    let orphan_pids = find_orphan_capsem_pids(&table, run_dir);
    if orphan_pids.is_empty() {
        return;
    }

    tracing::warn!(
        count = orphan_pids.len(),
        ?orphan_pids,
        "reaping capsem-process orphans from previous service run"
    );

    for pid in &orphan_pids {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(*pid), nix::sys::signal::Signal::SIGTERM);
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let survivors: Vec<i32> = orphan_pids
            .iter()
            .copied()
            .filter(|&pid| unsafe { nix::libc::kill(pid, 0) } == 0)
            .collect();
        if survivors.is_empty() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            tracing::warn!(
                count = survivors.len(),
                ?survivors,
                "orphan capsem-process did not exit, SIGKILLing"
            );
            for pid in survivors {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Kill every per-VM `capsem-process` the service has spawned.
///
/// Called from the graceful-shutdown path so a SIGTERM to capsem-service does
/// NOT orphan running guests. Without this, each service shutdown leaked one
/// `capsem-process` per live VM, which in turn held Apple VZ memory -- making
/// long test runs increasingly slow until boots timed out.
pub(super) fn kill_all_vm_processes(state: &ServiceState) {
    let pids_and_sockets: Vec<(u32, PathBuf, PathBuf, bool)> = {
        let instances = state.instances.lock().unwrap();
        instances
            .values()
            .map(|i| (i.pid, i.uds_path.clone(), i.session_dir.clone(), i.persistent))
            .collect()
    };
    // Nothing to reap -- skip the grace sleep. `_ensure-service` only waits
    // 500ms before respawning the service, so every unnecessary ms here
    // widens the orphan-gateway race.
    if pids_and_sockets.is_empty() {
        return;
    }
    let mut signaled_any_vm = false;
    for (pid, uds_path, session_dir, persistent) in &pids_and_sockets {
        let pid = *pid;
        if pid > 0 {
            // SIGTERM first so capsem-process gets a chance to run its own cleanup
            // (save state, unmount virtiofs). Graceful_shutdown is already holding
            // the axum server open briefly so a short wait is acceptable.
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
            signaled_any_vm = true;
        }
        let _ = std::fs::remove_file(uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
        if !persistent {
            let _ = std::fs::remove_dir_all(session_dir);
        }
    }
    if !signaled_any_vm {
        return;
    }

    // Bounded wait: poll for up to 2 seconds
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(2);
    let poll_interval = std::time::Duration::from_millis(100);

    loop {
        let survivors: Vec<u32> = pids_and_sockets
            .iter()
            .map(|(pid, _, _, _)| *pid)
            .filter(|&pid| pid > 0 && unsafe { nix::libc::kill(pid as i32, 0) } == 0)
            .collect();

        if survivors.is_empty() {
            break;
        }

        if start.elapsed() >= timeout {
            tracing::warn!(
                count = survivors.len(),
                "some VMs survived SIGTERM, escalating to SIGKILL"
            );
            for pid in survivors {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            break;
        }

        std::thread::sleep(poll_interval);
    }
}

pub(super) async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}

/// Find a sibling binary next to the current executable, falling back to
/// cache/target/cargo/debug/ for development builds.
pub(super) fn find_sibling_binary(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.parent().unwrap().join(name);
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from(format!("cache/target/cargo/debug/{name}"))
}

/// Open a log file for a companion process, returning Stdio handles for stdout and stderr.
/// Falls back to null if the file cannot be opened.
pub(super) fn companion_stdio(log_path: &std::path::Path) -> (std::process::Stdio, std::process::Stdio) {
    match std::fs::OpenOptions::new().create(true).append(true).open(log_path) {
        Ok(f) => {
            let stdout = f
                .try_clone()
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null());
            let stderr = std::process::Stdio::from(f);
            (stdout, stderr)
        }
        Err(_) => (std::process::Stdio::null(), std::process::Stdio::null()),
    }
}

/// Spawn the gateway and tray as child processes of the service.
pub(super) async fn spawn_companions(
    service_sock: &std::path::Path,
    run_dir: &std::path::Path,
    gateway_bin: Option<PathBuf>,
    gateway_port: Option<u16>,
    tray_bin: Option<PathBuf>,
) -> Vec<tokio::process::Child> {
    // tray_bin is only consumed by the macOS-gated tray-spawn block below.
    // On Linux there's no system tray, so the parameter is intentionally
    // unused -- silence the unused-variable warning without breaking the
    // platform-agnostic signature.
    #[cfg(not(target_os = "macos"))]
    let _ = tray_bin;

    let mut children = Vec::new();

    // Log files for companion processes. Tests set CAPSEM_RUN_DIR for isolation;
    // when it is set, keep logs under that run_dir so parallel test workers do
    // not trample each other's gateway.log in ~/Library/Logs/capsem.
    let log_dir = if std::env::var("CAPSEM_RUN_DIR").is_ok() {
        run_dir.join("logs")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Logs/capsem"))
            .unwrap_or_else(|_| run_dir.join("logs"))
    };
    let _ = std::fs::create_dir_all(&log_dir);

    // 1. Spawn capsem-gateway (TCP reverse proxy -> UDS)
    // A previous service may have exited before its gateway removed runtime
    // markers. Never let those stale files satisfy our readiness poll for the
    // replacement gateway.
    for name in ["gateway.token", "gateway.port", "gateway.pid"] {
        let path = run_dir.join(name);
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, "failed to remove stale gateway runtime file");
            }
        }
    }

    let gateway_bin = gateway_bin.unwrap_or_else(|| find_sibling_binary("capsem-gateway"));
    let (gw_out, gw_err) = companion_stdio(&log_dir.join("gateway.log"));
    info!(binary = %gateway_bin.display(), "spawning capsem-gateway");

    let mut gw_cmd = tokio::process::Command::new(&gateway_bin);
    gw_cmd.arg("--uds-path").arg(service_sock);
    // Pin the gateway to the service's run_dir so gateway.{token,port,pid} land
    // in the same place we poll for them below and the same place clients read.
    gw_cmd.arg("--run-dir").arg(run_dir);
    // Parent-watch: the gateway exits the moment we die, even if we die
    // ungracefully (SIGKILL/OOM). capsem-guard enforces this on the gateway
    // side; we just have to hand it our PID.
    gw_cmd.arg("--parent-pid").arg(std::process::id().to_string());
    if let Some(port) = gateway_port {
        gw_cmd.arg("--port").arg(port.to_string());
    }
    let gateway_span = tracing::debug_span!(
        target: "capsem.launch",
        capsem_foundation::telemetry::LAUNCH_GATEWAY_SPAN,
        status = tracing::field::Empty,
    );
    match gateway_span.in_scope(|| gw_cmd.stdout(gw_out).stderr(gw_err).kill_on_drop(true).spawn()) {
        Ok(child) => {
            info!(pid = child.id(), "capsem-gateway spawned");
            children.push(child);

            // Wait for gateway to write token + port files (up to 5s)
            let token_path = run_dir.join("gateway.token");
            let port_path = run_dir.join("gateway.port");
            {
                let tp = token_path.clone();
                let pp = port_path.clone();
                let _ = capsem_foundation::poll::poll_until(
                    capsem_foundation::poll::PollOpts::new("gateway-ready", std::time::Duration::from_secs(5)),
                    || {
                        let tp = tp.clone();
                        let pp = pp.clone();
                        async move {
                            if tp.exists() && pp.exists() {
                                Some(())
                            } else {
                                None
                            }
                        }
                    },
                )
                .instrument(gateway_span.clone())
                .await;
            }
            if token_path.exists() && port_path.exists() {
                gateway_span.record("status", "ok");
            } else {
                gateway_span.record("status", "error");
            }

            // 2. Spawn capsem-tray (menu bar) -- only on macOS, only after gateway ready
            #[cfg(target_os = "macos")]
            if token_path.exists() {
                let tray_bin = tray_bin.unwrap_or_else(|| find_sibling_binary("capsem-tray"));
                let (tray_out, tray_err) = companion_stdio(&log_dir.join("tray.log"));
                info!(binary = %tray_bin.display(), "spawning capsem-tray");
                match tokio::process::Command::new(&tray_bin)
                    .arg("--parent-pid")
                    .arg(std::process::id().to_string())
                    .stdout(tray_out)
                    .stderr(tray_err)
                    .kill_on_drop(true)
                    .spawn()
                {
                    Ok(child) => {
                        info!(pid = child.id(), "capsem-tray spawned");
                        children.push(child);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to spawn capsem-tray (non-fatal)");
                    }
                }
            }
        }
        Err(e) => {
            gateway_span.record("status", "error");
            tracing::warn!(error = %e, "failed to spawn capsem-gateway (non-fatal)");
        }
    }

    children
}
