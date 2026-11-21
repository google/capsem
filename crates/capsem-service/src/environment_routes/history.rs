//! History and edit validation require operator access to archived content.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentRevisionDetailResponse, EnvironmentRevisionListQuery, EnvironmentRevisionListResponse,
    EnvironmentRevisionReference, EnvironmentTomlValidationRequest, EnvironmentTomlValidationResponse,
};

fn bad_query(error: impl Into<String>) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, error.into())
}

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentRevisionListQuery>,
) -> Result<Json<EnvironmentRevisionListResponse>, AppError> {
    actor.require_operator()?;
    if !uuid::Uuid::parse_str(&id).is_ok_and(|value| !value.is_nil() && value.to_string() == id) {
        return Err(bad_query("canonical environment UUID required"));
    }
    let limit = query.validated_limit().map_err(bad_query)?;
    state
        .environments
        .list_history(id, query.before, limit)
        .await
        .map(Json)
        .map_err(unavailable)
}

fn exact_reference((environment_id, revision_id): (String, String)) -> Result<EnvironmentRevisionReference, AppError> {
    let reference = EnvironmentRevisionReference {
        environment_id,
        revision_id,
    };
    reference.validate().map_err(bad_query)?;
    Ok(reference)
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(path): Path<(String, String)>,
) -> Result<Json<EnvironmentRevisionDetailResponse>, AppError> {
    actor.require_operator()?;
    let reference = exact_reference(path)?;
    state
        .environments
        .revision_detail(&reference.environment_id, &reference.revision_id)
        .await
        .map_err(unavailable)?
        .map(Json)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "committed environment revision not found".into()))
}

pub(crate) async fn validate_toml(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(path): Path<(String, String)>,
    Json(request): Json<EnvironmentTomlValidationRequest>,
) -> Result<Json<EnvironmentTomlValidationResponse>, AppError> {
    actor.require_operator()?;
    let base = exact_reference(path)?;
    let current = state
        .environments
        .load_current(&base.environment_id)
        .await
        .map_err(|error| match error {
            EnvironmentFileError::Changed | EnvironmentFileError::RecoveryRequired => {
                AppError::new(StatusCode::CONFLICT, error.to_string())
            }
            _ => unavailable(error),
        })?;
    if current.revision.revision_id != base.revision_id {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "environment changed since the TOML draft was opened".into(),
        ));
    }
    let specification = tokio::task::spawn_blocking(move || current.document.draft_from_toml(&request.toml))
        .await
        .map_err(unavailable)?
        .map_err(bad_query)?;
    Ok(Json(EnvironmentTomlValidationResponse { base, specification }))
}
