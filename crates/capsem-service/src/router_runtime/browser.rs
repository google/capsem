//! A successful local opener retires one request at its actual VM owner.
use super::*;
use capsem_api::{BrowserAcknowledgeRequest, BrowserAcknowledgeResponse};

fn conflict() -> AppError {
    AppError::new(StatusCode::CONFLICT, "browser request owner or revision changed".into())
}

fn owner_path(state: &ServiceState, id: &str, generation: uuid::Uuid) -> Result<PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    let instance = instances
        .get(id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "VM not found".into()))?;
    if instance.generation != generation {
        return Err(conflict());
    }
    Ok(instance.uds_path.clone())
}

fn clear(state: &ServiceState, id: &str, generation: uuid::Uuid, request_id: u64) -> Result<(), AppError> {
    let mut instances = state.instances.lock().unwrap();
    let instance = instances
        .get_mut(id)
        .filter(|instance| instance.generation == generation)
        .ok_or_else(conflict)?;
    instance.clear_browser(request_id);
    Ok(())
}

pub(super) async fn handle_acknowledge(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<BrowserAcknowledgeRequest>,
) -> Result<Json<BrowserAcknowledgeResponse>, AppError> {
    request
        .validate()
        .map_err(|message| AppError::new(StatusCode::BAD_REQUEST, message.into()))?;
    let generation = uuid::Uuid::parse_str(&request.owner_generation).map_err(|_| conflict())?;
    let path = owner_path(&state, &id, generation)?;
    let owner = owner_connection::OwnerConnection::acquire(&state, &path).map_err(|_| conflict())?;
    if owner.generation() != generation {
        return Err(conflict());
    }
    let reply = vm_files::ipc_command::send_owner_command(
        &state,
        &owner,
        ServiceToProcess::AcknowledgeBrowserRequest {
            id: state.next_job_id(),
            request_id: request.request_id,
        },
        Some(5),
    )
    .await
    .map_err(|_| {
        AppError::new(
            StatusCode::BAD_GATEWAY,
            "browser owner acknowledgement unavailable".into(),
        )
    })?;
    match reply {
        ProcessToService::BrowserRequestAcknowledged { acknowledged: true, .. } => {
            clear(&state, &id, generation, request.request_id)?;
            Ok(Json(BrowserAcknowledgeResponse {
                vm_id: id,
                owner_generation: request.owner_generation,
                request_id: request.request_id,
            }))
        }
        ProcessToService::BrowserRequestAcknowledged {
            acknowledged: false, ..
        } => Err(conflict()),
        _ => Err(AppError::new(
            StatusCode::BAD_GATEWAY,
            "unexpected browser owner acknowledgement".into(),
        )),
    }
}

#[cfg(test)]
mod tests;
