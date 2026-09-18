//! Authenticated exposure control. The VM owner holds the listeners and is the
//! only registry; these routes resolve the VM, relay the request and never
//! carry a workload byte.

use super::*;
use capsem_api::{
    ExposureAccess, ExposureInfo, ExposureListResponse, ExposureRequest, ExposureTarget,
    PreviewBootstrapExchangeRequest, PreviewBootstrapExchangeResponse, PreviewConnectionAdmissionRequest,
    PreviewConnectionAdmissionResponse, PreviewSessionMaterial, PreviewSessionsRevokedResponse,
};
use capsem_proto::{PublicationAccess, PublicationTarget};

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

fn api_access(access: PublicationAccess) -> ExposureAccess {
    match access {
        PublicationAccess::LoopbackTcp => ExposureAccess::LoopbackTcp,
        PublicationAccess::HttpPreview => ExposureAccess::HttpPreview,
    }
}

fn api_publication(publication: capsem_proto::ipc::PublicationInfo) -> ExposureInfo {
    ExposureInfo {
        id: publication.id,
        host_port: publication.host_port,
        guest_port: publication.guest_port,
        target: api_target(publication.target),
        access: api_access(publication.access),
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
    let owner_request = match request.access {
        ExposureAccess::LoopbackTcp => ServiceToProcess::PublishPort {
            id: state.next_job_id(),
            host_port: request.host_port,
            guest_port: request.guest_port,
            target,
        },
        ExposureAccess::HttpPreview => {
            if request.host_port != 0 {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    "http_preview does not accept host_port".into(),
                ));
            }
            let listener_port = state
                .off_worker(|state| {
                    std::fs::read_to_string(state.run_dir.join("preview.port"))
                        .ok()
                        .and_then(|value| value.trim().parse::<u16>().ok())
                        .filter(|port| *port != 0)
                })
                .await?
                .ok_or_else(|| AppError(StatusCode::SERVICE_UNAVAILABLE, "preview listener unavailable".into()))?;
            ServiceToProcess::DeclarePreview {
                id: state.next_job_id(),
                listener_port,
                guest_port: request.guest_port,
                target,
            }
        }
    };
    let reply = ask_owner(&state, &id, owner_request).await?;
    match reply {
        ProcessToService::PortPublished {
            publication: Some(publication),
            error: None,
            ..
        } => Ok(Json(api_publication(publication))),
        ProcessToService::PortPublished {
            error: Some(error),
            policy_refused: true,
            ..
        } => Err(AppError(StatusCode::FORBIDDEN, error)),
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
            exposures: publications.into_iter().map(api_publication).collect(),
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
    if exposure_id.is_empty() || exposure_id.len() > 64 {
        return Err(not_found());
    }
    let request = ServiceToProcess::RevokeExposure {
        id: state.next_job_id(),
        exposure_id: exposure_id.clone(),
    };
    match ask_owner(&state, &id, request).await? {
        ProcessToService::ExposureRevoked { revoked: true, .. } => Ok(Json(api::VmActionResponse { success: true })),
        ProcessToService::ExposureRevoked { error: Some(error), .. } => {
            Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error))
        }
        ProcessToService::ExposureRevoked { .. } => Err(not_found()),
        other => Err(unexpected(&other)),
    }
}

async fn preview_publication(
    state: &ServiceState,
    vm_id: &str,
    exposure_id: &str,
) -> Result<(u64, ExposureInfo), AppError> {
    match ask_owner(
        state,
        vm_id,
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
        } => publications
            .into_iter()
            .find(|publication| publication.id == exposure_id && publication.access == PublicationAccess::HttpPreview)
            .map(api_publication)
            .map(|exposure| (generation, exposure))
            .ok_or_else(|| {
                AppError(
                    StatusCode::NOT_FOUND,
                    format!("preview exposure not found: {exposure_id}"),
                )
            }),
        other => Err(unexpected(&other)),
    }
}

pub(crate) async fn handle_create_preview_session(
    State(state): State<Arc<ServiceState>>,
    Path((vm_id, exposure_id)): Path<(String, String)>,
) -> Result<Json<PreviewSessionMaterial>, AppError> {
    let (generation, exposure) = preview_publication(&state, &vm_id, &exposure_id).await?;
    match ask_owner(
        &state,
        &vm_id,
        ServiceToProcess::CreatePreviewSession {
            id: state.next_job_id(),
            exposure_id,
        },
    )
    .await?
    {
        ProcessToService::PreviewSessionCreated {
            bootstrap_token: Some(bootstrap_token),
            expires_in_seconds,
            error: None,
            ..
        } => Ok(Json(PreviewSessionMaterial {
            exposure,
            owner_generation: generation.to_string(),
            bootstrap_token,
            expires_in_seconds,
        })),
        ProcessToService::PreviewSessionCreated { error: Some(error), .. } => {
            Err(AppError(StatusCode::UNAUTHORIZED, error))
        }
        other => Err(unexpected(&other)),
    }
}

/// End every session of a preview exposure and the flows they admitted,
/// without deleting the exposure (google/capsem#222).
pub(crate) async fn handle_revoke_preview_sessions(
    State(state): State<Arc<ServiceState>>,
    Path((vm_id, exposure_id)): Path<(String, String)>,
) -> Result<Json<PreviewSessionsRevokedResponse>, AppError> {
    match ask_owner(
        &state,
        &vm_id,
        ServiceToProcess::RevokePreviewSessions {
            id: state.next_job_id(),
            exposure_id,
        },
    )
    .await?
    {
        ProcessToService::PreviewSessionsRevoked {
            revoked, error: None, ..
        } => Ok(Json(PreviewSessionsRevokedResponse { revoked })),
        ProcessToService::PreviewSessionsRevoked { error: Some(error), .. } => {
            Err(AppError(StatusCode::NOT_FOUND, error))
        }
        other => Err(unexpected(&other)),
    }
}

pub(crate) async fn handle_exchange_preview_bootstrap(
    State(state): State<Arc<ServiceState>>,
    Path((vm_id, exposure_id)): Path<(String, String)>,
    Json(request): Json<PreviewBootstrapExchangeRequest>,
) -> Result<Json<PreviewBootstrapExchangeResponse>, AppError> {
    match ask_owner(
        &state,
        &vm_id,
        ServiceToProcess::ExchangePreviewBootstrap {
            id: state.next_job_id(),
            exposure_id,
            bootstrap_token: request.bootstrap_token,
        },
    )
    .await?
    {
        ProcessToService::PreviewBootstrapExchanged {
            session_token: Some(session_token),
            expires_in_seconds,
            error: None,
            ..
        } => Ok(Json(PreviewBootstrapExchangeResponse {
            session_token,
            expires_in_seconds,
        })),
        ProcessToService::PreviewBootstrapExchanged { error: Some(error), .. } => {
            Err(AppError(StatusCode::UNAUTHORIZED, error))
        }
        other => Err(unexpected(&other)),
    }
}

pub(crate) async fn handle_admit_preview_connection(
    State(state): State<Arc<ServiceState>>,
    Path((vm_id, exposure_id)): Path<(String, String)>,
    Json(request): Json<PreviewConnectionAdmissionRequest>,
) -> Result<Json<PreviewConnectionAdmissionResponse>, AppError> {
    let kind = match request.kind {
        capsem_api::PreviewAdmissionKind::Request => capsem_proto::PreviewAdmissionKind::Request,
        capsem_api::PreviewAdmissionKind::WebsocketUpgrade => capsem_proto::PreviewAdmissionKind::WebsocketUpgrade,
    };
    match ask_owner(
        &state,
        &vm_id,
        ServiceToProcess::AdmitPreviewConnection {
            id: state.next_job_id(),
            exposure_id,
            session_token: request.session_token,
            kind,
        },
    )
    .await?
    {
        ProcessToService::PreviewConnectionAdmitted {
            handoff_socket,
            handoff_token,
            owner_generation,
            error: None,
            ..
        } => Ok(Json(PreviewConnectionAdmissionResponse {
            handoff_socket,
            handoff_token,
            owner_generation: owner_generation.to_string(),
        })),
        ProcessToService::PreviewConnectionAdmitted { error: Some(error), .. } => {
            Err(AppError(StatusCode::UNAUTHORIZED, error))
        }
        other => Err(unexpected(&other)),
    }
}

#[cfg(test)]
mod tests;
