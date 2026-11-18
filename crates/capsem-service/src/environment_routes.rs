//! Authenticated reads of canonical content; the revision owner verifies the
//! file against Logger-owned history. Caller data never supplies authority.
use crate::{caller_auth::AuthenticatedActor, AppError, ServiceState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use capsem_api::{
    environments::{EnvironmentCreateRequest, EnvironmentCurrentResponse, EnvironmentMutationResponse},
    AgentOperation,
};
use capsem_core::environment::EnvironmentResolutionError;
use capsem_service::environment_revisions::{EnvironmentMutationCoordinator, EnvironmentMutationError};
mod sources;
use capsem_core::managed_sessions::LeaseClock;
use capsem_logger::db::EnvironmentReferenceQuery;
use capsem_service::environment_files::EnvironmentFileError;
use std::sync::Arc;

pub(crate) fn permits_inspect(actor: &AuthenticatedActor, id: &str) -> bool {
    actor.agent_grant().is_some_and(|grant| {
        grant.scope().environment_ids().iter().any(|candidate| candidate == id)
            && grant.authorizes_operation(AgentOperation::Inspect, || LeaseClock::now().ok())
    })
}

fn authorize(actor: &AuthenticatedActor, id: &str) -> Result<(), AppError> {
    if actor.agent_grant().is_none() {
        return actor.require_operator();
    }
    if permits_inspect(actor, id) {
        return Ok(());
    }
    Err(AppError::new(
        StatusCode::FORBIDDEN,
        "environment inspection scope ended or does not cover this environment".into(),
    ))
}

fn unavailable(error: impl std::fmt::Display) -> AppError {
    tracing::error!(%error, "verified environment view unavailable");
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "verified environment view unavailable".into(),
    )
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<EnvironmentCurrentResponse>, AppError> {
    authorize(&actor, &id)?;
    if !uuid::Uuid::parse_str(&id).is_ok_and(|value| !value.is_nil() && value.to_string() == id) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "canonical environment UUID required".into(),
        ));
    }
    let references = state
        .host_ledger
        .environment_references(EnvironmentReferenceQuery::CurrentEnvironment {
            environment_id: id.clone(),
        })
        .await
        .map_err(unavailable)?;
    authorize(&actor, &id)?;
    if references.is_empty() {
        return Err(AppError::new(StatusCode::NOT_FOUND, "environment not found".into()));
    }
    let observed = state.environments.load_current(&id).await;
    // Revocation/expiry during file, archive or DB work cannot publish a view
    // under the authorization snapshot from before the await.
    authorize(&actor, &id)?;
    let current = observed.map_err(|error| match error {
        EnvironmentFileError::Changed | EnvironmentFileError::RecoveryRequired => {
            AppError::new(StatusCode::CONFLICT, error.to_string())
        }
        _ => unavailable(error),
    })?;
    Ok(Json(EnvironmentCurrentResponse {
        document: current.document,
        timestamp_unix_ms: current.revision.timestamp_unix_ms,
        actor: current.revision.actor,
        reason: current.revision.reason,
        request_id: current.revision.request_id,
    }))
}

/// Operator creation commits a new blueprint; it does not mutate running VMs.
pub(crate) async fn create(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<EnvironmentCreateRequest>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    actor.require_operator()?;
    let owner = Arc::new(EnvironmentMutationCoordinator::new(state.environments.clone()));
    let job = owner.submit(request, sources::Sources(state)).map_err(creation_error)?;
    job.wait().await.map(Json).map_err(creation_error)
}

fn creation_error(error: EnvironmentMutationError) -> AppError {
    use EnvironmentMutationError as Create;
    let status = match error {
        Create::InvalidIntent | Create::Resolution(EnvironmentResolutionError::InvalidIntent) => {
            StatusCode::BAD_REQUEST
        }
        Create::Conflict
        | Create::Publication(EnvironmentFileError::Changed | EnvironmentFileError::RecoveryRequired) => {
            StatusCode::CONFLICT
        }
        Create::Resolution(EnvironmentResolutionError::Refused | EnvironmentResolutionError::UnusableCatalogEntry) => {
            StatusCode::FORBIDDEN
        }
        Create::Resolution(EnvironmentResolutionError::InvalidPin) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    // Source and durability diagnostics never expose draft values or credentials.
    AppError::new(status, error.to_string())
}

#[cfg(test)]
mod tests;
