//! Operator review endpoints own access to full archived request content.
use super::*;
use axum::extract::Query;
use capsem_api::requests::{RequestDetailResponse, RequestListQuery, RequestListResponse};

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
    match state.environments.review_request(&id).await {
        Err(EnvironmentMutationError::NotFound) => {
            Err(AppError::new(StatusCode::NOT_FOUND, "request not found".into()))
        }
        result => result.map(Json).map_err(mutation_error),
    }
}
