use super::*;
use capsem_credentials::GrantStopPolicy;
#[cfg(test)]
mod tests;

fn canonical(id: &str) -> Option<[u8; 16]> {
    let session = uuid::Uuid::parse_str(id).ok()?;
    (!session.is_nil() && session.to_string() == id).then_some(*session.as_bytes())
}
fn unavailable(code: api::CredentialErrorCode) -> AppError {
    warn!(?code, "credential lifecycle metadata remains pending");
    AppError(
        StatusCode::SERVICE_UNAVAILABLE,
        "credential lifecycle metadata remains pending".into(),
    )
}
impl Runtime {
    /// Used by synchronous stale-instance cleanup. The instance map must have
    /// released its lock before entering the grant admission lock.
    pub(crate) fn fence_runtime(&self, id: &str, generation: uuid::Uuid, policy: GrantStopPolicy) {
        if let Some(session) = canonical(id) {
            if let Err(error) = self.authority.detach_runtime(session, *generation.as_bytes(), policy) {
                warn!(%error, "credential runtime fence unavailable");
            }
        }
    }
    pub(crate) async fn stop_runtime(
        &self,
        id: &str,
        generation: uuid::Uuid,
        policy: GrantStopPolicy,
    ) -> Result<(), AppError> {
        let Some(session) = canonical(id) else {
            return Ok(());
        };
        self.authority
            .detach_runtime(session, *generation.as_bytes(), policy)
            .map_err(|_| unavailable(api::CredentialErrorCode::GrantUnavailable))?;
        if let Some(Ok(broker)) = self.broker.get() {
            broker
                .stop_runtime(session, *generation.as_bytes(), policy)
                .await
                .map_err(unavailable)?;
        }
        Ok(())
    }
    pub(crate) async fn retire_session(&self, id: &str) -> Result<(), AppError> {
        let Some(session) = canonical(id) else {
            return Ok(());
        };
        self.authority
            .revoke_session(session)
            .map_err(|_| unavailable(api::CredentialErrorCode::GrantUnavailable))?;
        if let Some(Ok(broker)) = self.broker.get() {
            broker.retire_session(session).await.map_err(unavailable)?;
        }
        Ok(())
    }
}

pub(crate) fn stop_policy(persistent: bool, retain: bool) -> GrantStopPolicy {
    if persistent && retain {
        GrantStopPolicy::Preserve
    } else {
        GrantStopPolicy::Revoke
    }
}

pub(crate) async fn deletion<'a>(
    state: &'a ServiceState,
    id: &str,
) -> Result<tokio::sync::RwLockWriteGuard<'a, ()>, AppError> {
    let guard = state.lifecycle.vz.write().await;
    if state.instances.lock().unwrap().contains_key(id) {
        return Err(AppError(StatusCode::CONFLICT, "session resumed during deletion".into()));
    }
    state.connections.retire_session(id).await?;
    Ok(guard)
}

/// Ready resume uses trusted current policy/ownership, without asking a client
/// to poll the grant list to bind preserved approval to its new incarnation.
pub(crate) async fn rebind(state: &Arc<ServiceState>, id: &str) -> Result<(), AppError> {
    let Some(Ok(broker)) = state.connections.broker.get() else {
        return Ok(());
    };
    let Some(session) = canonical(id) else {
        return Ok(());
    };
    if !broker.has_session_approval(session).await {
        return Ok(());
    }
    let _policy = state.policy_mutation.begin().await;
    let permitted = match enabled(state).await {
        Ok(()) => true,
        Err(CredentialError(api::CredentialErrorCode::PolicyDenied, _)) => false,
        Err(error) => return Err(unavailable(error.0)),
    };
    let context = context(state, id).await.map_err(|error| unavailable(error.0))?;
    broker
        .grants(context.binding, |_, _| permitted && context.current(state))
        .await
        .map_err(unavailable)?;
    Ok(())
}
