//! Selection is tied to one actual registered owner generation. A desired
//! reference is not an applied receipt; acknowledgement is recorded separately.
use crate::{active_policy::PublishedActivePolicy, ServiceState};
use anyhow::{ensure, Context, Result};
use capsem_config::environment::EnvironmentSpecification;
use capsem_core::environment::EnvironmentPolicyInputs;
use capsem_logger::events::EnvironmentRevisionParent;
use std::{collections::HashMap, path::Path};
mod image;
pub(crate) use image::{matches_carried_image, preparation_timeout, prepare_image, SelectedImage};

#[derive(Clone, Debug)]
pub(crate) struct EnvironmentBinding {
    pub(crate) boot: EnvironmentRevisionParent,
    pub(crate) live_policy: EnvironmentRevisionParent,
    pub(crate) desired: EnvironmentRevisionParent,
    pub(crate) policy_digest: String,
    pub(crate) acknowledged: bool,
    pub(crate) pending_restart: bool,
}
impl EnvironmentBinding {
    pub(crate) fn starting(reference: EnvironmentRevisionParent, policy_digest: String) -> Self {
        Self {
            boot: reference.clone(),
            live_policy: reference.clone(),
            desired: reference,
            policy_digest,
            acknowledged: false,
            pending_restart: false,
        }
    }
}

/// Refuse preparation fields until their trusted launch/binding owners are
/// connected. Never launch a valid-looking environment with fields discarded.
pub(crate) fn validate_preparation(specification: &EnvironmentSpecification) -> Result<()> {
    ensure!(
        specification.repositories.is_empty(),
        "environment repositories require trusted workload preparation"
    );
    ensure!(
        specification.secrets.is_empty(),
        "environment secret access requires credential bindings"
    );
    ensure!(
        specification.packages == Default::default(),
        "environment packages require registry policy resolution"
    );
    ensure!(
        specification.skills.selected.is_empty(),
        "environment skills require pinned runtime retrieval"
    );
    ensure!(
        specification.mcps.servers.is_empty() && specification.mcps.catalog.selected.is_empty(),
        "environment MCP definitions require scoped launch preparation"
    );
    Ok(())
}

impl ServiceState {
    /// Blocking launch/refresh owner. History and archive reads remain Logger
    /// operations, and current corporate inputs never come from user settings.
    pub(crate) fn materialize_environment_policy(
        &self,
        session_dir: &Path,
        boot: Option<&EnvironmentRevisionParent>,
        live: Option<&EnvironmentRevisionParent>,
    ) -> Result<PublishedActivePolicy> {
        Ok(self.prepare_environment_policy(session_dir, boot, live)?.0)
    }

    pub(crate) fn prepare_environment_policy(
        &self,
        session_dir: &Path,
        boot: Option<&EnvironmentRevisionParent>,
        live: Option<&EnvironmentRevisionParent>,
    ) -> Result<(PublishedActivePolicy, Option<HashMap<String, String>>)> {
        let (boot_ref, live_ref) = match (boot, live) {
            (None, None) => return Ok((self.materialize_active_policy(session_dir)?, None)),
            (Some(boot), Some(live)) => (boot, live),
            _ => anyhow::bail!("environment policy requires both boot and live revision ownership"),
        };
        let runtime = tokio::runtime::Handle::current();
        let boot = runtime
            .block_on(
                self.environments
                    .load_revision(&boot_ref.environment_id, &boot_ref.revision_id),
            )
            .context("verify environment boot revision")?;
        let live = if boot_ref == live_ref {
            None
        } else {
            Some(
                runtime
                    .block_on(
                        self.environments
                            .load_revision(&live_ref.environment_id, &live_ref.revision_id),
                    )
                    .context("verify environment live policy revision")?,
            )
        };
        let live = live.as_ref().unwrap_or(&boot);
        validate_preparation(&boot.specification)?;
        let corporate = capsem_core::net::policy_config::load_corp_policy_files()
            .map_err(anyhow::Error::msg)
            .context("load current corporate environment policy")?;
        let inputs = EnvironmentPolicyInputs::new(&boot.specification, &live.specification, &corporate);
        let policy = runtime
            .block_on(crate::environment_routes::launch_policy(self, &inputs))
            .context("admit and compile selected environment rule catalogs")?;
        Ok((
            crate::active_policy::publish_active_policy(session_dir, &policy)?,
            capsem_core::container::session_env(
                Some(
                    boot.specification
                        .variables
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                ),
                boot.specification.image.is_some(),
            ),
        ))
    }
}

/// Initial application records are written only after all original-generation
/// policy owners acknowledge. An interrupted receipt can be reconciled without
/// changing a previous generation or reselecting the shared environment head.
pub(crate) async fn initialize(state: &std::sync::Arc<ServiceState>, id: &str) -> Result<(), String> {
    let (owner, binding, session) = {
        let instances = state.instances.lock().unwrap();
        let instance = instances.get(id).ok_or("environment launch owner ended")?;
        let Some(binding) = &instance.environment else {
            return Ok(());
        };
        let owner = crate::owner_connection::OwnerConnection::capture(instance)?;
        let captured = (owner, binding.clone(), instance.session_dir.clone());
        drop(instances);
        captured
    };
    if binding.acknowledged {
        return owner.validate(state, false);
    }
    crate::vm_files::readiness::wait_for_owner_ready(state, &owner, 30).await?;
    let mutation = state.policy_mutation.begin().await;
    owner.validate(state, false)?;
    if state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .and_then(|instance| instance.environment.as_ref())
        .is_some_and(|binding| binding.acknowledged)
    {
        return Ok(());
    }
    refresh_binding(state, &owner, &binding, session, &mutation).await?;
    use capsem_logger::{
        db::{EnvironmentReferenceQuery, EnvironmentReferenceRecord},
        events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation},
        WriteOp,
    };
    let rows = state
        .host_ledger
        .environment_references(EnvironmentReferenceQuery::Sandbox {
            session_id: owner.id().into(),
            generation: owner.generation().to_string(),
        })
        .await?;
    owner.validate(state, false)?;
    for (kind, reference) in [
        (EnvironmentSandboxReferenceKind::Boot, &binding.boot),
        (EnvironmentSandboxReferenceKind::LivePolicy, &binding.live_policy),
        (EnvironmentSandboxReferenceKind::Desired, &binding.desired),
    ] {
        let previous = rows.iter().find_map(|row| match row {
            EnvironmentReferenceRecord::Sandbox { dimension, revision } if *dimension == kind => Some(revision),
            _ => None,
        });
        if let Some(previous) = previous {
            if previous != reference {
                return Err("environment launch receipt changed".into());
            }
            continue;
        }
        owner.validate(state, false)?;
        state
            .host_ledger
            .write(WriteOp::EnvironmentSandboxReference(
                EnvironmentSandboxReferenceMutation {
                    session_id: owner.id().into(),
                    generation: owner.generation().to_string(),
                    kind,
                    expected: None,
                    revision: Some(reference.clone()),
                    timestamp_unix_ms: crate::policy_mutation::unix_timestamp_ms(),
                    actor: "system".into(),
                },
            ))
            .await?;
    }
    state.host_ledger.flush().await?;
    owner.validate(state, false)?;
    let mut instances = state.instances.lock().unwrap();
    let binding = instances
        .get_mut(owner.id())
        .filter(|instance| instance.generation == owner.generation())
        .and_then(|instance| instance.environment.as_mut())
        .ok_or("environment launch owner changed")?;
    binding.acknowledged = true;
    drop(instances);
    Ok(())
}

/// Linearize guest preparation after current corporate publication and an
/// exact original-owner ACK. Keep the guard until guest execution is dispatched.
pub(crate) async fn preparation_policy<'a>(
    state: &'a std::sync::Arc<ServiceState>,
    owner: &crate::owner_connection::OwnerConnection,
) -> Result<crate::policy_mutation::PolicyMutation<'a>, String> {
    let mutation = state.policy_mutation.begin().await;
    owner.validate(state, false)?;
    let selected = {
        let instances = state.instances.lock().unwrap();
        let instance = instances
            .get(owner.id())
            .filter(|instance| instance.generation == owner.generation())
            .ok_or("preparation owner changed")?;
        let selected = instance
            .environment
            .as_ref()
            .map(|binding| (binding.clone(), instance.session_dir.clone()));
        drop(instances);
        selected
    };
    if let Some((binding, session)) = selected {
        if !binding.acknowledged {
            return Err("environment policy has not been acknowledged".into());
        }
        refresh_binding(state, owner, &binding, session, &mutation).await?;
        image::admit_preparation_image(state, owner).await?;
    }
    Ok(mutation)
}

async fn refresh_binding(
    state: &std::sync::Arc<ServiceState>,
    owner: &crate::owner_connection::OwnerConnection,
    binding: &EnvironmentBinding,
    session: std::path::PathBuf,
    mutation: &crate::policy_mutation::PolicyMutation<'_>,
) -> Result<(), String> {
    let (boot, live) = (binding.boot.clone(), binding.live_policy.clone());
    let published = state
        .off_worker(move |state| state.materialize_environment_policy(&session, Some(&boot), Some(&live)))
        .await
        .map_err(|error| error.body.error)?
        .map_err(|error| format!("{error:#}"))?;
    owner.validate(state, false)?;
    crate::active_policy::apply_to_owner(state, owner, &published, mutation).await
}

#[cfg(test)]
mod tests;

/// Validate selection before allocating a VM or touching its guest directory.
pub(crate) async fn validate_selection(
    state: &std::sync::Arc<ServiceState>,
    reference: &capsem_api::environments::EnvironmentRevisionReference,
    attach: bool,
) -> Result<(), crate::AppError> {
    use axum::http::StatusCode;
    let unavailable = |_| {
        crate::AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "verified environment launch selection unavailable".into(),
        )
    };
    let records = state
        .host_ledger
        .environment_revisions(capsem_logger::db::EnvironmentRevisionQuery::Get {
            environment_id: reference.environment_id.clone(),
            revision_id: reference.revision_id.clone(),
        })
        .await
        .map_err(unavailable)?;
    if records.is_empty() {
        return Err(crate::AppError::new(
            StatusCode::NOT_FOUND,
            "environment revision not found".into(),
        ));
    }
    let selected = state
        .environments
        .load_revision(&reference.environment_id, &reference.revision_id)
        .await
        .map_err(|_| {
            crate::AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "verified environment launch selection unavailable".into(),
            )
        })?;
    if attach && selected.specification.image.is_none() {
        return Err(crate::AppError::new(
            StatusCode::BAD_REQUEST,
            "attached environment execution requires an image".into(),
        ));
    }
    validate_preparation(&selected.specification)
        .map_err(|error| crate::AppError::new(StatusCode::NOT_IMPLEMENTED, error.to_string()))
}
