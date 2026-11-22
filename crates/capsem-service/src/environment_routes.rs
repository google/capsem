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
        EnvironmentChangeIntent, EnvironmentCreateRequest, EnvironmentCurrentResponse, EnvironmentMutationResponse,
        EnvironmentPolicyListQuery, EnvironmentPromotionRequest, EnvironmentRevisionReference,
        EnvironmentSpecification,
    },
    AgentOperation,
};
use capsem_core::environment::EnvironmentResolutionError;
use capsem_service::environment_revisions::{EnvironmentMutationCoordinator, EnvironmentMutationError};
mod application;
mod apply;
pub(crate) mod catalog_discovery;
pub(crate) mod decisions;
pub(crate) mod history;
pub(crate) mod library;
pub(crate) mod mcps;
pub(crate) mod plugins;
mod promotion;
pub(crate) mod proposals;
pub(crate) mod reviews;
pub(crate) mod rules;
pub(crate) mod skills;
mod sources;
pub(crate) use apply::apply;
use capsem_core::managed_sessions::LeaseClock;
use capsem_logger::db::EnvironmentReferenceQuery;
use capsem_service::environment_files::EnvironmentFileError;
pub(crate) use promotion::promote;
pub(crate) use skills::admit_boot_skills;
pub(crate) use sources::catalogs::{launch_policy, refresh_catalog_sources, CatalogStatusCache};
use std::sync::Arc;

fn coordinator(state: &ServiceState) -> EnvironmentMutationCoordinator {
    EnvironmentMutationCoordinator::new(state.environments.clone())
        .with_request_notifications(state.approvals.environment_request_notifier())
}

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
    let archived = state.environments.is_archived(&id).await;
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
        archived: archived.map_err(unavailable)?,
        timestamp_unix_ms: current.revision.timestamp_unix_ms,
        actor: current.revision.actor,
        reason: current.revision.reason,
        request_id: current.revision.request_id,
    }))
}

async fn policy_document(
    actor: &AuthenticatedActor,
    state: &Arc<ServiceState>,
    id: String,
    query: EnvironmentPolicyListQuery,
) -> Result<(EnvironmentRevisionReference, EnvironmentSpecification), AppError> {
    let Json(current) = info(actor.clone(), State(state.clone()), Path(id.clone())).await?;
    let reference = EnvironmentRevisionReference {
        environment_id: id,
        revision_id: current.document.identity.revision_id,
    };
    if let Some(expected) = query.revision_id {
        EnvironmentRevisionReference {
            environment_id: reference.environment_id.clone(),
            revision_id: expected.clone(),
        }
        .validate()
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
        if expected != reference.revision_id {
            return Err(AppError::new(
                StatusCode::CONFLICT,
                "saved environment revision changed".into(),
            ));
        }
    }
    Ok((reference, current.document.specification))
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
    let startup_timeout_seconds = snapshot.specification.preparation_timeout_seconds();
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

/// Operator changes publish a blueprint revision. Agents queue a proposal for
/// review under the authenticated running generation.
pub(crate) async fn change(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<EnvironmentChangeIntent>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    let request = match request {
        EnvironmentChangeIntent::Edit(edit) if actor.agent_grant().is_some() => {
            return proposals::submit(actor, state, id, *edit).await.map(Json);
        }
        intent => intent,
    };
    actor.require_operator()?;
    let owner = Arc::new(coordinator(&state));
    let job = match request {
        EnvironmentChangeIntent::Edit(edit) => owner.submit_change(id, *edit, sources::Sources(state)),
        EnvironmentChangeIntent::Archive(intent) => owner.submit_archive(id, intent),
        EnvironmentChangeIntent::ImageUpdate(intent) => {
            owner.submit_image_update(id, intent, move |image| sources::Sources(state).image_update(image))
        }
    }
    .map_err(mutation_error)?;
    job.wait().await.map(Json).map_err(mutation_error)
}

pub(crate) fn mutation_error(error: EnvironmentMutationError) -> AppError {
    use EnvironmentMutationError as Mutation;
    let status = match error {
        Mutation::InvalidIntent | Mutation::Resolution(EnvironmentResolutionError::InvalidIntent) => {
            StatusCode::BAD_REQUEST
        }
        Mutation::NotFound => StatusCode::NOT_FOUND,
        Mutation::Refused => StatusCode::FORBIDDEN,
        Mutation::Conflict
        | Mutation::Archived
        | Mutation::Publication(EnvironmentFileError::Changed | EnvironmentFileError::RecoveryRequired) => {
            StatusCode::CONFLICT
        }
        Mutation::Resolution(
            EnvironmentResolutionError::Refused | EnvironmentResolutionError::UnusableCatalogEntry,
        ) => StatusCode::FORBIDDEN,
        Mutation::Resolution(EnvironmentResolutionError::CatalogChanged) => StatusCode::PRECONDITION_FAILED,
        Mutation::Resolution(EnvironmentResolutionError::InvalidPin) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    // Source and durability diagnostics never expose draft values or credentials.
    AppError::new(status, error.to_string())
}

#[cfg(test)]
mod tests;
