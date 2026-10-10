//! One serialization boundary for policy mutations and reloads.
//!
//! A policy edit is load, modify, save, active-policy materialization and VM
//! acknowledgement. Each step had its own short lock, so two edits could
//! interleave between them: B loaded before A saved and wrote A's rule away,
//! or B re-materialized a session while A was waiting for that VM's
//! acknowledgement (google/capsem#202). Every route that edits policy or
//! reloads it holds one `PolicyMutation` for the whole sequence; the functions
//! that record or push a mutation take it as an argument, so an unserialized
//! call does not compile.
use super::*;

use capsem_core::net::policy_config::{PolicyMutationSummary, SettingsFile, SettingsPolicyEdit};

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
    /// Open the user's settings.toml for an edit. Only a held mutation hands
    /// one out, so no route can read-modify-write outside the boundary.
    fn settings_edit(&self) -> Result<SettingsPolicyEdit, AppError> {
        let path = capsem_core::net::policy_config::settings_config_path()
            .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".to_string()))?;
        SettingsPolicyEdit::open(&path).map_err(bad_request)
    }
}

/// Names one route-level policy mutation in logs.
pub(crate) struct MutationRoute<'a> {
    pub(crate) name: &'static str,
    pub(crate) target_kind: &'static str,
    pub(crate) target_key: &'a str,
    pub(crate) operation: &'static str,
}

/// The one path for a route that edits the user's policy: take the boundary,
/// open settings.toml, apply `mutate` against the current corp config, record
/// the edit in the host ledger, then put it in force in every running VM
/// before the route answers. Rejections are logged with the route's identity.
pub(crate) async fn apply_policy_mutation(
    state: &Arc<ServiceState>,
    route: MutationRoute<'_>,
    mutate: impl FnOnce(SettingsPolicyEdit, &SettingsFile) -> Result<PolicyMutationSummary, String> + Send + 'static,
) -> Result<capsem_logger::PolicyMutationEvent, AppError> {
    let MutationRoute {
        name,
        target_kind,
        target_key,
        operation,
    } = route;
    info!(
        target: "capsem.policy_mutation",
        route = name,
        target_kind,
        target_key,
        operation,
        actor = "service-api",
        "policy mutation route requested"
    );
    let rejected = |error: &AppError| {
        warn!(
            target: "capsem.policy_mutation",
            route = name,
            target_kind,
            target_key,
            operation,
            actor = "service-api",
            error = error.1.as_str(),
            "policy mutation route rejected"
        )
    };
    let mutation = state.policy_mutation.begin().await;
    let edit = mutation.settings_edit().inspect_err(rejected)?;
    // settings.toml and the corp files are read and written on the blocking pool.
    let summary = state
        .off_worker(move |_| mutate(edit, &capsem_core::net::policy_config::load_corp_files()))
        .await?
        .map_err(bad_request)
        .inspect_err(rejected)?;
    state.invalidate_plugin_policy_cache();
    let event = write_policy_mutation_event(state, &mutation, summary).await?;
    info!(
        target: "capsem.policy_mutation",
        route = name,
        mutation_id = %event.mutation_id,
        target_kind = %event.target_kind,
        target_key = %event.target_key,
        operation = %event.operation,
        rule_id = event.rule_id.as_deref().unwrap_or(""),
        old_hash = %event.old_hash,
        new_hash = %event.new_hash,
        "policy mutation applied"
    );
    // The edit is on disk and in the audit ledger whatever the VMs do; say so
    // rather than let a failed push read as a failed edit.
    push_policy_to_running_instances(state, &mutation)
        .await
        .map_err(|AppError(status, error)| AppError(status, format!("edit saved and recorded; {error}")))?;
    // Held through the VM acknowledgement: that is the end of the mutation.
    drop(mutation);
    Ok(event)
}

async fn write_policy_mutation_event(
    state: &ServiceState,
    _mutation: &PolicyMutation<'_>,
    summary: PolicyMutationSummary,
) -> Result<capsem_logger::PolicyMutationEvent, AppError> {
    let mutation_id = capsem_core::security_engine::SecurityEventId::new_uuid4()
        .as_str()
        .to_string();
    let event = summary.into_logger_event(
        unix_timestamp_ms(),
        mutation_id,
        capsem_logger::PolicyMutationStatus::Applied,
        None,
        None,
    );
    state
        .host_ledger
        .write(capsem_logger::WriteOp::PolicyMutationEvent(event.clone()))
        .await
        .map_err(|error| {
            error!(
                target: "capsem.policy_mutation",
                mutation_id = %event.mutation_id,
                error = %error,
                "policy mutation ledger write failed"
            );
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("policy mutation ledger write failed: {error}"),
            )
        })?;
    Ok(event)
}

pub(crate) fn unix_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

pub(crate) fn bad_request(error: String) -> AppError {
    AppError(StatusCode::BAD_REQUEST, error)
}
