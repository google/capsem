//! A delegated Execute proof retains its exact admitted owner, never a pathname.
use super::AuthenticatedActor;
use crate::{owner_connection::OwnerConnection, AppError, ServiceState};
use axum::http::StatusCode;
use capsem_core::managed_sessions::LeaseClock;
use std::sync::Arc;

impl AuthenticatedActor {
    pub(crate) fn bind_execution(&mut self, state: &ServiceState, id: &str) -> bool {
        let owner = match capture(state, id) {
            Ok(owner) => Arc::new(owner),
            Err(_) => return false,
        };
        let valid = self.agent_grant().is_some_and(|grant| {
            grant.authorizes_session(
                capsem_api::AgentOperation::Execute,
                owner.id(),
                &owner.generation().to_string(),
                || LeaseClock::now().ok(),
            )
        }) && owner.validate(state, false).is_ok();
        if valid {
            self.execution = Some(owner);
        }
        valid
    }

    pub(crate) fn execution_owner(&self, state: &ServiceState, id: &str) -> Result<Arc<OwnerConnection>, AppError> {
        if self.agent_grant().is_some() {
            let owner = self
                .execution
                .as_ref()
                .filter(|owner| owner.id() == id)
                .ok_or_else(denied)?;
            self.check_execution(state, owner)?;
            return Ok(Arc::clone(owner));
        }
        self.require_operator()?;
        capture(state, id).map(Arc::new)
    }

    pub(crate) fn check_execution(&self, state: &ServiceState, owner: &OwnerConnection) -> Result<(), AppError> {
        let Some(grant) = self.agent_grant() else {
            return self.require_operator();
        };
        let valid = self
            .execution
            .as_ref()
            .is_some_and(|bound| std::ptr::eq(bound.as_ref(), owner))
            && grant.authorizes_session(
                capsem_api::AgentOperation::Execute,
                owner.id(),
                &owner.generation().to_string(),
                || LeaseClock::now().ok(),
            )
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
        "delegated execution scope ended or does not cover this owner".into(),
    )
}
