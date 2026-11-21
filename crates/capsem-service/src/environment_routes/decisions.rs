//! Dispatch deliberate decisions under the original runtime's lifecycle owners.
use super::*;
use capsem_api::requests::{RequestDecision, RequestResolveRequest};

pub(crate) fn permits_cancel(actor: &AuthenticatedActor) -> bool {
    actor
        .agent_grant()
        .is_some_and(|grant| grant.authorizes_operation(AgentOperation::CancelOwn, || LeaseClock::now().ok()))
}

pub(crate) async fn resolve(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(intent): Json<RequestResolveRequest>,
) -> Result<Json<EnvironmentMutationResponse>, AppError> {
    if actor.agent_grant().is_some() {
        if intent.decision != RequestDecision::Cancel || !permits_cancel(&actor) {
            return Err(crate::caller_auth::session::denied());
        }
    } else {
        actor.require_operator()?;
    }
    intent
        .validate()
        .map_err(|message| AppError::new(StatusCode::BAD_REQUEST, message.into()))?;
    capsem_api::validate_managed_request_id(&id)
        .map_err(|message| AppError::new(StatusCode::BAD_REQUEST, message.into()))?;
    // The operator's accepted work is independent of the HTTP waiter.
    tokio::spawn(resolve_owned(state, actor, id, intent))
        .await
        .map_err(|_| mutation_error(EnvironmentMutationError::WorkerFailed))?
        .map(Json)
}

async fn resolve_owned(
    state: Arc<ServiceState>,
    actor: AuthenticatedActor,
    id: String,
    intent: RequestResolveRequest,
) -> Result<EnvironmentMutationResponse, AppError> {
    use EnvironmentMutationError as Error;
    let approving = intent.decision == RequestDecision::Approve;
    let _admission = state.lifecycle.admit().map_err(|_| mutation_error(Error::Conflict))?;
    let native = if approving {
        Some(state.lifecycle.vz.write().await)
    } else {
        None
    };
    let mutation = if approving {
        Some(state.policy_mutation.begin().await)
    } else {
        None
    };
    let coordinator = EnvironmentMutationCoordinator::new(state.environments.clone());
    let resolved = coordinator
        .resolve_proposal(
            id.clone(),
            intent,
            actor.evidence(),
            |target| {
                actor.agent_grant().is_some_and(|grant| {
                    grant.authorizes_session(
                        AgentOperation::CancelOwn,
                        &target.session_id,
                        &target.generation,
                        || LeaseClock::now().ok(),
                    )
                })
            },
            |review| application::validate(&state, review),
        )
        .await
        .map_err(|error| match error {
            Error::NotFound if actor.agent_grant().is_some() => crate::caller_auth::session::denied(),
            Error::NotFound => AppError::new(StatusCode::NOT_FOUND, "request not found".into()),
            error => mutation_error(error),
        })?;
    let response = resolved.response;
    if let Some(approved) = resolved.approved {
        let guard = mutation.as_ref().ok_or_else(|| mutation_error(Error::WorkerFailed))?;
        let completion = application::apply(&state, approved, guard).await;
        drop(mutation);
        drop(native);
        if let Some(unconfirmed) = completion.map_err(mutation_error)? {
            // No further runtime operations may use this generation's
            // unconfirmed policy. Teardown cannot select a replacement owner.
            crate::vm_lifecycle::shutdown_vm_process(
                &state,
                unconfirmed.owner.id(),
                crate::shutdown_policy::ShutdownMode::Retain,
                Some(unconfirmed.owner.generation()),
            )
            .await?;
            unconfirmed.recorded.map_err(mutation_error)?;
        }
        return coordinator.request_status(id).await.map_err(mutation_error);
    }
    Ok(response)
}
