//! Authenticated exposure control. The VM owner holds the listeners and is the
//! only registry; these routes resolve the VM, relay the request and never
//! carry a workload byte.

use super::*;
use capsem_api::{ExposureInfo, ExposureListResponse, ExposureRequest, ExposureTarget};
use capsem_proto::PublicationTarget;

/// How long the owner may take to bind a listener or answer a query.
const OWNER_TIMEOUT_SECS: u64 = 15;

fn proto_target(target: ExposureTarget) -> PublicationTarget {
    match target {
        ExposureTarget::Container => PublicationTarget::Container,
        ExposureTarget::Vm => PublicationTarget::Vm,
    }
}

fn api_target(target: PublicationTarget) -> ExposureTarget {
    match target {
        PublicationTarget::Container => ExposureTarget::Container,
        PublicationTarget::Vm => ExposureTarget::Vm,
    }
}

async fn ask_owner(state: &ServiceState, id: &str, request: ServiceToProcess) -> Result<ProcessToService, AppError> {
    let uds_path = running_uds_path(state, id)?;
    send_ipc_command(&uds_path, request, Some(OWNER_TIMEOUT_SECS))
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, format!("VM owner unavailable: {e}")))
}

fn unexpected(reply: &ProcessToService) -> AppError {
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("unexpected VM owner reply: {:?}", std::mem::discriminant(reply)),
    )
}

/// POST /vms/{id}/exposures -- listen on a host loopback port for a guest port.
pub(crate) async fn handle_create_exposure(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<ExposureRequest>,
) -> Result<Json<ExposureInfo>, AppError> {
    let target = proto_target(request.target);
    if !target.admits(request.guest_port) {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "guest port {} cannot be exposed from the {:?} namespace",
                request.guest_port, request.target
            ),
        ));
    }
    let reply = ask_owner(
        &state,
        &id,
        ServiceToProcess::PublishPort {
            id: state.next_job_id(),
            host_port: request.host_port,
            guest_port: request.guest_port,
            target,
        },
    )
    .await?;
    match reply {
        ProcessToService::PortPublished {
            host_port, error: None, ..
        } => Ok(Json(ExposureInfo::new(host_port, request.guest_port, request.target))),
        ProcessToService::PortPublished { error: Some(error), .. } => Err(AppError(StatusCode::CONFLICT, error)),
        other => Err(unexpected(&other)),
    }
}

/// GET /vms/{id}/exposures -- the VM owner's live exposures.
pub(crate) async fn handle_list_exposures(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<ExposureListResponse>, AppError> {
    match ask_owner(
        &state,
        &id,
        ServiceToProcess::ListPublications {
            id: state.next_job_id(),
        },
    )
    .await?
    {
        ProcessToService::PublicationList {
            generation,
            publications,
            ..
        } => Ok(Json(ExposureListResponse {
            owner_generation: generation.to_string(),
            exposures: publications
                .into_iter()
                .map(|p| ExposureInfo::new(p.host_port, p.guest_port, api_target(p.target)))
                .collect(),
        })),
        other => Err(unexpected(&other)),
    }
}

/// DELETE /vms/{id}/exposures/{exposure_id} -- close an exposure for good.
pub(crate) async fn handle_delete_exposure(
    State(state): State<Arc<ServiceState>>,
    Path((id, exposure_id)): Path<(String, String)>,
) -> Result<Json<api::VmActionResponse>, AppError> {
    let not_found = || AppError(StatusCode::NOT_FOUND, format!("exposure not found: {exposure_id}"));
    let host_port: u16 = exposure_id
        .parse()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(not_found)?;
    let request = ServiceToProcess::RevokePort {
        id: state.next_job_id(),
        host_port,
    };
    match ask_owner(&state, &id, request).await? {
        ProcessToService::PortRevoked { revoked: true, .. } => Ok(Json(api::VmActionResponse { success: true })),
        ProcessToService::PortRevoked { error: Some(error), .. } => {
            Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error))
        }
        ProcessToService::PortRevoked { .. } => Err(not_found()),
        other => Err(unexpected(&other)),
    }
}

#[cfg(test)]
mod tests;
