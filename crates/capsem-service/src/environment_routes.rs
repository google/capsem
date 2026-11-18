//! Authenticated reads of canonical content; the revision owner verifies the
//! file against Logger-owned history. Caller data never supplies authority.
use crate::{caller_auth::AuthenticatedActor, AppError, ServiceState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use capsem_api::{
    environments::{
        EnvironmentChangeRequest, EnvironmentCreateRequest, EnvironmentCurrentResponse, EnvironmentMutationResponse,
    },
    AgentOperation,
};
use capsem_core::environment::EnvironmentResolutionError;
use capsem_service::environment_revisions::{EnvironmentMutationCoordinator, EnvironmentMutationError};
mod sources;
use capsem_core::managed_sessions::LeaseClock;
use capsem_logger::db::EnvironmentReferenceQuery;
use capsem_service::environment_files::EnvironmentFileError;
pub(crate) use sources::catalogs::launch_policy;
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

/// Exact historical timing is data, not launch permission. Never consult the
/// current shared file: another owner may have advanced its head since approval.
pub(crate) async fn readiness(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path((environment_id, revision_id)): Path<(String, String)>,
) -> Result<Json<capsem_api::EnvironmentReadinessResponse>, AppError> {
    authorize(&actor, &environment_id)?;
    let environment = capsem_api::environments::EnvironmentRevisionReference {
        environment_id,
        revision_id,
    };
    environment
        .validate()
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
    let observed = state
        .environments
        .load_revision(&environment.environment_id, &environment.revision_id)
        .await;
    authorize(&actor, &environment.environment_id)?;
    let snapshot = observed.map_err(unavailable)?;
    let startup_timeout_seconds = snapshot
        .specification
        .startup
        .as_ref()
        .map_or(0, |startup| startup.timeout_seconds);
    Ok(Json(capsem_api::EnvironmentReadinessResponse {
        environment,
        startup_timeout_seconds,
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
    let job = owner.submit(request, sources::Sources(state)).map_err(mutation_error)?;
    job.wait().await.map(Json).map_err(mutation_error)
}

/// Operator changes affect this blueprint's future creations. Running
/// generations and agent approval/fork authority are separate operations.
pub(crate) async fn change(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<EnvironmentChangeRequest>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    actor.require_operator()?;
    let owner = Arc::new(EnvironmentMutationCoordinator::new(state.environments.clone()));
    let job = owner
        .submit_change(id, request, sources::Sources(state))
        .map_err(mutation_error)?;
    job.wait().await.map(Json).map_err(mutation_error)
}

fn mutation_error(error: EnvironmentMutationError) -> AppError {
    use EnvironmentMutationError as Mutation;
    let status = match error {
        Mutation::InvalidIntent | Mutation::Resolution(EnvironmentResolutionError::InvalidIntent) => {
            StatusCode::BAD_REQUEST
        }
        Mutation::NotFound => StatusCode::NOT_FOUND,
        Mutation::Conflict
        | Mutation::Publication(EnvironmentFileError::Changed | EnvironmentFileError::RecoveryRequired) => {
            StatusCode::CONFLICT
        }
        Mutation::Resolution(
            EnvironmentResolutionError::Refused | EnvironmentResolutionError::UnusableCatalogEntry,
        ) => StatusCode::FORBIDDEN,
        Mutation::Resolution(EnvironmentResolutionError::InvalidPin) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    // Source and durability diagnostics never expose draft values or credentials.
    AppError::new(status, error.to_string())
}

#[cfg(test)]
mod tests;
