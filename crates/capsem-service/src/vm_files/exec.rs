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
    actor: crate::caller_auth::AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let execution = execute(&actor, &state, &id, payload);
    let result = if let Some(grant) = actor.agent_grant() {
        tokio::select! {
            biased;
            () = grant.revoked() => Err(crate::caller_auth::execution::denied()),
            () = tokio::time::sleep_until(grant.deadline().into()) => Err(crate::caller_auth::execution::denied()),
            result = execution => result,
        }
    } else {
        execution.await
    };
    if actor.agent_grant().is_some()
        && result
            .as_ref()
            .is_err_and(|error| error.status == StatusCode::FORBIDDEN)
    {
        crate::agent_access::refusals::record_execution(&state, actor.evidence().clone()).await?;
    }
    result
}

async fn execute(
    actor: &crate::caller_auth::AuthenticatedActor,
    state: &Arc<ServiceState>,
    id: &str,
    payload: ExecRequest,
) -> Result<Json<ExecResponse>, AppError> {
    let timeout_secs =
        capsem_api::exec_timeout_secs(payload.timeout_secs).map_err(|e| AppError::new(StatusCode::BAD_REQUEST, e))?;
    let owner = actor.execution_owner(state, id)?;
    let target = capsem_api::exec_target(payload.target, crate::container_setup::runs_container(state, id).await)
        .map_err(|e| AppError::new(StatusCode::BAD_REQUEST, e))?;

    super::readiness::wait_for_owner_ready(state, &owner, 30)
        .await
        .map_err(|error| {
            if actor.agent_grant().is_some() && actor.check_execution(state, &owner).is_err() {
                crate::caller_auth::execution::denied()
            } else {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error)
            }
        })?;

    let res = ipc_command::send_owner_command_guarded(
        state,
        &owner,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command: payload.command,
            target: proto_exec_target(target),
        },
        Some(timeout_secs),
        || {
            actor
                .check_execution(state, &owner)
                .map_err(|_| ipc_command::IpcCommandError::AccessDenied)
        },
    )
    .await
    .map_err(|e| e.into_exec_app_error(false))?;

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
        _ => Err(AppError::new(
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

#[cfg(test)]
mod tests;
