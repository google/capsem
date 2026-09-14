//! Fork a running session into a new persistent one.

use super::*;

pub(crate) async fn handle_fork(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, AppError> {
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check name is not taken
    {
        let registry = state.persistent_registry.lock().unwrap();
        if registry.contains(name) {
            return Err(AppError(
                StatusCode::CONFLICT,
                format!("sandbox '{}' already exists", name),
            ));
        }
    }

    // Find source: running instance or stopped persistent VM
    let (
        session_dir,
        profile_id,
        profile_revision,
        profile_payload_hash,
        asset_pins,
        ram_mb,
        cpus,
        base_version,
        uds_path,
    ) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.profile_id.clone(),
                i.profile_revision.clone(),
                i.profile_payload_hash.clone(),
                i.asset_pins.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
                Some(i.uds_path.clone()),
            )
        } else {
            drop(instances);
            if let Some(p) = find_persistent_entry_by_route_id(&state, &id) {
                (
                    p.session_dir,
                    p.profile_id,
                    p.profile_revision,
                    p.profile_payload_hash,
                    p.asset_pins,
                    p.ram_mb,
                    p.cpus,
                    p.base_version,
                    None,
                )
            } else {
                return Err(AppError(
                    StatusCode::NOT_FOUND,
                    format!("source sandbox not found: {}", id),
                ));
            }
        }
    };
    let profile = state
        .cached_profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    state
        .validate_profile_pins(&profile, &profile_revision, &profile_payload_hash, &asset_pins)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;

    // Flush the guest root filesystem so the ext4 system overlay (/dev/vdb
    // backed by rootfs.img) has pushed dirty pages into the host-visible image
    // before fork clone. Do not fsfreeze here: the old shell command thawed
    // before cloning, so it paid freeze latency without actually snapshotting
    // while frozen.
    if let Some(ref uds) = uds_path {
        let flush_id = state.next_job_id();
        if let Err(e) = send_ipc_command(
            uds,
            ServiceToProcess::Exec {
                id: flush_id,
                command: "sync; true".to_string(),
            },
            Some(10),
        )
        .await
        {
            tracing::warn!(error = %e, "pre-fork guest sync failed (non-fatal)");
        }
    }

    // Clone state into new persistent sandbox. The route/runtime id is
    // separate from the human display name.
    let vm_id = new_persistent_vm_id();
    let new_session_dir = state.run_dir.join("persistent").join(&vm_id);

    // clone_sandbox_state does fsync + APFS clonefile + walkdir -- all blocking.
    // Offload to the blocking pool so axum worker threads aren't starved under
    // concurrent fork load.
    let clone_dst = new_session_dir.clone();
    let size_bytes = tokio::task::spawn_blocking(move || {
        let _ = std::fs::create_dir_all(&clone_dst);
        capsem_core::auto_snapshot::clone_sandbox_state(&session_dir, &clone_dst)
    })
    .await
    .map_err(|e| {
        capsem_service::app_error_logged!(error, StatusCode::INTERNAL_SERVER_ERROR, "fork: clone-task panic: {e}")
    })?
    .map_err(|e| {
        capsem_service::app_error_logged!(error, StatusCode::INTERNAL_SERVER_ERROR, "fork: clone failed: {e}")
    })?;

    // Register as persistent VM; the registry saves to disk, so off the worker.
    let entry = PersistentVmEntry {
        id: vm_id.clone(),
        name: name.clone(),
        profile_id,
        profile_revision,
        profile_payload_hash,
        asset_pins,
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
        session_dir: new_session_dir,
        forked_from: Some(id.clone()),
        description: payload.description.clone(),
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    };
    state
        .off_worker(move |state| state.persistent_registry.lock().unwrap().register(entry))
        .await?
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ForkResponse {
        id: vm_id,
        name: name.clone(),
        size_bytes,
    }))
}
