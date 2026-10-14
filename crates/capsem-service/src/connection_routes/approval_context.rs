//! Retained approval control resolves canonical registry identity, never a name.
use super::*;
use capsem_credentials::broker::ApprovalOwner;

pub(super) enum Target {
    Running(Box<Context>),
    Retained(ApprovalOwner),
}
impl Target {
    pub(super) fn owner(&self) -> ApprovalOwner {
        match self {
            Self::Running(context) => context.binding.owner(),
            Self::Retained(owner) => *owner,
        }
    }
}
pub(super) fn canonical(id: &str) -> Result<uuid::Uuid, CredentialError> {
    let session =
        uuid::Uuid::parse_str(id).map_err(|_| CredentialError(api::CredentialErrorCode::SessionUnavailable, false))?;
    if session.is_nil() || session.to_string() != id {
        return Err(api::CredentialErrorCode::SessionUnavailable.into());
    }
    Ok(session)
}
pub(super) async fn resolve(state: &ServiceState, id: &str) -> Result<Target, CredentialError> {
    let session = canonical(id)?;
    let running = state.instances.lock().unwrap().contains_key(id);
    if running {
        return context(state, id).await.map(Box::new).map(Target::Running);
    }
    // The immutable service-issued identity must still have a retained registry
    // entry. No directory scan, request-selected ownership or phantom runtime.
    vm_lifecycle::find_persistent_entry_by_route_id(state, id).ok_or(api::CredentialErrorCode::SessionUnavailable)?;
    Ok(Target::Retained(ApprovalOwner::new(
        *session.as_bytes(),
        *session.as_bytes(),
    )?))
}
