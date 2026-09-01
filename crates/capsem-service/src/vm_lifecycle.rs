use super::*;

// ---------------------------------------------------------------------------
// History endpoints
// ---------------------------------------------------------------------------

/// Helper: resolve session_dir from instance ID (running or persistent).
pub(super) fn persistent_entry_vm_id(entry: &PersistentVmEntry) -> String {
    if !entry.id.trim().is_empty() {
        return entry.id.clone();
    }
    entry
        .session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(&entry.name)
        .to_string()
}

pub(super) fn persistent_resume_state_fingerprint(state: &ServiceState, entry: &PersistentVmEntry) -> String {
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let active_profile = entry.session_dir.join(ACTIVE_PROFILE_DIR).join(ACTIVE_PROFILE_FILE);
    let rootfs = capsem_core::guest_share_dir(&entry.session_dir).join("system/rootfs.img");
    json!({
        "id": persistent_entry_vm_id(entry),
        "profile_id": entry.profile_id,
        "profile_revision": entry.profile_revision,
        "profile_payload_hash": entry.profile_payload_hash,
        "asset_pins": entry.asset_pins,
        "session_dir": entry.session_dir,
        "suspended": entry.suspended,
        "defunct": entry.defunct,
        "last_error": entry.last_error,
        "active_profile": small_file_fingerprint(&active_profile),
        "installed_manifest": small_file_fingerprint(&state.assets_dir.join("manifest.json")),
        "rootfs": file_metadata_fingerprint(&rootfs),
        "kernel": file_metadata_fingerprint(&boot_asset_pin_path(&state.assets_dir, arch, &entry.asset_pins.kernel)),
        "initrd": file_metadata_fingerprint(&boot_asset_pin_path(&state.assets_dir, arch, &entry.asset_pins.initrd)),
        "rootfs_asset": file_metadata_fingerprint(&boot_asset_pin_path(&state.assets_dir, arch, &entry.asset_pins.rootfs)),
    })
    .to_string()
}

pub(super) fn small_file_fingerprint(path: &StdPath) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

pub(super) fn file_metadata_fingerprint(path: &StdPath) -> Option<(u64, u128)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((metadata.len(), modified))
}

pub(super) fn find_persistent_entry_by_route_id(state: &ServiceState, id: &str) -> Option<PersistentVmEntry> {
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry
        .list()
        .find(|entry| persistent_entry_vm_id(entry) == id)
        .cloned();
    entry
}

pub(super) fn persistent_registry_key_for_route_id(state: &ServiceState, id: &str) -> Option<String> {
    let registry = state.persistent_registry.lock().unwrap();
    let key = registry
        .list()
        .find(|entry| persistent_entry_vm_id(entry) == id)
        .map(|entry| entry.name.clone());
    key
}

pub(super) fn resolve_session_dir(state: &ServiceState, id: &str) -> Result<PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    if let Some(i) = instances.get(id) {
        return Ok(i.session_dir.clone());
    }
    drop(instances);
    if let Some(entry) = find_persistent_entry_by_route_id(state, id) {
        return Ok(entry.session_dir);
    }
    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

/// GET /vms/{id}/history -- unified command history (exec + audit events).
pub(super) async fn handle_history(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<api::HistoryQuery>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let route_key = format!(
        "history:layer={}:limit={}:offset={}:search={}",
        params.layer,
        params.limit,
        params.offset,
        params.search.as_deref().unwrap_or("")
    );
    if let Some(body) = session_response_cache_get(&state, &id, &route_key, &db_path) {
        return Ok(json_bytes_response(body));
    }
    let session = history_ledger_for_vm(&state, &id).await?;
    let response = query_history_ledger(&session, &params);
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize history response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, &route_key, &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/history/processes -- process-centric view of audit events.
pub(super) async fn handle_history_processes(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    if let Some(body) = session_response_cache_get(&state, &id, "history_processes", &db_path) {
        return Ok(json_bytes_response(body));
    }
    let session = history_ledger_for_vm(&state, &id).await?;
    let processes = session.processes.into_iter().take(100).collect();
    let response = api::HistoryProcessesResponse { processes };
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize history processes response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, "history_processes", &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/history/counts -- exec and audit event counts.
pub(super) async fn handle_history_counts(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    if let Some(body) = session_response_cache_get(&state, &id, "history_counts", &db_path) {
        return Ok(json_bytes_response(body));
    }
    let session = history_ledger_for_vm(&state, &id).await?;
    let response = api::HistoryCountsResponse {
        exec_count: session.counts.exec_count,
        audit_count: session.counts.audit_count,
    };
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize history counts response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, "history_counts", &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/history/transcript -- raw PTY output (base64-encoded).
pub(super) async fn handle_history_transcript(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(_params): Query<api::TranscriptQuery>,
) -> Result<Json<api::TranscriptResponse>, AppError> {
    use base64::Engine;
    let session_dir = resolve_session_dir(&state, &id)?;
    let pty_log_path = session_dir.join("pty.log");

    if !pty_log_path.exists() {
        return Ok(Json(api::TranscriptResponse {
            content: String::new(),
            bytes: 0,
        }));
    }

    let output = std::fs::read(&pty_log_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read pty.log: {e}"),
        )
    })?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&output);
    Ok(Json(api::TranscriptResponse {
        bytes: output.len(),
        content: encoded,
    }))
}

/// Acquire the host-wide VZ lifecycle flock (`startup::VzHostLock`)
/// from an async context. The underlying `flock(2)` syscall is blocking
/// and can wait on a sibling service; wrap in `spawn_blocking` so we
/// don't stall a tokio worker.
///
/// Default wait budget is 60s -- the longest single suspend under `-n 4`
/// test load observed is ~15s, so 60s absorbs the typical p99. Returning
/// 503 on timeout tells the caller "try again" instead of blocking
/// indefinitely.
pub(super) fn requires_vz_host_lock() -> bool {
    cfg!(target_os = "macos")
}

pub(super) async fn acquire_vz_host_lock(
    mode: startup::VzHostLockMode,
) -> Result<Option<startup::VzHostLock>, AppError> {
    // This lock exists solely for an Apple Virtualization.framework
    // save/restore constraint. KVM VMs have independent VM fds and device
    // state; forcing every Linux service/xdist worker through the same flock
    // lets an exclusive suspend waiter starve behind unrelated cold starts.
    if !requires_vz_host_lock() {
        return Ok(None);
    }

    let result =
        tokio::task::spawn_blocking(move || startup::VzHostLock::acquire(mode, std::time::Duration::from_secs(60)))
            .await
            .map_err(|e| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("vz host lock task panicked: {e}"),
                )
            })?;
    match result {
        Ok(Some(guard)) => Ok(Some(guard)),
        Ok(None) => Err(AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            "another process holds the Apple VZ save/restore lock; retry shortly".into(),
        )),
        Err(e) => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("vz host lock acquire failed: {e:#}"),
        )),
    }
}

/// Wait for a process to exit, force-killing after timeout.
/// Wait for a VM process to exit, SIGKILLing it if it outstays `timeout`.
///
/// Returns whether it left on its own. A caller that asked to retain state
/// needs to know: everything the guest had not flushed when it was killed is
/// gone, and reporting that stop as a success is how acknowledged writes went
/// missing without a word.
pub(super) async fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) -> bool {
    if pid == 0 {
        return true;
    }
    let pid_i32 = pid as i32;
    let exited = || async move { (unsafe { nix::libc::kill(pid_i32, 0) } != 0).then_some(()) };
    if poll_until(process_exit_poll_options(timeout), exited).await.is_ok() {
        return true;
    }
    tracing::warn!(pid, "VM process did not exit within timeout, sending SIGKILL");
    let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid_i32), nix::sys::signal::Signal::SIGKILL);
    if poll_until(
        PollOpts::new("vm-process-sigkill", std::time::Duration::from_secs(2)),
        exited,
    )
    .await
    .is_err()
    {
        tracing::error!(pid, "VM process survived SIGKILL");
    }
    false
}

/// Shutdown a running VM process by ID. Returns (session_dir, persistent, pid).
///
/// `ShutdownMode::Retain` sends `ServiceToProcess::Shutdown` via IPC so
/// the guest agent can `sync()` and bash can run traps / save history, then
/// waits up to 5s for natural exit. The in-process 2.5s self-timer in
/// capsem-process (capsem-process/src/vsock.rs, ServiceToProcess::Shutdown
/// branch) sets the floor at ~2.5s. Required for `handle_stop` on
/// persistent VMs (preserves workspace state) and `handle_run` (session DB
/// rollup reads main.db after exit).
///
/// `ShutdownMode::Discard` means the caller is permanently deleting the session,
/// so its guest state and per-session ledger are explicitly disposable. Kill
/// capsem-process directly rather than paying its retained-session fs-monitor
/// reconciliation and WAL checkpoint. Closing the process tears down the VM;
/// the service still waits for that exit before deleting the session tree.
///
/// Either way, UDS socket / `.ready` files are removed inline and the
/// instance is removed from the registry before return. The leak detector
/// and suspend/resume both rely on "process is gone when this returns".
pub(super) async fn shutdown_vm_process(
    state: &ServiceState,
    id: &str,
    mode: ShutdownMode,
) -> Result<Option<(PathBuf, bool, u32)>, AppError> {
    // Teardown must not overlap save_state/restore_state, but it does not
    // need to block independent cold starts. Take the shared lifecycle rail
    // before shutdown bookkeeping so save/restore still gets a clean edge.
    let _vz_guard = state.save_restore_lock.read().await;
    let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Shared).await?;

    // Serialize VM teardown across the service. Concurrent deletes under
    // load starve each other: VZ guest teardown + DbWriter WAL checkpoint +
    // socket cleanup all compete, and a single shutdown can exceed the 1s
    // fast-path exit budget, which SIGKILLs capsem-process mid-checkpoint
    // and leaves a non-empty session.db-wal on disk (see
    // tests/capsem-session-lifecycle/test_wal_cleanup.py).
    // See web/docs/src/content/docs/gotchas/serialized-vm-shutdown.md.
    let _shutdown_guard = state.shutdown_lock.lock().await;

    let (uds_path, session_dir, pid, persistent) = {
        let instances = state.instances.lock().unwrap();
        let Some(i) = instances.get(id) else {
            return Ok(None);
        };
        let result = (i.uds_path.clone(), i.session_dir.clone(), i.pid, i.persistent);
        drop(instances);
        result
    };

    // Claim before signalling. The watcher may already have claimed a process
    // which exited independently; otherwise this intentional shutdown owns
    // the record and the watcher must not preserve it as a crash.
    let shutdown_claimed = claim_shutdown_instance(state, id);
    state.unregister_session_db_handle(id);

    if mode.retains_state() && shutdown_claimed {
        // Send shutdown command via IPC (or SIGTERM as fallback).
        let stream_res = tokio::net::UnixStream::connect(&uds_path).await;
        if let Ok(stream) = stream_res {
            if let Ok(mut std_stream) = stream.into_std() {
                if capsem_foundation::ipc_handshake::negotiate_initiator(
                    &mut std_stream,
                    "capsem-service",
                    capsem_foundation::telemetry::current_parent_traceparent(),
                )
                .is_ok()
                {
                    if let Ok((tx, _)) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream) {
                        capsem_core::try_send!("ipc_graceful_shutdown", tx.send(ServiceToProcess::Shutdown).await);
                    }
                }
            }
        } else if pid > 0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    } else if shutdown_claimed && pid > 0 {
        // Destructive delete has no state to flush. SIGKILL also prevents the
        // ordinary signal handler from doing a full workspace reconciliation
        // whose output would be deleted immediately afterward.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }

    tracing::debug!(id, shutdown_claimed, "shutdown_vm_process removing instance");

    // Wait for actual exit (poll_until + SIGKILL fallback), then clean up
    // sockets. Synchronous: callers must not see "shutdown returned" while
    // the process is still alive (leak detector + suspend/resume rely on it).
    let exited_cleanly = wait_for_process_exit(pid, mode.exit_timeout()).await;
    if !exited_cleanly && mode.retains_state() {
        // The guest was killed mid-sync on a shutdown that promised to keep
        // its state. Whatever had not reached the disk is gone, and this is
        // the only place that knows it.
        tracing::error!(
            id,
            timeout_secs = mode.exit_timeout().as_secs(),
            "VM was killed before it finished flushing; writes made just before              stop may be lost"
        );
    }
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));
    if !shutdown_claimed {
        tracing::debug!(id, "child watcher retained shutdown ownership");
        return Ok(None);
    }
    state
        .record_session_index_stop(id, "stopped", mode.session_dir_for_rollup(&session_dir))
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session index rollup failed for {id}: {error}"),
            )
        })?;

    drop(_shutdown_guard);
    Ok(Some((session_dir, persistent, pid)))
}

/// Tear down a warm-restore process that failed to reach ready while the
/// caller already holds the save/restore locks.
pub(super) async fn stop_failed_restore_process_under_lock(state: &ServiceState, id: &str) {
    let Some((uds_path, pid)) = ({
        let instances = state.instances.lock().unwrap();
        instances.get(id).map(|i| (i.uds_path.clone(), i.pid))
    }) else {
        return;
    };

    if pid > 0 {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }

    tracing::warn!(id, pid, "removing failed warm restore before cold fallback");
    state.instances.lock().unwrap().remove(id);
    state.unregister_session_db_handle(id);
    wait_for_process_exit(pid, std::time::Duration::from_secs(1)).await;
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));
}

#[tracing::instrument(skip_all, fields(vm_id = %id))]
pub(super) async fn handle_suspend(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Apple VZ corrupts the VirtioFS-backed overlay of a sibling VM if two
    // save_state / restore_state calls overlap. Serialize across all VMs
    // managed by this service. Held for the whole handler; released when
    // the child has exited and the checkpoint is durable.
    let _vz_guard = state.save_restore_lock.write().await;
    // Plus a host-wide flock so serialization survives pytest-xdist's
    // per-worker `capsem-service` processes. See `VzHostLock`.
    let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Exclusive).await?;

    let (uds_path, pid) = {
        let mut instances = state.instances.lock().unwrap();
        let i = instances
            .get_mut(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if !i.persistent {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                "ephemeral VMs cannot be suspended (persist first)".into(),
            ));
        }
        let result = (i.uds_path.clone(), i.pid);
        drop(instances);
        result
    };

    let stream = tokio::net::UnixStream::connect(&uds_path).await.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to connect to VM IPC: {e}"),
        )
    })?;
    let mut std_stream = stream.into_std().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to convert stream: {e}"),
        )
    })?;
    capsem_foundation::ipc_handshake::negotiate_initiator(
        &mut std_stream,
        "capsem-service",
        capsem_foundation::telemetry::current_parent_traceparent(),
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("IPC handshake failed: {e}")))?;
    let (tx, rx) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create IPC channel: {e}"),
        )
    })?;

    let checkpoint_path = RESUME_CHECKPOINT_NAME.to_string();
    tx.send(ServiceToProcess::Suspend { checkpoint_path })
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to send suspend command: {e}"),
            )
        })?;

    // Wait for process exit (channel closed). The process sends StateChanged {"Suspended"}
    // right before exiting. We must wait for full exit to avoid a race condition where
    // a subsequent resume request fails with permission denied because the old process
    // hasn't released the checkpoint file yet.
    let confirmation = match tokio::time::timeout(std::time::Duration::from_secs(SUSPEND_CONFIRM_TIMEOUT_SECS), async {
        let mut suspended = false;
        loop {
            match rx.recv().await {
                Ok(message) => {
                    if let Some(confirmation) = observe_suspend_message(message, &mut suspended) {
                        return confirmation;
                    }
                }
                Err(error) => {
                    if !suspended {
                        tracing::warn!(%error, "suspend IPC channel closed before confirmation");
                    }
                    return suspend_channel_closed(suspended);
                }
            }
        }
    })
    .await
    {
        Ok(confirmation) => confirmation,
        Err(_) => SuspendConfirmation::TimedOut,
    };

    if let Some((outcome, error)) = suspend_failure(confirmation) {
        // The guest never acknowledged suspend. Leaving the process alive
        // would leak a wedged Apple VZ instance (seen in the wild: 945
        // orphan temp dirs accumulated over one test run). SIGKILL the
        // child, reclaim the instance slot, and surface the error.
        if pid > 0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        tracing::warn!(id, outcome, "handle_suspend removing failed instance");
        state.instances.lock().unwrap().remove(&id);
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error));
    }

    // Channel closure proves the process released IPC; prove the process also
    // released its VZ checkpoint and VirtioFS share before resume can spawn a
    // replacement. The helper bounds natural exit, then SIGKILLs and waits for
    // reaping instead of guessing that a fixed post-kill sleep was sufficient.
    wait_for_process_exit(pid, std::time::Duration::from_millis(500)).await;

    tracing::warn!(id, "handle_suspend (success) removing instance");
    state.instances.lock().unwrap().remove(&id);
    state.unregister_session_db_handle(&id);
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));

    // Update persistent registry
    {
        if let Some(key) = persistent_registry_key_for_route_id(&state, &id) {
            let mut registry = state.persistent_registry.lock().unwrap();
            if let Some(entry) = registry.get_mut(&key) {
                entry.suspended = true;
                entry.checkpoint_path = Some(RESUME_CHECKPOINT_NAME.to_string());
                if let Err(e) = registry.save() {
                    error!(id, error = %e, "failed to save persistent registry");
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

pub(super) async fn handle_stop(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // shutdown_vm_process now waits for actual process exit and cleans the
    // socket inline -- when it returns, resume can immediately reuse the
    // path without a SO_REUSEADDR-style race. Graceful so persistent VMs
    // get bash history + filesystem sync before teardown.
    if let Some((session_dir, persistent, _pid)) = shutdown_vm_process(&state, &id, ShutdownMode::Retain).await? {
        if !persistent {
            let dir = session_dir;
            tokio::task::spawn_blocking(move || {
                let _ = std::fs::remove_dir_all(&dir);
            });
        }
        Ok(Json(json!({ "success": true, "persistent": persistent })))
    } else {
        Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
    }
}

pub(super) async fn handle_delete(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Delete fast-paths through direct process teardown: the session dir is
    // about to be removed, so guest sync() and bash history don't matter.
    let session_dir =
        if let Some((session_dir, _, _pid)) = shutdown_vm_process(&state, &id, ShutdownMode::Discard).await? {
            session_dir
        } else {
            // Not running -- check persistent registry for stopped VM
            if let Some(entry) = find_persistent_entry_by_route_id(&state, &id) {
                entry.session_dir
            } else {
                return Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")));
            }
        };

    // DELETE is a destructive contract. Validate registry-derived paths,
    // perform the blocking removal off the async runtime, and do not report
    // success until the directory is actually gone. Failed VM exits use the
    // separate `preserve_failed_session_dir` path; a clean delete must never
    // be relabelled as a failure.
    let state_clone = Arc::clone(&state);
    tokio::task::spawn_blocking(move || state_clone.delete_session_dir(&session_dir))
        .await
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("delete session task failed: {error}"),
            )
        })?
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("delete session state failed: {error:#}"),
            )
        })?;

    // Unregister from persistent registry only after filesystem deletion
    // succeeds. An unsafe or failed delete therefore remains discoverable
    // and can be retried after the underlying problem is repaired.
    if let Some(key) = persistent_registry_key_for_route_id(&state, &id) {
        let mut registry = state.persistent_registry.lock().unwrap();
        if registry.contains(&key) {
            registry.unregister(&key).map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("unregister deleted session failed: {error:#}"),
                )
            })?;
        }
    }

    Ok(Json(json!({ "success": true })))
}

pub(super) async fn handle_resume(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<ProvisionResponse>, AppError> {
    // See handle_suspend: same lock, same reason. Restore happens in the
    // freshly spawned capsem-process's boot, so the lock must bridge the
    // spawn and the readiness sentinel for a sibling save_state not to
    // overlap with the restoreMachineStateFromURL call.
    let _vz_guard = state.save_restore_lock.write().await;
    let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Exclusive).await?;

    let attempted_checkpoint = state.has_existing_resume_checkpoint(&id);

    match state.resume_sandbox(&id, None, None) {
        Ok(resumed_id) => {
            let uds_path = state.instance_socket_path(&resumed_id);
            if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&resumed_id)).await {
                error!(id, error = %e, "resume ready-wait failed");
                if attempted_checkpoint {
                    warn!(
                        id,
                        "warm restore failed; archiving checkpoint and retrying as a cold persistent boot"
                    );
                    stop_failed_restore_process_under_lock(&state, &resumed_id).await;
                    state.archive_failed_restore_checkpoint(&resumed_id);

                    match state.resume_sandbox(&resumed_id, None, None) {
                        Ok(cold_id) => {
                            let cold_uds_path = state.instance_socket_path(&cold_id);
                            if let Err(cold_e) =
                                wait_for_vm_ready(&cold_uds_path, 30, Some(&state), Some(&cold_id)).await
                            {
                                error!(id, "cold resume fallback failed after warm restore failure: {cold_e}");
                                return Err(AppError(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!(
                                        "resume failed: warm restore failed ({e}); cold fallback failed ({cold_e})"
                                    ),
                                ));
                            }
                            state.clear_resume_checkpoint(&cold_id);
                            return provision_response_for_running(&state, cold_id, cold_uds_path).map(Json);
                        }
                        Err(cold_e) => {
                            error!(
                                id,
                                "cold resume fallback spawn failed after warm restore failure: {cold_e}"
                            );
                            return Err(AppError(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("resume failed: warm restore failed ({e}); cold fallback failed ({cold_e})"),
                            ));
                        }
                    }
                }
                return Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("resume failed: {e}"),
                ));
            }
            state.clear_resume_checkpoint(&resumed_id);
            provision_response_for_running(&state, resumed_id, uds_path).map(Json)
        }
        Err(e) => {
            error!(id, error = %e, "resume failed");
            Err(AppError(StatusCode::NOT_FOUND, format!("resume failed: {e}")))
        }
    }
}

pub(super) fn provision_response_for_running(
    state: &ServiceState,
    id: String,
    uds_path: std::path::PathBuf,
) -> Result<ProvisionResponse, AppError> {
    let instances = state.instances.lock().unwrap();
    let instance = instances.get(&id).ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("provisioned VM missing from runtime registry: {id}"),
        )
    })?;
    let status = VmLifecycleState::Running;
    let response = ProvisionResponse {
        name: instance.name.clone(),
        id,
        profile_id: instance.profile_id.clone(),
        status,
        persistent: instance.persistent,
        can_resume: false,
        available_actions: status.available_actions(false),
        uds_path: Some(uds_path),
    };
    drop(instances);
    Ok(response)
}

pub(super) async fn handle_persist(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<PersistRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check name is not taken
    {
        let registry = state.persistent_registry.lock().unwrap();
        if registry.contains(name) {
            return Err(AppError(
                StatusCode::CONFLICT,
                format!("persistent VM \"{}\" already exists", name),
            ));
        }
    }

    // Find the running ephemeral instance
    let (
        old_session_dir,
        profile_id,
        profile_revision,
        profile_payload_hash,
        asset_pins,
        ram_mb,
        cpus,
        base_version,
        forked_from,
        env,
    ) = {
        let instances = state.instances.lock().unwrap();
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if i.persistent {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("VM \"{}\" is already persistent", id),
            ));
        }
        let result = (
            i.session_dir.clone(),
            i.profile_id.clone(),
            i.profile_revision.clone(),
            i.profile_payload_hash.clone(),
            i.asset_pins.clone(),
            i.ram_mb,
            i.cpus,
            i.base_version.clone(),
            i.forked_from.clone(),
            i.env.clone(),
        );
        drop(instances);
        result
    };
    let profile = state
        .profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    state
        .validate_profile_pins(&profile, &profile_revision, &profile_payload_hash, &asset_pins)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;

    // Move session dir to persistent location without changing the runtime id.
    let new_session_dir = state.run_dir.join("persistent").join(&id);
    let _ = std::fs::create_dir_all(state.run_dir.join("persistent"));
    std::fs::rename(&old_session_dir, &new_session_dir).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to move session dir: {e}"),
        )
    })?;

    // Register in persistent registry
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry
            .register(PersistentVmEntry {
                id: id.clone(),
                name: name.clone(),
                profile_id: profile_id.clone(),
                profile_revision: profile_revision.clone(),
                profile_payload_hash: profile_payload_hash.clone(),
                asset_pins: asset_pins.clone(),
                ram_mb,
                cpus,
                base_version,
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: new_session_dir.clone(),
                forked_from: forked_from.clone(),
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env,
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        drop(registry);
    }

    // Update instance info in-place
    {
        let mut instances = state.instances.lock().unwrap();
        if let Some(info) = instances.remove(&id) {
            state.unregister_session_db_handle(&id);
            if session_db_path_for_session_dir(&new_session_dir).exists() {
                state
                    .register_session_db_handle(&id, &new_session_dir)
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }
            instances.insert(
                id.clone(),
                InstanceInfo {
                    id: id.clone(),
                    name: name.clone(),
                    profile_id,
                    profile_revision,
                    profile_payload_hash,
                    asset_pins,
                    pid: info.pid,
                    uds_path: info.uds_path,
                    session_dir: new_session_dir,
                    ram_mb: info.ram_mb,
                    cpus: info.cpus,
                    start_time: info.start_time,
                    base_version: info.base_version,
                    persistent: true,
                    env: info.env,
                    forked_from,
                },
            );
        }
    }

    Ok(Json(json!({ "success": true, "name": name })))
}

pub(super) async fn handle_purge(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<PurgeRequest>,
) -> Result<Json<PurgeResponse>, AppError> {
    let mut ephemeral_purged: u32 = 0;
    let mut persistent_purged: u32 = 0;

    // Collect VMs to purge
    let to_purge: Vec<(String, bool)> = {
        let instances = state.instances.lock().unwrap();
        instances
            .values()
            .filter(|i| !i.persistent || payload.all)
            .map(|i| (i.id.clone(), i.persistent))
            .collect()
    };

    let results = futures::future::join_all(to_purge.iter().map(|(id, persistent)| {
        let state_ref = &state;
        let id = id.clone();
        let persistent = *persistent;
        async move {
            // Purge fast-paths for the same reason as delete: every VM
            // here is being destroyed, so the 2.5s graceful floor is pure
            // waste per VM. join_all still runs them concurrently.
            shutdown_vm_process(state_ref, &id, ShutdownMode::Discard)
                .await
                .map(|result| result.map(|(session_dir, _, _pid)| (id, session_dir, persistent)))
        }
    }))
    .await;

    for result in results {
        let Some((id, session_dir, persistent)) = result? else {
            continue;
        };
        if persistent {
            if let Some(key) = persistent_registry_key_for_route_id(&state, &id) {
                let mut registry = state.persistent_registry.lock().unwrap();
                let _ = registry.unregister(&key);
            }
        }
        let dir = session_dir;
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_dir_all(&dir);
        });
        if persistent {
            persistent_purged += 1;
        } else {
            ephemeral_purged += 1;
        }
    }

    // Default purge removes stopped defunct persistent VMs. `--all` broadens
    // that to every stopped persistent VM after CLI confirmation.
    let stopped_names: Vec<String> = {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        registry
            .list()
            .filter(|e| !instances.contains_key(&persistent_entry_vm_id(e)))
            .filter(|e| payload.all || e.defunct)
            .map(|e| e.name.clone())
            .collect()
    };
    for name in &stopped_names {
        let session_dir = {
            let registry = state.persistent_registry.lock().unwrap();
            registry.get(name).map(|e| e.session_dir.clone())
        };
        if let Some(dir) = session_dir {
            let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&dir)).await;
        }
        let mut registry = state.persistent_registry.lock().unwrap();
        let _ = registry.unregister(name);
        drop(registry);
        persistent_purged += 1;
    }

    let purged = ephemeral_purged + persistent_purged;
    Ok(Json(PurgeResponse {
        purged,
        persistent_purged,
        ephemeral_purged,
    }))
}

/// One-shot exec: provision a temp VM, run a command, return output, destroy VM.
pub(super) async fn handle_run(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<RunRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let profile_id = validate_profile_route_id(payload.profile_id.clone())?;
    if let Some(reason) = vm_asset_block_reason(&state, &profile_id) {
        return Err(AppError(StatusCode::PRECONDITION_FAILED, reason));
    }

    let id = {
        let existing = existing_session_names(&state);
        generate_profile_session_name(&profile_id, existing.iter().map(|s| s.as_str()))
    };

    let profile = state
        .profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    let resources = resolve_profile_vm_resources(&profile, payload.ram_mb, payload.cpus);
    let ram_mb = resources.ram_mb;
    let cpus = resources.cpus;
    let scratch_disk_size_gb = resources.scratch_disk_size_gb;

    // 1. Provision ephemeral VM. `provision_sandbox` is synchronous and
    // does heavy I/O (APFS clonefile, rootfs.img fsync, child spawn);
    // offload to the blocking pool, matching `handle_provision` -- the
    // tokio::process::Command::spawn inside still works because
    // spawn_blocking preserves the runtime handle via thread-locals.
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    let version = state.current_version.clone();
    let env = payload.env.clone();
    {
        let _vz_guard = state.save_restore_lock.read().await;
        let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Shared).await?;
        let provision_result = tokio::task::spawn_blocking(move || {
            state_clone.provision_sandbox(ProvisionOptions {
                id: &id_clone,
                name: &id_clone,
                profile_id,
                ram_mb,
                cpus,
                scratch_disk_size_gb,
                version_override: Some(version),
                persistent: false,
                env,
                from: None,
                description: None,
            })
        })
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("provision task: {e}")))?;
        provision_result.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("provision failed: {e}")))?;

        // 3. Wait for VM socket to appear while still holding the VZ
        // lifecycle rail. The child does its Apple VZ start/restore before it
        // writes the ready sentinel; releasing earlier reintroduces the
        // sibling-service overlap this lock exists to prevent.
        let uds_path = state.instance_socket_path(&id);
        if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id)).await {
            drop(_vz_host_guard);
            drop(_vz_guard);
            // Wait for the child to actually exit before renaming. Rename on
            // an open-for-write dir is safe (fds survive) but any path-based
            // reopens the child might do during shutdown (log rotation, db
            // reopen) would ENOENT -- so we let it finish flushing first.
            // shutdown_vm_process now blocks until exit (5s budget, SIGKILL
            // fallback) and cleans the UDS socket inline. Graceful because
            // preserve_failed_session_dir inspects session logs that capsem-process
            // is still flushing.
            let shutdown_result = shutdown_vm_process(&state, &id, ShutdownMode::Retain).await?;
            preserve_failed_run_shutdown_result(Arc::clone(&state), id.clone(), shutdown_result).await?;
            return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, e));
        }
    }

    let uds_path = state.instance_socket_path(&id);

    // 2. Execute command.
    let job_id = state.next_job_id();
    let exec_result = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec {
            id: job_id,
            command: payload.command,
        },
        payload.timeout_secs,
    )
    .await;

    // 3. Tear down VM process and build response. shutdown_vm_process
    // blocks until the process is actually gone -- the leak detector needs
    // that guarantee. Route handlers must not mine session.db before returning;
    // durable telemetry is recovered by the ledger rails.
    let shutdown_result = shutdown_vm_process(&state, &id, ShutdownMode::Retain).await?;
    let failed = !matches!(&exec_result, Ok(ProcessToService::ExecResult { .. }));
    finalize_one_shot_session(Arc::clone(&state), id.clone(), shutdown_result, failed).await?;

    let response = match exec_result {
        Ok(ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            truncated,
            ..
        }) => Ok(Json(ExecResponse {
            stdout: String::from_utf8(stdout).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            stderr: String::from_utf8(stderr).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            exit_code,
            truncated,
        })),
        Ok(_) => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response".into(),
        )),
        Err(e) => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("exec failed: {e}"))),
    };

    response
}
