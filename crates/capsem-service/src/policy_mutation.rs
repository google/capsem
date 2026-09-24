//! One serialization boundary for policy mutations and reloads.
//!
//! A policy edit is load, modify, save, cache refresh, active-profile
//! materialization and VM acknowledgement. Each step had its own short lock,
//! so two edits could interleave between them: B loaded before A saved and
//! wrote A's rule away, or B re-materialized a session while A was waiting
//! for that VM's acknowledgement (google/capsem#202). Every route that edits
//! policy or reloads it holds one `PolicyMutation` for the whole sequence;
//! the functions that record, refresh or push a mutation take it as an
//! argument, so an unserialized call does not compile.
use super::*;

#[derive(Default)]
pub(crate) struct PolicyMutationLock {
    mutex: tokio::sync::Mutex<()>,
    /// Signalled when a mutation has to wait for another, so tests can order
    /// two edits without sleeping.
    #[cfg(test)]
    pub(crate) contended: tokio::sync::Notify,
}

/// Proof that the caller holds the policy mutation boundary.
pub(crate) struct PolicyMutation<'a> {
    _guard: tokio::sync::MutexGuard<'a, ()>,
}

impl PolicyMutationLock {
    pub(crate) async fn begin(&self) -> PolicyMutation<'_> {
        let guard = match self.mutex.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                #[cfg(test)]
                self.contended.notify_one();
                self.mutex.lock().await
            }
        };
        PolicyMutation { _guard: guard }
    }
}

impl PolicyMutation<'_> {
    /// Load a profile to edit. Only a held mutation hands out an editable
    /// profile, so no route can read-modify-write outside the boundary.
    pub(crate) fn profile(&self, profile_id: String) -> Result<Profile, AppError> {
        profile_routes::profile_for_route(profile_id)
    }
}

/// Names one route-level profile mutation in logs and the audit ledger.
pub(crate) struct MutationRoute<'a> {
    pub(crate) name: &'static str,
    pub(crate) target_kind: &'static str,
    pub(crate) target_key: &'a str,
    pub(crate) operation: &'static str,
}

/// Whether a profile mutation changes what running VMs enforce.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Enforcement {
    /// Materialize and reload every running VM on the profile before answering.
    Push,
    /// Profile metadata only (skills, MCP server definitions).
    ProfileOnly,
}

/// The one path for a route that edits a profile: take the boundary, load the
/// profile, apply `mutate`, record it in the audit ledger and refresh caches,
/// then, for `Enforcement::Push`, put it in force in the profile's running VMs
/// before the route answers. Rejections are logged with the route's identity.
pub(crate) async fn apply_profile_mutation(
    state: &Arc<ServiceState>,
    route: MutationRoute<'_>,
    profile_id: String,
    enforcement: Enforcement,
    mutate: impl FnOnce(&mut Profile) -> Result<capsem_core::net::policy_config::ProfileMutationSummary, AppError>,
) -> Result<capsem_logger::ProfileMutationEvent, AppError> {
    let MutationRoute {
        name,
        target_kind,
        target_key,
        operation,
    } = route;
    log_profile_mutation_route_request(name, &profile_id, target_kind, target_key, operation);
    let rejected = |error: &AppError| {
        log_profile_mutation_route_rejected(name, &profile_id, target_kind, target_key, operation, &error.1)
    };
    let mutation = state.policy_mutation.begin().await;
    let mut profile = mutation.profile(profile_id.clone()).inspect_err(rejected)?;
    let summary = mutate(&mut profile).inspect_err(rejected)?;
    let event = write_profile_mutation_event(state, &mutation, summary, &profile).await?;
    log_profile_mutation_applied(name, &event);
    if enforcement == Enforcement::Push {
        push_profile_to_running_instances(state, &mutation, Some(profile_id.as_str())).await?;
    }
    // Held through the VM acknowledgement: that is the end of the mutation.
    drop(mutation);
    Ok(event)
}

pub(crate) fn bad_request(error: String) -> AppError {
    AppError(StatusCode::BAD_REQUEST, error)
}
