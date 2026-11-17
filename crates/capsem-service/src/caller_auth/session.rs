//! A delegated session proof retains its exact admitted owner, never a pathname.
use super::AuthenticatedActor;
use crate::{owner_connection::OwnerConnection, AppError, ServiceState};
use axum::http::StatusCode;
use capsem_core::managed_sessions::LeaseClock;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct SessionAdmission {
    owner: Arc<OwnerConnection>,
    operation: capsem_api::AgentOperation,
}

impl AuthenticatedActor {
    pub(crate) fn bind_session(
        &mut self,
        state: &ServiceState,
        id: &str,
        operation: capsem_api::AgentOperation,
    ) -> bool {
        self.session = None;
        let owner = match capture(state, id) {
            Ok(owner) => owner,
            Err(_) => return false,
        };
        let valid = self.agent_grant().is_some_and(|grant| {
            grant.authorizes_session(operation, owner.id(), &owner.generation().to_string(), || {
                LeaseClock::now().ok()
            })
        }) && owner.validate(state, false).is_ok();
        if valid {
            self.session = Some(SessionAdmission {
                owner: Arc::new(owner),
                operation,
            });
        }
        valid
    }

    pub(crate) fn session_owner(
        &self,
        state: &ServiceState,
        id: &str,
        operation: capsem_api::AgentOperation,
    ) -> Result<Arc<OwnerConnection>, AppError> {
        if self.agent_grant().is_some() {
            let admission = self
                .session
                .as_ref()
                .filter(|admission| admission.owner.id() == id)
                .ok_or_else(denied)?;
            self.check_session(state, &admission.owner, operation)?;
            return Ok(Arc::clone(&admission.owner));
        }
        self.require_operator()?;
        capture(state, id).map(Arc::new)
    }

    pub(crate) fn check_session(
        &self,
        state: &ServiceState,
        owner: &OwnerConnection,
        operation: capsem_api::AgentOperation,
    ) -> Result<(), AppError> {
        let Some(grant) = self.agent_grant() else {
            return self.require_operator();
        };
        let valid = self
            .session
            .as_ref()
            .is_some_and(|bound| bound.operation == operation && std::ptr::eq(bound.owner.as_ref(), owner))
            && grant.authorizes_session(operation, owner.id(), &owner.generation().to_string(), || {
                LeaseClock::now().ok()
            })
            && owner.validate(state, false).is_ok();
        if valid {
            Ok(())
        } else {
            Err(denied())
        }
    }
}

fn capture(state: &ServiceState, id: &str) -> Result<OwnerConnection, AppError> {
    let instances = state.instances.lock().unwrap();
    let instance = instances.get(id).ok_or_else(|| AppError::vm_not_found(id))?;
    let owner = OwnerConnection::capture(instance);
    drop(instances);
    owner.map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))
}

pub(crate) fn denied() -> AppError {
    AppError::new(
        StatusCode::FORBIDDEN,
        "delegated session scope ended or does not cover this operation and owner".into(),
    )
}

#[cfg(test)]
mod tests;
