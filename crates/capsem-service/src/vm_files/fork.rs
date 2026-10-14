//! Fork a running session into a new persistent one.

use super::*;

pub(crate) async fn handle_fork(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, AppError> {
    let _launch = state
        .lifecycle
        .admit()
        .map_err(|e| AppError(StatusCode::CONFLICT, e.to_string()))?;
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    validate_vm_labels(payload.labels.as_ref()).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

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

    // Find source: running instance or stopped persistent VM. A fork boots
    // the source's own images, so it inherits the source's asset pins.
    let (session_dir, asset_pins, ram_mb, cpus, base_version, source_labels, uds_path) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.asset_pins.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
                i.labels.clone(),
                Some(i.uds_path.clone()),
            )
        } else {
            drop(instances);
            let Some(p) = find_persistent_entry_by_route_id(&state, &id) else {
                return Err(AppError(
                    StatusCode::NOT_FOUND,
                    format!("source sandbox not found: {}", id),
                ));
            };
            // A VM in the old shape is refused, never laundered into a fork.
            state
                .validate_persistent_entry(&p)
                .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
            (
                p.session_dir,
                p.asset_pins,
                p.ram_mb,
                p.cpus,
                p.base_version,
                p.labels,
                None,
            )
        }
    };
    let labels = match payload.labels {
        Some(labels) => non_empty_labels(Some(labels)),
        None => non_empty_labels(source_labels),
    };

    // Clone state into new persistent sandbox. The route/runtime id is
    // separate from the human display name.
    let vm_id = new_persistent_vm_id();
    let new_session_dir = state.run_dir.join("persistent").join(&vm_id);
    let size_bytes = clone_session_state(&state, uds_path.as_deref(), session_dir, new_session_dir.clone())
        .await
        .map_err(|e| {
            capsem_service::app_error_logged!(error, StatusCode::INTERNAL_SERVER_ERROR, "fork: clone failed: {e}")
        })?;

    // Register as persistent VM; the registry saves to disk, so off the worker.
    let entry = PersistentVmEntry {
        id: vm_id.clone(),
        name: name.clone(),
        legacy_profile_id: None,
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
        labels,
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

/// Clone a sandbox's state into `destination`, an empty directory (created
/// when absent).
///
/// A running sandbox is cloned by its own process, which freezes the guest's
/// system filesystem for the copy and always thaws it: copying a live ext4
/// overlay can otherwise produce an image the fork cannot boot, and a freeze
/// held across two processes could outlive the service that asked for it. A
/// stopped sandbox has no writer and is copied here, off the async workers.
pub(crate) async fn clone_session_state(
    state: &ServiceState,
    running: Option<&std::path::Path>,
    source: PathBuf,
    destination: PathBuf,
) -> Result<u64, String> {
    tokio::fs::create_dir_all(&destination)
        .await
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    let size = clone_guest_state(state, running, source.clone(), destination.clone()).await?;
    // The clone boots the source's staged image; the service must know it
    // runs one, or exec and the files API would treat it as a bare VM.
    if let Err(error) = crate::container_setup::carry_launch_record(&source, &destination) {
        let _ = tokio::fs::remove_dir_all(&destination).await;
        return Err(error);
    }
    Ok(size)
}

async fn clone_guest_state(
    state: &ServiceState,
    running: Option<&std::path::Path>,
    source: PathBuf,
    destination: PathBuf,
) -> Result<u64, String> {
    let Some(uds_path) = running else {
        return tokio::task::spawn_blocking(move || {
            capsem_core::session::clone_sandbox_state(&source, &destination).map_err(|error| {
                let _ = std::fs::remove_dir_all(&destination);
                format!("{error:#}")
            })
        })
        .await
        .map_err(|error| format!("clone task failed: {error}"))?;
    };
    let id = state.next_job_id();
    let request = ServiceToProcess::CloneState {
        id,
        destination: destination.to_string_lossy().into_owned(),
    };
    let result = match send_ipc_command(uds_path, request, Some(CLONE_STATE_REPLY_SECS)).await {
        Ok(ProcessToService::CloneStateResult {
            size_bytes: Some(size),
            error: None,
            ..
        }) => Ok(size),
        Ok(ProcessToService::CloneStateResult { error, .. }) => {
            Err(error.unwrap_or_else(|| "the sandbox reported no clone size".into()))
        }
        Ok(other) => Err(format!("unexpected clone reply: {other:?}")),
        Err(error) => Err(error),
    };
    if result.is_err() {
        // The owner removes what it wrote; this covers an owner that never
        // answered, so no half-made fork directory outlives the request.
        let _ = tokio::fs::remove_dir_all(&destination).await;
    }
    result
}

/// The owner answers within its own 900 s clone bound; wait a little longer
/// so its answer, not this deadline, is what the caller sees.
const CLONE_STATE_REPLY_SECS: u64 = 930;
