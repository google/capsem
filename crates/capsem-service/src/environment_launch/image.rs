//! Saved workloads are selected only from the acknowledged original owner.
use super::*;
use crate::owner_connection::OwnerConnection;
use capsem_config::environment::EnvironmentImage;
use std::sync::Arc;

pub(crate) struct SelectedImage {
    pub(crate) owner: Arc<OwnerConnection>,
    pub(crate) reference: EnvironmentRevisionParent,
    pub(crate) image: EnvironmentImage,
    pub(crate) startup: Option<capsem_config::environment::EnvironmentStartup>,
}
impl SelectedImage {
    pub(crate) fn context(&self) -> Result<capsem_core::container::stage::EnvironmentLaunchContext> {
        Ok(capsem_core::container::stage::EnvironmentLaunchContext {
            session_id: uuid::Uuid::parse_str(self.owner.id())?,
            owner_generation: self.owner.generation(),
            environment_id: uuid::Uuid::parse_str(&self.reference.environment_id)?,
            revision_id: uuid::Uuid::parse_str(&self.reference.revision_id)?,
        })
    }
}

/// Returns true when a fresh image setup was started. Recorded workloads are
/// re-admitted and rebound here, then restored by the existing lifecycle owner.
pub(crate) async fn prepare_image(state: &Arc<ServiceState>, id: &str, attach: bool) -> Result<bool, crate::AppError> {
    let unavailable = |error: String| crate::AppError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, error);
    let selected = selected_image(state, id).await.map_err(unavailable)?;
    let Some((selected, variables)) = selected else {
        return Ok(false);
    };
    let observed_id = id.to_owned();
    let recorded = state
        .off_worker(move |state| crate::container_setup::recorded_image(&state, &observed_id))
        .await?
        .map_err(unavailable)?;
    selected.owner.validate(state, false).map_err(unavailable)?;
    if let Some(recorded) = recorded {
        let mut admission = crate::container_setup::images::Decision::new(state)
            .await
            .map_err(crate::AppError::from)?;
        let requested = admission
            .resolve_saved(&selected.image)
            .await
            .map_err(crate::AppError::from)?;
        let admitted = admission.admit_saved(&requested).await.map_err(crate::AppError::from)?;
        if recorded != admitted.to_string() {
            return Err(unavailable(
                "recorded workload does not match the selected environment image".into(),
            ));
        }
        let context = selected
            .context()
            .map_err(|_| unavailable("invalid environment launch identity".into()))?;
        let owner = selected.owner;
        state
            .off_worker(move |state| {
                owner.validate(&state, false)?;
                let session = crate::resolve_session_dir(&state, owner.id()).map_err(crate::AppError::into_message)?;
                capsem_core::container::workspace_owner::rebind_environment(&session, &context)
                    .map_err(|error| format!("rebind trusted environment launch: {error:#}"))?;
                owner.validate(&state, false)
            })
            .await?
            .map_err(unavailable)?;
        return Ok(false);
    }
    let spec = capsem_api::ContainerSpec {
        image: selected.image.reference.clone(),
        args: Vec::new(),
        env: variables,
        registry: None,
        attach,
    };
    crate::container_setup::start_selected(state, id.to_owned(), spec, selected);
    Ok(true)
}

async fn selected_image(
    state: &Arc<ServiceState>,
    id: &str,
) -> Result<Option<(SelectedImage, std::collections::BTreeMap<String, String>)>, String> {
    let (owner, reference) = {
        let instances = state.instances.lock().unwrap();
        let instance = instances.get(id).ok_or("environment image owner ended")?;
        let Some(binding) = &instance.environment else {
            return Ok(None);
        };
        if !binding.acknowledged {
            return Err("environment policy has not been acknowledged".into());
        }
        let captured = (Arc::new(OwnerConnection::capture(instance)?), binding.boot.clone());
        drop(instances);
        captured
    };
    let snapshot = state
        .environments
        .load_revision(&reference.environment_id, &reference.revision_id)
        .await
        .map_err(|_| "verified environment image unavailable")?;
    owner.validate(state, false)?;
    validate_preparation(&snapshot.specification).map_err(|error| error.to_string())?;
    let specification = *snapshot.specification;
    let Some(image) = specification.image else {
        return Ok(None);
    };
    Ok(Some((
        SelectedImage {
            owner,
            reference,
            image,
            startup: specification.startup,
        },
        specification.variables,
    )))
}

/// Read the exact acknowledged boot revision once for a lifecycle wait. No
/// polled route fetches a registry or retains another specification cache.
pub(crate) async fn preparation_timeout(state: &Arc<ServiceState>, id: &str) -> Result<std::time::Duration, String> {
    let selected = selected_image(state, id).await?;
    let seconds = selected
        .and_then(|(selected, _variables)| selected.startup)
        .map_or(0, |startup| startup.timeout_seconds);
    Ok(std::time::Duration::from_secs(u64::from(seconds)))
}

/// Staging may precede attachment by hours. Re-admit the retained image under
/// the dispatch policy lock, without resolving a moving tag or pulling bytes.
pub(super) async fn admit_preparation_image(state: &Arc<ServiceState>, owner: &OwnerConnection) -> Result<(), String> {
    let Some((selected, _variables)) = selected_image(state, owner.id()).await? else {
        return Ok(());
    };
    if selected.owner.generation() != owner.generation() {
        return Err("image admission owner changed".into());
    }
    owner.validate(state, false)?;
    let mut decision = crate::container_setup::images::Decision::new(state)
        .await
        .map_err(|error| format!("saved image admission: {error}"))?;
    let requested = decision
        .resolve_saved(&selected.image)
        .await
        .map_err(|error| format!("saved image admission: {error}"))?;
    decision
        .admit_saved(&requested)
        .await
        .map_err(|error| format!("saved image admission: {error}"))?;
    owner.validate(state, false)
}
