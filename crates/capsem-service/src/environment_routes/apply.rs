//! Explicit operator activation for selected, currently registered generations.
use super::*;
use capsem_api::environments::{
    EnvironmentApplyPreviewRequest, EnvironmentApplyPreviewResponse, EnvironmentApplyRequest, EnvironmentApplyTarget,
    EnvironmentApplyTargetPreview, EnvironmentRevisionReference,
};
use capsem_core::environment::{EnvironmentChangeImpact, EnvironmentChangeTiming};
use capsem_logger::events::{environment::request::EnvironmentRequestTarget, EnvironmentRevisionParent};
use capsem_service::environment_revisions::{
    ApprovedEnvironmentReview, OperatorEnvironmentApplication, ReviewedEnvironmentChange,
};

type Error = EnvironmentMutationError;

pub(crate) async fn preview(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<EnvironmentApplyPreviewRequest>,
) -> Result<Json<EnvironmentApplyPreviewResponse>, AppError> {
    actor.require_operator()?;
    preview_owned(&state, id, request)
        .await
        .map(Json)
        .map_err(mutation_error)
}

async fn preview_owned(
    state: &ServiceState,
    id: String,
    request: EnvironmentApplyPreviewRequest,
) -> Result<EnvironmentApplyPreviewResponse, Error> {
    request.validate().map_err(|_| Error::InvalidIntent)?;
    let base = EnvironmentRevisionReference {
        environment_id: id,
        revision_id: request.base_revision_id,
    };
    base.validate().map_err(|_| Error::InvalidIntent)?;
    let owner = coordinator(state);
    owner.check_application_base(&base).await?;
    let proposed = state
        .environments
        .load_revision(&base.environment_id, &request.revision_id)
        .await
        .map_err(Error::Publication)?;
    let snapshots = |targets: &[EnvironmentApplyTarget]| {
        let instances = state.instances.lock().unwrap();
        targets
            .iter()
            .map(|target| {
                let instance = instances.get(&target.session_id).ok_or(Error::Conflict)?;
                let base = EnvironmentRevisionParent {
                    environment_id: target.base.environment_id.clone(),
                    revision_id: target.base.revision_id.clone(),
                };
                let binding = application::reviewed_binding(instance, &target.generation, &base)
                    .filter(|_| !instance.authority.grant().is_revoked())
                    .ok_or(Error::Conflict)?;
                Ok((binding.boot.clone(), binding.live_policy.clone()))
            })
            .collect::<Result<Vec<_>, Error>>()
    };
    let reviewed = snapshots(&request.targets)?;
    let mut targets = Vec::with_capacity(reviewed.len());
    let mut impacts = std::collections::BTreeMap::new();
    for (target, (boot, live)) in request.targets.iter().zip(&reviewed) {
        for reference in [boot, live] {
            let key = (&reference.environment_id, &reference.revision_id);
            if let std::collections::btree_map::Entry::Vacant(entry) = impacts.entry(key) {
                let snapshot = state
                    .environments
                    .load_revision(key.0, key.1)
                    .await
                    .map_err(Error::Publication)?;
                entry.insert(EnvironmentChangeImpact::classify(
                    &snapshot.specification,
                    &proposed.specification,
                ));
            }
        }
        let changes = |reference: &EnvironmentRevisionParent, timing| {
            impacts[&(&reference.environment_id, &reference.revision_id)]
                .changes()
                .iter()
                .filter(|change| change.timing == timing)
                .map(|change| change.kind.setting().to_owned())
                .collect()
        };
        let as_reference = |reference: &EnvironmentRevisionParent| EnvironmentRevisionReference {
            environment_id: reference.environment_id.clone(),
            revision_id: reference.revision_id.clone(),
        };
        targets.push(EnvironmentApplyTargetPreview {
            target: target.clone(),
            boot: as_reference(boot),
            live_policy: as_reference(live),
            live_changes: changes(live, EnvironmentChangeTiming::Live),
            next_start_changes: changes(boot, EnvironmentChangeTiming::NextStart),
        });
    }
    owner.check_application_base(&base).await?;
    if snapshots(&request.targets)? != reviewed {
        return Err(Error::Conflict);
    }
    Ok(EnvironmentApplyPreviewResponse { targets })
}

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
