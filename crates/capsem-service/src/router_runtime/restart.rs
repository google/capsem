//! Restart only a managed, idle service and acknowledge before draining it.

use super::*;

pub(super) async fn handle_restart(
    State(state): State<Arc<ServiceState>>,
) -> Result<(StatusCode, Json<api::RestartResponse>), AppError> {
    let _update = state
        .update_lock
        .try_lock()
        .map_err(|_| AppError(StatusCode::CONFLICT, "an update is in progress".to_string()))?;
    let manager = capsem_service::management::managed_service().await;
    accept_restart(&state, manager)
}

pub(crate) fn accept_restart(
    state: &ServiceState,
    manager: Option<api::ServiceManager>,
) -> Result<(StatusCode, Json<api::RestartResponse>), AppError> {
    let manager = manager.ok_or_else(|| {
        AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            "restart requires a launchd or systemd service configured to restart clean exits".to_string(),
        )
    })?;
    state
        .lifecycle
        .begin_restart(|| !state.instances.lock().unwrap().is_empty())
        .map_err(|error| AppError(StatusCode::CONFLICT, error.to_string()))?;
    state.update_restart.notify_one();
    Ok((
        StatusCode::ACCEPTED,
        Json(api::RestartResponse {
            status: api::RestartStatus::Accepted,
            manager,
            authentication: api::RestartAuthentication::NewTokenRequired,
        }),
    ))
}
