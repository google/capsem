//! Resume a stopped or suspended persistent session, with the cold-boot
//! fallback when a warm restore fails.

use super::*;

pub(crate) async fn handle_resume(
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

    // resume_sandbox reads the registry and profile files and spawns the
    // child: all blocking, all off the worker.
    let resume_id = id.clone();
    match state
        .off_worker(move |state| state.resume_sandbox(&resume_id, None, None))
        .await?
    {
        Ok(resumed_id) => {
            let uds_path = state
                .instance_socket_path(&resumed_id)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&resumed_id)).await {
                error!(id, error = %e, "resume ready-wait failed");
                if attempted_checkpoint {
                    warn!(
                        id,
                        "warm restore failed; archiving checkpoint and retrying as a cold persistent boot"
                    );
                    stop_failed_restore_process_under_lock(&state, &resumed_id).await;
                    let cold_id = resumed_id.clone();
                    match state
                        .off_worker(move |state| {
                            state.archive_failed_restore_checkpoint(&cold_id);
                            state.resume_sandbox(&cold_id, None, None)
                        })
                        .await?
                    {
                        Ok(cold_id) => {
                            let cold_uds_path = state
                                .instance_socket_path(&cold_id)
                                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
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
                            let cleared_id = cold_id.clone();
                            state
                                .off_worker(move |state| state.clear_resume_checkpoint(&cleared_id))
                                .await?;
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
            let cleared_id = resumed_id.clone();
            state
                .off_worker(move |state| state.clear_resume_checkpoint(&cleared_id))
                .await?;
            provision_response_for_running(&state, resumed_id, uds_path).map(Json)
        }
        Err(e) => {
            error!(id, error = %e, "resume failed");
            Err(AppError(StatusCode::NOT_FOUND, format!("resume failed: {e}")))
        }
    }
}

/// Tear down a warm-restore process that failed to reach ready while the
/// caller already holds the save/restore locks.
pub(crate) async fn stop_failed_restore_process_under_lock(state: &ServiceState, id: &str) {
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
