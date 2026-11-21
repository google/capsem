//! Delegate proposal authority to one authenticated, acknowledged VM owner.
use super::*;
use crate::{caller_auth::session::denied, owner_connection::OwnerConnection};
use capsem_api::environments::EnvironmentRevisionReference;
use capsem_service::environment_revisions::EnvironmentProposalOrigin;

pub(crate) fn permits_submit(actor: &AuthenticatedActor) -> bool {
    actor
        .agent_grant()
        .is_some_and(|grant| grant.authorizes_operation(AgentOperation::Propose, || LeaseClock::now().ok()))
}

pub(super) async fn submit(
    mut actor: AuthenticatedActor,
    state: Arc<ServiceState>,
    environment_id: String,
    request: EnvironmentChangeRequest,
) -> Result<EnvironmentMutationResponse, AppError> {
    let base = EnvironmentRevisionReference {
        environment_id,
        revision_id: request.base_revision_id.clone(),
    };
    base.validate()
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
    if request.requesting_sandbox.is_some() {
        return Err(mutation_error(EnvironmentMutationError::InvalidIntent));
    }
    let grant = actor.agent_grant().ok_or_else(denied)?;
    let access_id = grant.id();
    let session = {
        let instances = state.instances.lock().unwrap();
        let mut eligible = grant.scope().sessions().iter().filter(|scope| {
            instances.get(scope.session_id()).is_some_and(|instance| {
                instance.generation.to_string() == scope.owner_generation()
                    && instance.environment.as_ref().is_some_and(|binding| {
                        binding.acknowledged && binding.desired.environment_id == base.environment_id
                    })
            })
        });
        let first = eligible.next().ok_or_else(denied)?;
        if eligible.next().is_some() {
            return Err(denied());
        }
        first.session_id().to_owned()
    };
    if !actor.bind_session(&state, &session, AgentOperation::Propose) {
        return Err(denied());
    }
    let owner = actor.session_owner(&state, &session, AgentOperation::Propose)?;
    let origin = EnvironmentProposalOrigin::delegated(access_id, session, owner.generation());
    let coordinator = Arc::new(coordinator(&state));
    let job = coordinator
        .submit_proposal(
            base.environment_id,
            request,
            origin,
            sources::Sources(state.clone()),
            move |base| authorize(&actor, &state, &owner, base),
        )
        .map_err(mutation_error)?;
    job.wait().await.map_err(mutation_error)
}

fn authorize(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    owner: &OwnerConnection,
    base: &EnvironmentRevisionReference,
) -> Result<(), EnvironmentMutationError> {
    use EnvironmentMutationError::{Conflict, Refused};
    actor
        .check_session(state, owner, AgentOperation::Propose)
        .map_err(|_| Refused)?;
    let instances = state.instances.lock().unwrap();
    let instance = instances
        .get(owner.id())
        .filter(|instance| instance.generation == owner.generation())
        .ok_or(Refused)?;
    let binding = instance
        .environment
        .as_ref()
        .filter(|binding| binding.acknowledged && binding.desired.environment_id == base.environment_id)
        .ok_or(Refused)?;
    if binding.desired.revision_id != base.revision_id {
        return Err(Conflict);
    }
    Ok(())
}
