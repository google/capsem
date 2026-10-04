//! `POST /vms/{id}/exec`: one command, in the session's workload or its VM.
//!
//! The service decides the target, because it is the one that knows whether a
//! session runs a container. The VM owner records the caller's command and the
//! target in the session ledger, applies policy to that command, and only then
//! wraps it for the workload (`capsem_core::container::workload_exec_command`),
//! so neither the ledger nor a rule ever sees the wrapper instead of what the
//! caller asked for.

use super::*;

pub(crate) async fn handle_exec(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let timeout_secs =
        capsem_api::exec_timeout_secs(payload.timeout_secs).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let uds_path = running_uds_path(&state, &id)?;
    let target = capsem_api::exec_target(
        payload.target,
        crate::container_setup::runs_container(&state, &id).await,
    )
    .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;

    wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command: payload.command,
            target: proto_exec_target(target),
        },
        Some(timeout_secs),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            truncated,
            ..
        } => Ok(Json(ExecResponse {
            stdout: ExecOutput::from_bytes(stdout),
            stderr: ExecOutput::from_bytes(stderr),
            exit_code,
            truncated,
        })),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for exec".to_string(),
        )),
    }
}

pub(crate) fn proto_exec_target(target: capsem_api::ExecTarget) -> capsem_proto::ipc::ExecTarget {
    match target {
        capsem_api::ExecTarget::Workload => capsem_proto::ipc::ExecTarget::Workload,
        capsem_api::ExecTarget::Vm => capsem_proto::ipc::ExecTarget::Vm,
    }
}
