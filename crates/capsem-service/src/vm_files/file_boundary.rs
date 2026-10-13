//! Shared file import/export admission; host staging needs only a launched owner.

use super::*;

pub(crate) async fn log_file_boundary(
    state: &Arc<ServiceState>,
    sandbox_id: &str,
    action: FileBoundaryAction,
    path: String,
    data_preview: Vec<u8>,
    size: u64,
    mime_type: Option<String>,
) -> Result<Option<Vec<u8>>, AppError> {
    let uds_path = active_instance_uds_path(state, sandbox_id)?;
    wait_for_vm_ready(&uds_path, 30, Some(state), Some(sandbox_id))
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    log_file_boundary_on_owner(state, &uds_path, action, path, data_preview, size, mime_type).await
}

/// The setup task has already observed the owner's launch signal. Admission
/// still runs on that owner before any bytes are written, even before the
/// guest handshake. Public file routes retain their guest-readiness barrier.
pub(crate) async fn log_file_boundary_on_owner(
    state: &Arc<ServiceState>,
    uds_path: &StdPath,
    action: FileBoundaryAction,
    path: String,
    data_preview: Vec<u8>,
    size: u64,
    mime_type: Option<String>,
) -> Result<Option<Vec<u8>>, AppError> {
    let id = state.next_job_id();
    let res = send_ipc_command(
        uds_path,
        ServiceToProcess::LogFileBoundary {
            id,
            action,
            path,
            data: data_preview,
            size,
            mime_type,
        },
        Some(5),
    )
    .await
    .map_err(ipc_command::IpcCommandError::into_internal_app_error)?;

    match res {
        ProcessToService::LogFileBoundaryResult {
            success: true, data, ..
        } => Ok(data),
        ProcessToService::LogFileBoundaryResult { error, .. } => Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            error.unwrap_or_else(|| "failed to log file boundary".into()),
        )),
        _ => Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for file boundary log".into(),
        )),
    }
}
