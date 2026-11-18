//! Verified wait metadata for a particular lifecycle operation.
use crate::{caller_auth::AuthenticatedActor, vm_lifecycle::persistent_entry_vm_id, AppError, ServiceState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use capsem_api::{
    environments::EnvironmentRevisionReference, VmLifecycleIntent, VmLifecycleSelection, VmReadinessResponse,
};
use capsem_foundation::unix::contained::ContainedDir;
use capsem_logger::events::EnvironmentRevisionParent;
use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadinessQuery {
    intent: VmLifecycleIntent,
}

fn reference(value: &EnvironmentRevisionParent) -> EnvironmentRevisionReference {
    EnvironmentRevisionReference {
        environment_id: value.environment_id.clone(),
        revision_id: value.revision_id.clone(),
    }
}

fn unavailable(error: impl std::fmt::Display) -> AppError {
    tracing::warn!(%error, "lifecycle readiness selection unavailable");
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "verified lifecycle readiness unavailable".into(),
    )
}

fn changed() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "lifecycle selection changed; request a new readiness view".into(),
    )
}

/// Retain a typed HTTP refusal through a blocking producer's anyhow boundary.
#[derive(Debug)]
pub(crate) struct SelectionRefusal(pub(crate) AppError);
impl std::fmt::Display for SelectionRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.body.error.fmt(formatter)
    }
}
impl std::error::Error for SelectionRefusal {}

impl ServiceState {
    /// Blocking, read-only observation. A stopped sidecar identifies the last
    /// spawn; it is never proof of a live process or permission to signal one.
    pub(crate) fn lifecycle_selection(
        &self,
        id: &str,
        intent: VmLifecycleIntent,
    ) -> Result<VmLifecycleSelection, AppError> {
        if intent != VmLifecycleIntent::Clone {
            let instances = self.instances.lock().unwrap();
            if let Some(instance) = instances.get(id) {
                let environment = instance.environment.as_ref().map(|binding| {
                    reference(match intent {
                        VmLifecycleIntent::Start => &binding.boot,
                        VmLifecycleIntent::Fork => &binding.desired,
                        VmLifecycleIntent::Clone => unreachable!(),
                    })
                });
                let selection = VmLifecycleSelection {
                    id: instance.id.clone(),
                    generation: Some(instance.generation.to_string()),
                    running: true,
                    environment,
                };
                selection.validate().map_err(unavailable)?;
                return Ok(selection);
            }
        }
        let entry = {
            let registry = self.persistent_registry.lock().unwrap();
            let entry = registry
                .list()
                .find(|entry| {
                    persistent_entry_vm_id(entry) == id || (intent == VmLifecycleIntent::Clone && entry.name == id)
                })
                .cloned();
            entry
        }
        .ok_or_else(|| AppError::vm_not_found(id))?;
        let resolved_id = persistent_entry_vm_id(&entry);
        let active_generation = self
            .instances
            .lock()
            .unwrap()
            .get(&resolved_id)
            .map(|instance| instance.generation);
        let running = active_generation.is_some();
        let generation = match active_generation {
            Some(generation) => Some(generation.to_string()),
            None => {
                let session = ContainedDir::open_root(&entry.session_dir).map_err(unavailable)?;
                let identity = capsem_core::session::read_spawn_identity(&session).map_err(unavailable)?;
                if identity.as_ref().is_some_and(|identity| identity.id() != resolved_id) {
                    return Err(unavailable("durable spawn identity targets another VM"));
                }
                identity.map(|identity| identity.generation().to_string())
            }
        };
        let selection = VmLifecycleSelection {
            id: resolved_id,
            generation,
            running,
            environment: entry.environment.as_ref().map(reference),
        };
        selection.validate().map_err(unavailable)?;
        Ok(selection)
    }

    /// Comparison is matching data only. A mutation owner must retain its
    /// lifecycle/policy boundaries through consumption of the selected binding.
    pub(crate) fn verify_lifecycle_selection(
        &self,
        id: &str,
        intent: VmLifecycleIntent,
        expected: &VmLifecycleSelection,
    ) -> Result<(), AppError> {
        expected
            .validate()
            .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
        let actual = self.lifecycle_selection(id, intent).map_err(|error| {
            if error.status == StatusCode::NOT_FOUND {
                changed()
            } else {
                error
            }
        })?;
        if actual != *expected {
            return Err(changed());
        }
        Ok(())
    }
}

pub(crate) async fn handle(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadinessQuery>,
) -> Result<Json<VmReadinessResponse>, AppError> {
    actor.require_operator()?;
    let capture_id = id.clone();
    let selection = state
        .off_worker(move |state| state.lifecycle_selection(&capture_id, query.intent))
        .await??;
    finish_view(&state, id, query.intent, selection).await.map(Json)
}

async fn finish_view(
    state: &Arc<ServiceState>,
    id: String,
    intent: VmLifecycleIntent,
    selection: VmLifecycleSelection,
) -> Result<VmReadinessResponse, AppError> {
    let startup_timeout_seconds = match &selection.environment {
        Some(environment) => {
            let revision = state
                .environments
                .load_revision(&environment.environment_id, &environment.revision_id)
                .await
                .map_err(unavailable)?;
            revision
                .specification
                .startup
                .as_ref()
                .map_or(0, |startup| startup.timeout_seconds)
        }
        None => 0,
    };
    // History reads and archive/file verification can yield. Never publish a
    // budget after the original owner or its selected launch revision changes.
    let expected = selection.clone();
    state
        .off_worker(move |state| state.verify_lifecycle_selection(&id, intent, &expected))
        .await??;
    Ok(VmReadinessResponse {
        selection,
        startup_timeout_seconds,
    })
}

#[cfg(test)]
mod tests;
