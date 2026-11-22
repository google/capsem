//! Authorize an explicit operator promotion and re-admit the exact source.
use super::*;
use capsem_core::environment::EnvironmentPolicyInputs;

type Error = EnvironmentMutationError;

pub(crate) async fn promote(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<EnvironmentPromotionRequest>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    actor.require_operator()?;
    let owner = Arc::new(coordinator(&state));
    let job = owner
        .submit_promotion(id, request, move |specification| async move {
            validate_source(&state, &specification).await
        })
        .map_err(mutation_error)?;
    job.wait().await.map(Json).map_err(mutation_error)
}

async fn validate_source(state: &Arc<ServiceState>, specification: &EnvironmentSpecification) -> Result<(), Error> {
    let corporate = application::corporate_settings(state).await?;
    application::validate_workload(state, specification).await?;
    admit_boot_skills(state, &specification.skills.selected)
        .await
        .map_err(Error::Resolution)?;
    let desired = EnvironmentPolicyInputs::new(specification, specification, &corporate);
    launch_policy(state, &desired).await.map_err(Error::Resolution)?;
    if application::corporate_settings(state).await? != corporate {
        return Err(Error::Resolution(EnvironmentResolutionError::Refused));
    }
    Ok(())
}
