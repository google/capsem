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
    let (session_dir, asset_pins, ram_mb, cpus, base_version, uds_path) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.asset_pins.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
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
            (p.session_dir, p.asset_pins, p.ram_mb, p.cpus, p.base_version, None)
        }
    };

    // Clone state into new persistent sandbox. The route/runtime id is
    // separate from the human display name.
    let vm_id = new_persistent_vm_id();
    let new_session_dir = state.run_dir.join("persistent").join(&vm_id);
    let size_bytes = clone_session_state(&state, &id, uds_path.as_deref(), session_dir, new_session_dir.clone())
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
    source_id: &str,
    running: Option<&std::path::Path>,
    source: PathBuf,
    destination: PathBuf,
) -> Result<u64, String> {
    tokio::fs::create_dir_all(&destination)
        .await
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    let size = match clone_guest_state(state, source_id, running, source.clone(), destination.clone()).await {
        Ok(size) => size,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&destination).await;
            return Err(error);
        }
    };
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
    source_id: &str,
    running: Option<&std::path::Path>,
    source: PathBuf,
    destination: PathBuf,
) -> Result<u64, String> {
    let Some(uds_path) = running else {
        return clone_coordinator_state(state, source_id, source, destination).await;
    };
    let id = state.next_job_id();
    let owner = owner_connection::OwnerConnection::acquire(state, uds_path)?;
    let (tx, rx) = owner.open(state, "capsem-service", false).await?;
    tx.send(ServiceToProcess::CloneState { id })
        .await
        .map_err(|error| format!("send clone freeze request: {error}"))?;
    match receive_clone_message(&rx, id, ClonePhase::Ready).await? {
        ProcessToService::CloneStateReady { .. } => {}
        ProcessToService::CloneStateResult { error, .. } => {
            return Err(error.unwrap_or_else(|| "the sandbox thawed before clone readiness".into()));
        }
        _ => unreachable!(),
    }
    owner.validate(state, false)?;

    let copied = clone_coordinator_state(state, source_id, source, destination.clone()).await;
    let completion = match &copied {
        Ok(size_bytes) => ServiceToProcess::CloneStateComplete {
            id,
            size_bytes: Some(*size_bytes),
            error: None,
        },
        Err(error) => ServiceToProcess::CloneStateComplete {
            id,
            size_bytes: None,
            error: Some(error.clone()),
        },
    };
    tx.send(completion)
        .await
        .map_err(|error| format!("send clone completion: {error}"))?;
    let owner_result = match receive_clone_message(&rx, id, ClonePhase::Complete).await? {
        ProcessToService::CloneStateResult {
            size_bytes: Some(size),
            error: None,
            ..
        } => Ok(size),
        ProcessToService::CloneStateResult { error, .. } => {
            Err(error.unwrap_or_else(|| "the sandbox reported no clone size".into()))
        }
        _ => unreachable!(),
    };
    owner.validate(state, false)?;
    let result = match (copied, owner_result) {
        (Err(error), _) => Err(error),
        (Ok(expected), Ok(actual)) if expected == actual => Ok(actual),
        (Ok(expected), Ok(actual)) => Err(format!(
            "the sandbox reported clone size {actual}, coordinator measured {expected}"
        )),
        (Ok(_), Err(error)) => Err(error),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&destination).await;
    }
    result
}

#[derive(Clone, Copy)]
enum ClonePhase {
    Ready,
    Complete,
}

async fn receive_clone_message(
    rx: &capsem_foundation::ipc_channel::Receiver<ProcessToService>,
    id: u64,
    phase: ClonePhase,
) -> Result<ProcessToService, String> {
    let receive = async {
        loop {
            let message = rx
                .recv()
                .await
                .map_err(|error| format!("clone owner connection closed: {error}"))?;
            let matches = match (&phase, &message) {
                (ClonePhase::Ready, ProcessToService::CloneStateReady { id: reply })
                | (ClonePhase::Ready, ProcessToService::CloneStateResult { id: reply, .. })
                | (ClonePhase::Complete, ProcessToService::CloneStateResult { id: reply, .. }) => *reply == id,
                _ => false,
            };
            if matches {
                return Ok(message);
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(CLONE_STATE_REPLY_SECS), receive)
        .await
        .map_err(|_| format!("clone owner timed out after {CLONE_STATE_REPLY_SECS}s"))?
}

async fn clone_coordinator_state(
    state: &ServiceState,
    source_id: &str,
    source: PathBuf,
    destination: PathBuf,
) -> Result<u64, String> {
    let files_source = source.clone();
    let files_destination = destination.clone();
    tokio::task::spawn_blocking(move || {
        capsem_core::session::clone_sandbox_files(&files_source, &files_destination)
            .map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| format!("clone task failed: {error}"))??;

    match tokio::fs::symlink_metadata(source.join("session.db")).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect source ledger: {error}")),
        Ok(_) => clone_ledger(state, source_id, &source, &destination).await?,
    }
    Ok(capsem_core::session::disk_usage_bytes(&destination))
}

async fn clone_ledger(
    state: &ServiceState,
    source_id: &str,
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    let database = source.join("session.db");
    let channel = state
        .ledger_workers
        .acquire(
            source_id,
            &database,
            &source.join("ledger.log"),
            capsem_proto::ledger::LedgerClientRole::Maintainer,
        )
        .await
        .map_err(|error| format!("acquire source ledger owner: {error:#}"))?;
    let (stream, grant) = channel.into_parts();
    let client = capsem_logger::ledger_client::LedgerClient::connect(stream, grant, database).await?;
    let snapshot = client.snapshot(*uuid::Uuid::new_v4().as_bytes()).await?;
    let snapshot_copy = snapshot.clone();
    let destination = destination.to_path_buf();
    let copied = tokio::task::spawn_blocking(move || {
        let source = capsem_foundation::unix::contained::ContainedDir::open_root(&snapshot_copy)
            .map_err(|error| format!("open staged ledger snapshot: {error}"))?;
        let destination = capsem_foundation::unix::contained::ContainedDir::open_root(&destination)
            .map_err(|error| format!("open clone destination: {error}"))?;
        capsem_foundation::unix::tree_clone::clone_tree(&source, &destination)
            .map_err(|error| format!("copy staged ledger snapshot: {error}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| format!("ledger copy task failed: {error}"))?;
    let removed = tokio::fs::remove_dir_all(&snapshot)
        .await
        .map_err(|error| format!("remove staged ledger snapshot {}: {error}", snapshot.display()));
    copied?;
    removed
}

/// The owner answers within its own 900 s clone bound; wait a little longer
/// so its answer, not this deadline, is what the caller sees.
const CLONE_STATE_REPLY_SECS: u64 = 930;
