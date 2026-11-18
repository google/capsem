//! Resume a stopped or suspended persistent session, preserving saved state
//! if its native restore fails.

use super::*;

fn invalid_request() -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid start/resume request".into())
}

pub(crate) async fn handle_resume(
    actor: crate::caller_auth::AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<ProvisionResponse>, AppError> {
    actor.require_operator()?;
    // Empty legacy requests may carry an older client's automatic content
    // type. A supplied body is always strict JSON matching data, never code.
    let request = if body.is_empty() {
        capsem_api::VmStartRequest::default()
    } else {
        // Serde structs also accept positional arrays. The HTTP contract
        // requires a JSON object, including when every field is optional.
        if !body.trim_ascii_start().starts_with(b"{") {
            return Err(invalid_request());
        }
        serde_json::from_slice::<capsem_api::VmStartRequest>(&body).map_err(|_| invalid_request())?
    };
    request
        .validate(&id)
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
    // The job owns native exclusion and launch admission. Disconnecting the
    // HTTP waiter cannot drop either while its blocking producer still runs.
    tokio::spawn(resume_owned(state, id, request)).await.map_err(|_| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "resume job ended unexpectedly".into(),
        )
    })?
}

async fn resume_owned(
    state: Arc<ServiceState>,
    id: String,
    request: capsem_api::VmStartRequest,
) -> Result<Json<ProvisionResponse>, AppError> {
    if !capsem_proto::warm_suspend_available()
        && find_persistent_entry_by_route_id(&state, &id).is_some_and(|entry| entry.suspended)
    {
        return Err(AppError::new(
            StatusCode::NOT_IMPLEMENTED,
            capsem_proto::WARM_SUSPEND_UNAVAILABLE.into(),
        ));
    }
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
        .off_worker(move |state| state.resume_sandbox(&resume_id, None, None, request.expected.as_ref()))
        .await?
    {
        Ok(resumed_id) => {
            let owner = {
                let instances = state.instances.lock().unwrap();
                let instance = instances
                    .get(&resumed_id)
                    .ok_or_else(|| AppError::new(StatusCode::CONFLICT, "resume owner ended before readiness".into()))?;
                let owner = owner_connection::OwnerConnection::capture(instance)
                    .map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
                drop(instances);
                owner
            };
            let generation = owner.generation();
            let uds_path = state
                .instance_socket_path(&resumed_id)
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&resumed_id)).await {
                error!(id, error = %e, "resume ready-wait failed");
                // A replacement boot would discard the foreground program
                // and terminal state while reporting resume success. Retire
                // this failed owner, retaining the checkpoint for retry.
                stop_failed_resume_process_under_lock(&state, &resumed_id, generation).await?;
                return Err(AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("resume failed; inspect session logs before retrying: {e}"),
                ));
            }
            if let Err(error) = crate::environment_launch::initialize(&state, &resumed_id).await {
                stop_failed_resume_process_under_lock(&state, &resumed_id, generation).await?;
                return Err(AppError::new(StatusCode::SERVICE_UNAVAILABLE, error));
            }
            let cleared_id = resumed_id.clone();
            state
                .off_worker(move |state| state.clear_resume_checkpoint(&cleared_id))
                .await?;
            connection_routes::lifecycle::rebind(&state, &resumed_id).await?;
            let preparation = crate::environment_launch::preparation_timeout(&state, &resumed_id)
                .await
                .map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
            let has_workload = crate::container_setup::prepare_restore(&state, &resumed_id, false).await?;
            // Native restoration is finished. Approved workload preparation
            // can be long and must not exclude independent cold launches.
            drop(_vz_host_guard);
            drop(_vz_guard);
            drop(_launch);
            if has_workload {
                crate::container_setup::require_owner_ready(&state, &owner, preparation).await?;
            }
            owner
                .validate(&state, false)
                .map_err(|_| AppError::new(StatusCode::CONFLICT, "resume owner ended before response".into()))?;
            provision_response_for_running(&state, resumed_id, Some(generation)).map(Json)
        }
        Err(e) => {
            let e = match e.downcast::<crate::lifecycle_readiness::SelectionRefusal>() {
                Ok(refusal) => return Err(refusal.0),
                Err(error) => error,
            };
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
pub(crate) async fn stop_failed_resume_process_under_lock(
    state: &ServiceState,
    id: &str,
    generation: uuid::Uuid,
) -> Result<(), AppError> {
    let Some((uds_path, current_generation)) = ({
        let instances = state.instances.lock().unwrap();
        instances.get(id).map(|i| (i.uds_path.clone(), i.generation))
    }) else {
        return Ok(());
    };
    if current_generation != generation {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "resume cleanup generation changed; retirement refused".into(),
        ));
    }

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
