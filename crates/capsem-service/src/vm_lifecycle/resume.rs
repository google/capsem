//! Resume a stopped or suspended persistent session, preserving saved state
//! if its native restore fails.

use super::*;

pub(crate) async fn handle_resume(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let _launch = state
        .lifecycle
        .admit()
        .map_err(|e| AppError::new(StatusCode::CONFLICT, e.to_string()))?;
    // See handle_suspend: same lock, same reason. Restore happens in the
    // freshly spawned capsem-process's boot, so the lock must bridge the
    // spawn and the readiness sentinel for a sibling save_state not to
    // overlap with the restoreMachineStateFromURL call.
    let _vz_guard = state.lifecycle.vz.write().await;
    let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Exclusive).await?;

    // resume_sandbox reads the registry and policy files and spawns the
    // child: all blocking, all off the worker.
    let resume_id = id.clone();
    match state
        .off_worker(move |state| state.resume_sandbox(&resume_id, None, None))
        .await?
    {
        Ok(resumed_id) => {
            let uds_path = state
                .instance_socket_path(&resumed_id)
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&resumed_id)).await {
                error!(id, error = %e, "resume ready-wait failed");
                // A replacement boot would discard the foreground program
                // and terminal state while reporting resume success. Retire
                // this failed owner, retaining the checkpoint for retry.
                stop_failed_resume_process_under_lock(&state, &resumed_id).await?;
                return Err(AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("resume failed; inspect session logs before retrying: {e}"),
                ));
            }
            let cleared_id = resumed_id.clone();
            state
                .off_worker(move |state| state.clear_resume_checkpoint(&cleared_id))
                .await?;
            connection_routes::lifecycle::rebind(&state, &resumed_id).await?;
            crate::container_setup::restore_ready(&state, &resumed_id).await?;
            provision_response_for_running(&state, resumed_id).map(Json)
        }
        Err(e) => {
            error!(id, error = %e, "resume failed");
            let exists = find_persistent_entry_by_route_id(&state, &id).is_some();
            let status = if exists {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::NOT_FOUND
            };
            let mut err = AppError::new(status, format!("resume failed: {e}"));
            if !exists {
                err = err.with_code(ErrorCode::VmNotFound).with_vm_id(&id);
            }
            Err(err)
        }
    }
}

/// Tear down a resumed process that failed to reach ready while the
/// caller already holds the save/restore locks.
pub(crate) async fn stop_failed_resume_process_under_lock(state: &ServiceState, id: &str) -> Result<(), AppError> {
    let Some((uds_path, generation)) = ({
        let instances = state.instances.lock().unwrap();
        instances.get(id).map(|i| (i.uds_path.clone(), i.generation))
    }) else {
        return Ok(());
    };

    signal_vm_process(state, id, generation, process_control::Signal::Terminate).await?;

    tracing::warn!(id, "retiring failed resume owner; preserving saved session state");
    claim_shutdown_instance(state, id, generation);
    state.unregister_session_db_handle(id);
    wait_for_process_exit(state, id, generation, std::time::Duration::from_secs(1)).await?;
    state
        .connections
        .stop_runtime(id, generation, capsem_credentials::GrantStopPolicy::Preserve)
        .await?;
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));
    Ok(())
}

#[cfg(test)]
mod tests;
