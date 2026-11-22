//! Apply a committed fork to its original owner and retain exact startup intent.
use super::*;
use crate::{
    active_policy,
    environment_launch::{validate_preparation, EnvironmentBinding},
    owner_connection::OwnerConnection,
    policy_mutation::{unix_timestamp_ms, PolicyMutation},
    vm_lifecycle::{persistent_entry_vm_id, persistent_registry_key_for_route_id},
};
use capsem_api::environments::{EnvironmentRevisionReference, EnvironmentSandboxTarget};
use capsem_config::environment::EnvironmentSpecification;
use capsem_core::{
    environment::{EnvironmentChangeImpact, EnvironmentPolicyInputs},
    net::policy_config::{load_corp_policy_files, ActivePolicyFile},
};
use capsem_logger::{
    events::environment::{
        application::{
            EnvironmentApplicationComposition, EnvironmentApplicationMutation, EnvironmentApplicationState,
            EnvironmentSandboxReferences,
        },
        request::EnvironmentRequestTarget,
    },
    events::EnvironmentRevisionParent,
    WriteOp,
};
use capsem_service::environment_revisions::{ApprovedEnvironmentReview, ReviewedEnvironmentChange};
use std::path::PathBuf;

type Error = EnvironmentMutationError;
type Application = EnvironmentApplicationState;

pub(super) struct Plan {
    owner: OwnerConnection,
    target: EnvironmentRequestTarget,
    binding: EnvironmentBinding,
    session_dir: PathBuf,
    persistent: bool,
    policy: ActivePolicyFile,
    corporate_policy_id: String,
    pending_restart: bool,
}

pub(super) struct UnconfirmedPolicy {
    pub owner: OwnerConnection,
    pub recorded: Result<(), Error>,
}

fn reviewed_binding<'a>(
    instance: &'a crate::InstanceInfo,
    generation: &str,
    base: &EnvironmentRevisionParent,
) -> Option<&'a EnvironmentBinding> {
    instance.environment.as_ref().filter(|binding| {
        instance.generation.to_string() == generation && binding.acknowledged && &binding.desired == base
    })
}

pub(super) async fn preview_restart(
    state: &ServiceState,
    target: &EnvironmentSandboxTarget,
    base: &EnvironmentRevisionReference,
    proposed: &EnvironmentSpecification,
) -> Option<bool> {
    let base = EnvironmentRevisionParent {
        environment_id: base.environment_id.clone(),
        revision_id: base.revision_id.clone(),
    };
    let current_boot = || {
        let instances = state.instances.lock().unwrap();
        let instance = instances.get(&target.session_id)?;
        let binding = reviewed_binding(instance, &target.generation, &base)?;
        (!instance.authority.grant().is_revoked()).then(|| binding.boot.clone())
    };
    let boot = current_boot()?;
    let original = state
        .environments
        .load_revision(&boot.environment_id, &boot.revision_id)
        .await
        .ok()?;
    (current_boot().as_ref() == Some(&boot))
        .then(|| EnvironmentChangeImpact::classify(&original.specification, proposed).has_deferred_changes())
}

pub(super) async fn validate(state: &Arc<ServiceState>, review: ReviewedEnvironmentChange) -> Result<Plan, Error> {
    let (owner, binding, session_dir, persistent) = {
        let instances = state.instances.lock().unwrap();
        let instance = instances.get(&review.target.session_id).ok_or(Error::Conflict)?;
        let binding = reviewed_binding(instance, &review.target.generation, &review.base).ok_or(Error::Conflict)?;
        let owner = OwnerConnection::capture(instance).map_err(|_| Error::Conflict)?;
        (
            owner,
            binding.clone(),
            instance.session_dir.clone(),
            instance.persistent,
        )
    };
    owner.validate(state, false).map_err(|_| Error::Conflict)?;
    validate_preparation(&review.specification).map_err(|_| Error::Resolution(EnvironmentResolutionError::Refused))?;
    let original = state
        .environments
        .load_revision(&binding.boot.environment_id, &binding.boot.revision_id)
        .await
        .map_err(Error::Publication)?;
    let corporate = corporate_settings(state).await?;
    if let Some(image) = &review.specification.image {
        use crate::container_setup::images::{Decision, ImageError};
        let refused = |error| {
            Error::Resolution(match error {
                ImageError::Refused(_) => EnvironmentResolutionError::Refused,
                _ => EnvironmentResolutionError::Unavailable,
            })
        };
        let mut admission = Decision::new(state).await.map_err(refused)?;
        let selected = admission.resolve_saved(image).await.map_err(refused)?;
        admission.admit_saved(&selected).await.map_err(refused)?;
    }
    let pending_restart =
        EnvironmentChangeImpact::classify(&original.specification, &review.specification).has_deferred_changes();
    let desired = EnvironmentPolicyInputs::new(&review.specification, &review.specification, &corporate);
    let future_policy = launch_policy(state, &desired).await.map_err(Error::Resolution)?;
    let policy = if pending_restart {
        let live = EnvironmentPolicyInputs::new(&original.specification, &review.specification, &corporate);
        launch_policy(state, &live).await.map_err(Error::Resolution)?
    } else {
        future_policy
    };
    if corporate_settings(state).await? != corporate {
        return Err(Error::Resolution(EnvironmentResolutionError::Refused));
    }
    owner.validate(state, false).map_err(|_| Error::Conflict)?;
    let canonical = serde_json::to_value(&corporate).map_err(|_| Error::WorkerFailed)?;
    let bytes = serde_json::to_vec(&canonical).map_err(|_| Error::WorkerFailed)?;
    let corporate_policy_id = format!("blake3:{}", capsem_crypto::blake3::hash(&bytes).to_hex());
    Ok(Plan {
        owner,
        target: review.target,
        binding,
        session_dir,
        persistent,
        policy,
        corporate_policy_id,
        pending_restart,
    })
}

async fn corporate_settings(state: &Arc<ServiceState>) -> Result<capsem_config::SettingsFile, Error> {
    state
        .off_worker(|_| load_corp_policy_files())
        .await
        .map_err(|_| Error::WorkerFailed)?
        .map_err(|_| Error::Resolution(EnvironmentResolutionError::Unavailable))
}

pub(super) async fn apply(
    state: &Arc<ServiceState>,
    approved: ApprovedEnvironmentReview<Plan>,
    mutation: &PolicyMutation<'_>,
) -> Result<Option<UnconfirmedPolicy>, Error> {
    let ApprovedEnvironmentReview {
        request_id,
        result,
        runtime: plan,
    } = approved;
    let mut record = EnvironmentApplicationMutation {
        request_id,
        target: plan.target.clone(),
        expected_version: None,
        expected_references: EnvironmentSandboxReferences {
            boot: Some(plan.binding.boot.clone()),
            live_policy: Some(plan.binding.live_policy.clone()),
            desired: Some(plan.binding.desired.clone()),
        },
        timestamp_unix_ms: unix_timestamp_ms(),
        actor: "operator".into(),
        state: Application::Pending,
        composition: EnvironmentApplicationComposition {
            boot: plan.binding.boot.clone(),
            live_policy: plan.binding.live_policy.clone(),
            desired: result.clone(),
            corporate_policy_id: plan.corporate_policy_id.clone(),
            pending_restart: plan.pending_restart,
        },
        failure_reason: None,
    };
    write(state, record.clone()).await?;
    record.expected_references.desired = Some(result.clone());
    record.expected_version = Some(1);
    record.state = Application::Applying;
    record.timestamp_unix_ms = unix_timestamp_ms();
    write(state, record.clone()).await?;
    let outcome = apply_runtime(state, &plan, &result, mutation).await;
    record.expected_version = Some(2);
    record.timestamp_unix_ms = unix_timestamp_ms();
    match outcome {
        Ok(()) => {
            record.state = Application::Applied;
            record.composition.live_policy = result;
            write(state, record).await?;
            Ok(None)
        }
        Err(failure) => {
            record.state = Application::Failed;
            record.failure_reason = Some(failure.reason.into());
            let recorded = write(state, record).await;
            if failure.fence {
                Ok(Some(UnconfirmedPolicy {
                    owner: plan.owner,
                    recorded,
                }))
            } else {
                recorded.map(|_| None)
            }
        }
    }
}

struct ApplyFailure {
    reason: &'static str,
    fence: bool,
}

async fn apply_runtime(
    state: &Arc<ServiceState>,
    plan: &Plan,
    desired: &EnvironmentRevisionParent,
    mutation: &PolicyMutation<'_>,
) -> Result<(), ApplyFailure> {
    let owner_ended = ApplyFailure {
        reason: "original environment owner ended before application",
        fence: false,
    };
    plan.owner.validate(state, false).map_err(|_| owner_ended)?;
    // A durable selection remains the desired future configuration even when
    // a subsequent policy receiver refuses the live update.
    if plan.persistent {
        let id = plan.owner.id().to_owned();
        let expected = plan.binding.desired.clone();
        let desired = desired.clone();
        let saved = state
            .off_worker(move |state| save_selection(&state, &id, &expected, &desired))
            .await;
        if !matches!(saved, Ok(Ok(()))) {
            return Err(ApplyFailure {
                reason: "persistent environment selection could not be saved",
                fence: false,
            });
        }
    }
    {
        let mut instances = state.instances.lock().unwrap();
        let binding = instances
            .get_mut(plan.owner.id())
            .filter(|instance| instance.generation == plan.owner.generation())
            .and_then(|instance| instance.environment.as_mut())
            .ok_or(ApplyFailure {
                reason: "original environment owner ended before publication",
                fence: false,
            })?;
        binding.desired = desired.clone();
        binding.pending_restart = plan.pending_restart;
    }
    let session_dir = plan.session_dir.clone();
    let policy = plan.policy.clone();
    let published = state
        .off_worker(move |_| active_policy::publish_active_policy(&session_dir, &policy))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(ApplyFailure {
            reason: "approved active policy could not be published",
            fence: true,
        })?;
    if published.digest != plan.binding.policy_digest {
        active_policy::apply_to_owner(state, &plan.owner, &published, mutation)
            .await
            .map_err(|error| {
                tracing::warn!(%error, request_target = %plan.target.session_id, "environment policy not acknowledged");
                ApplyFailure {
                    reason: "original owner did not acknowledge the approved active policy",
                    fence: true,
                }
            })?;
    }
    plan.owner.validate(state, false).map_err(|_| ApplyFailure {
        reason: "original environment owner ended after publication",
        fence: true,
    })?;
    let mut instances = state.instances.lock().unwrap();
    let binding = instances
        .get_mut(plan.owner.id())
        .filter(|instance| instance.generation == plan.owner.generation())
        .and_then(|instance| instance.environment.as_mut())
        .ok_or(ApplyFailure {
            reason: "original environment owner ended after publication",
            fence: true,
        })?;
    binding.live_policy = desired.clone();
    Ok(())
}

fn save_selection(
    state: &ServiceState,
    id: &str,
    expected: &EnvironmentRevisionParent,
    desired: &EnvironmentRevisionParent,
) -> anyhow::Result<()> {
    let key = persistent_registry_key_for_route_id(state, id).ok_or_else(|| anyhow::anyhow!("persistent VM ended"))?;
    let mut registry = state.persistent_registry.lock().unwrap();
    let entry = registry
        .get_mut(&key)
        .filter(|entry| persistent_entry_vm_id(entry) == id && entry.environment.as_ref() == Some(expected))
        .ok_or_else(|| anyhow::anyhow!("persistent VM selection changed"))?;
    entry.environment = Some(desired.clone());
    if let Err(error) = registry.save() {
        let mut registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry
            .get_mut(&key)
            .filter(|entry| persistent_entry_vm_id(entry) == id && entry.environment.as_ref() == Some(desired))
        {
            entry.environment = Some(expected.clone());
        }
        return Err(error);
    }
    Ok(())
}

async fn write(state: &ServiceState, record: EnvironmentApplicationMutation) -> Result<(), Error> {
    state
        .host_ledger
        .write(WriteOp::EnvironmentApplication(record))
        .await
        .map_err(|_| Error::LedgerFailure)?;
    state.host_ledger.flush().await.map_err(|_| Error::LedgerFailure)
}
