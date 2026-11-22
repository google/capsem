//! Explicit operator activation for selected, currently registered generations.
use super::*;
use capsem_api::environments::EnvironmentApplyRequest;
use capsem_logger::events::{environment::request::EnvironmentRequestTarget, EnvironmentRevisionParent};
use capsem_service::environment_revisions::{
    ApprovedEnvironmentReview, OperatorEnvironmentApplication, ReviewedEnvironmentChange,
};

type Error = EnvironmentMutationError;

pub(crate) async fn apply(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<EnvironmentApplyRequest>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    actor.require_operator()?;
    let job = tokio::spawn(apply_owned(state, id, request));
    job.await
        .map_err(|_| mutation_error(Error::WorkerFailed))?
        .map(Json)
        .map_err(mutation_error)
}

async fn apply_owned(
    state: Arc<ServiceState>,
    id: String,
    request: EnvironmentApplyRequest,
) -> Result<EnvironmentMutationResponse, Error> {
    let _admission = state.lifecycle.admit().map_err(|_| Error::Conflict)?;
    let native = state.lifecycle.vz.write().await;
    let mutation = state.policy_mutation.begin().await;
    let owner = coordinator(&state);
    let runtime_state = state.clone();
    let approved = owner
        .approve_application(id, request, move |specification, targets| async move {
            let mut plans = Vec::with_capacity(targets.len());
            for target in targets {
                plans.push(
                    application::validate(
                        &runtime_state,
                        ReviewedEnvironmentChange {
                            base: EnvironmentRevisionParent {
                                environment_id: target.base.environment_id,
                                revision_id: target.base.revision_id,
                            },
                            target: EnvironmentRequestTarget {
                                session_id: target.session_id,
                                generation: target.generation,
                            },
                            specification: specification.clone(),
                        },
                    )
                    .await?,
                );
            }
            Ok(plans)
        })
        .await?;
    let ApprovedEnvironmentReview {
        request_id,
        result,
        runtime,
    } = match approved {
        OperatorEnvironmentApplication::Observed(response) => return Ok(response),
        OperatorEnvironmentApplication::Approved(approved) => approved,
    };
    let mut fence = Vec::new();
    let mut error = None;
    for plan in runtime {
        let approval = ApprovedEnvironmentReview {
            request_id: request_id.clone(),
            result: result.clone(),
            runtime: plan,
        };
        match application::apply(&state, approval, &mutation).await {
            Ok(Some(unconfirmed)) => fence.push(unconfirmed),
            Ok(None) => {}
            Err(failure) => {
                error.get_or_insert(failure);
            }
        }
    }
    drop(mutation);
    drop(native);
    for unconfirmed in fence {
        if crate::vm_lifecycle::shutdown_vm_process(
            &state,
            unconfirmed.owner.id(),
            crate::shutdown_policy::ShutdownMode::Retain,
            Some(unconfirmed.owner.generation()),
        )
        .await
        .is_err()
        {
            error.get_or_insert(Error::Conflict);
        }
        if let Err(failure) = unconfirmed.recorded {
            error.get_or_insert(failure);
        }
    }
    state.approvals.environment_request_notifier()(&request_id);
    if let Some(error) = error {
        return Err(error);
    }
    owner.request_status(request_id).await
}
