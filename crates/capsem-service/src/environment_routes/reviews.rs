//! Operator review endpoints own access to full archived request content.
use super::*;
use axum::extract::Query;
use capsem_api::requests::{
    RequestDetailResponse, RequestKind, RequestListQuery, RequestListResponse, RequestReviewAvailability, RequestState,
};

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<RequestListQuery>,
) -> Result<Json<RequestListResponse>, AppError> {
    actor.require_operator()?;
    state
        .environments
        .list_requests(query)
        .await
        .map(Json)
        .map_err(mutation_error)
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<RequestDetailResponse>, AppError> {
    actor.require_operator()?;
    capsem_api::validate_managed_request_id(&id)
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
    let mut detail = state
        .environments
        .review_request(&id)
        .await
        .map_err(|error| match error {
            EnvironmentMutationError::NotFound => AppError::new(StatusCode::NOT_FOUND, "request not found".into()),
            error => mutation_error(error),
        })?;
    if detail.summary.kind == RequestKind::EnvironmentChange
        && detail.summary.decision == RequestState::Pending
        && detail.proposed.availability == RequestReviewAvailability::Complete
    {
        if let (Some(target), Some(base), Some(proposed)) = (
            detail.summary.requester.as_ref(),
            detail.summary.base.as_ref(),
            detail.proposed.specification.as_deref(),
        ) {
            detail.restart_required = super::application::preview_restart(&state, target, base, proposed).await;
        }
    }
    Ok(Json(detail))
}
