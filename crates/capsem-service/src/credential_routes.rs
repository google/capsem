//! Explicit credentials enter the broker; HTTP responses contain references only.
use super::*;
use capsem_core::credential_broker::{CredentialProvider, CredentialStore};
use capsem_credentials::CredentialPersistence;
pub(crate) mod startup;

pub(crate) async fn inject(
    State(state): State<Arc<ServiceState>>,
    payload: Result<Json<api::CredentialInjectRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<api::CredentialInjectResponse>, AppError> {
    let Json(request) =
        payload.map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid credential injection request".into()))?;
    let storage = request.storage;
    let reference = state
        .off_worker(move |_| {
            let persistence = match storage {
                api::CredentialStorage::File => CredentialPersistence::File,
                api::CredentialStorage::Memory => CredentialPersistence::Memory,
            };
            let provider = CredentialProvider::all()
                .iter()
                .copied()
                .find(|provider| provider.as_str() == request.provider.as_str())
                .expect("shared provider spelling");
            CredentialStore::global().inject(provider, &request.value, persistence)
        })
        .await?
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error))?;
    if storage == api::CredentialStorage::Memory {
        let paths: Vec<_> = state
            .instances
            .lock()
            .unwrap()
            .values()
            .map(|instance| instance.uds_path.clone())
            .collect();
        for path in paths {
            sync_memory(&state, &path).await.map_err(|_| {
                AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "credential stored in service memory; a VM owner has not acknowledged injection; retry explicitly"
                        .into(),
                )
            })?;
        }
    }
    Ok(Json(api::CredentialInjectResponse {
        credential_ref: reference,
        storage,
    }))
}

/// Called before declaring readiness, including after resume/restart. No
/// credentials means no IPC work. File-backed material uses the existing store.
pub(crate) async fn sync_memory(state: &Arc<ServiceState>, socket: &StdPath) -> Result<(), String> {
    let credentials = CredentialStore::global().memory_credentials()?;
    if credentials.is_empty() {
        return Ok(());
    }
    let id = state.job_counter.fetch_add(1, Ordering::Relaxed);
    let command = ServiceToProcess::InjectCredentials { id, credentials };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        vm_files::send_ipc_command(socket, command, Some(5)),
    )
    .await;
    match result {
        Ok(Ok(ProcessToService::CredentialsInjected { error: None, .. })) => Ok(()),
        _ => Err("credential owner handoff unavailable".into()),
    }
}

#[cfg(test)]
mod tests;
