//! Applied-policy receipts are scoped to the exact original VM generation.
use super::*;
use capsem_core::net::policy_config::{active_policy_digest, ActivePolicyFile};
use capsem_core::security_engine::CredentialGrantAdmission;
#[cfg(test)]
mod tests;

#[derive(Clone)]
struct Cached {
    inputs: Arc<(
        capsem_core::net::policy_config::SettingsFile,
        capsem_core::net::policy_config::SettingsFile,
    )>,
    admission: Arc<CredentialGrantAdmission>,
}

#[derive(Clone)]
pub(super) struct Applied {
    policy: Arc<ActivePolicyFile>,
    cached: Option<Cached>,
}

fn denied() -> CredentialError {
    api::CredentialErrorCode::PolicyDenied.into()
}

fn read_policy(path: &StdPath) -> Result<(String, Arc<ActivePolicyFile>), CredentialError> {
    let bytes = capsem_foundation::unix::fs::read_regular_file_no_follow(path).map_err(|_| denied())?;
    let policy: ActivePolicyFile =
        toml::from_str(std::str::from_utf8(&bytes).map_err(|_| denied())?).map_err(|_| denied())?;
    policy.validate().map_err(|_| denied())?;
    Ok((active_policy_digest(&bytes), Arc::new(policy)))
}

fn record(state: &ServiceState, binding: &VmBinding, policy: Arc<ActivePolicyFile>) -> Result<(), CredentialError> {
    let instances = state.instances.lock().unwrap();
    if !instances
        .get(binding.id())
        .is_some_and(|instance| instance.generation == binding.generation())
        || !state
            .retirements
            .is_live(binding.id(), binding.generation())
            .unwrap_or(false)
    {
        return Err(api::CredentialErrorCode::OwnerChanged.into());
    }
    state.connections.applied.lock().unwrap().insert(
        (binding.id().into(), binding.generation()),
        Applied { policy, cached: None },
    );
    drop(instances);
    Ok(())
}

/// Called only for a matching existing ReloadConfig acknowledgement. Verify
/// the exact bytes again before remembering the generation's applied policy.
pub(crate) async fn acknowledge(
    state: &Arc<ServiceState>,
    id: &str,
    generation: uuid::Uuid,
    expected: &str,
) -> Result<(), CredentialError> {
    if !state.connections.broker.get().is_some_and(|broker| {
        broker
            .as_ref()
            .is_ok_and(|broker| broker.configured_capabilities().next().is_some())
    }) {
        return Ok(());
    }
    let path = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .filter(|instance| instance.generation == generation)
        .map(|instance| instance.session_dir.join(ACTIVE_POLICY_DIR).join(ACTIVE_POLICY_FILE))
        .ok_or(api::CredentialErrorCode::OwnerChanged)?;
    let (digest, policy) = state
        .off_worker(move |_| read_policy(&path))
        .await
        .map_err(|_| CredentialError(api::CredentialErrorCode::StorageUnavailable, false))??;
    if digest != expected {
        return Err(denied());
    }
    let binding = VmBinding::new(id.into(), generation).map_err(|_| denied())?;
    record(state, &binding, policy)
}

fn compute(
    applied: Applied,
    current: Arc<(
        capsem_core::net::policy_config::SettingsFile,
        capsem_core::net::policy_config::SettingsFile,
    )>,
    capabilities: &[capsem_credentials::GrantCapability],
) -> Result<Arc<CredentialGrantAdmission>, CredentialError> {
    match applied.cached {
        Some(cached) if cached.inputs.as_ref() == current.as_ref() => Ok(cached.admission),
        _ => Ok(Arc::new(
            CredentialGrantAdmission::new(
                &ActivePolicyFile::from_settings_and_corp(&current.0, &current.1).map_err(|_| denied())?,
                &applied.policy,
                capabilities.iter(),
            )
            .map_err(|_| denied())?,
        )),
    }
}

async fn confirm(state: &Arc<ServiceState>, context: &Context) -> Result<Applied, CredentialError> {
    let path = context.policy_path.clone();
    let (expected, applied) = state
        .off_worker(move |_| read_policy(&path))
        .await
        .map_err(|_| CredentialError(api::CredentialErrorCode::StorageUnavailable, false))??;
    let response = send_ipc_command(
        &context.uds_path,
        ServiceToProcess::ReloadConfig {
            id: state.next_job_id(),
        },
        Some(5),
    )
    .await
    .map_err(|_| denied())?;
    if !matches!(response, ProcessToService::ConfigReloadResult { active_policy_digest: Some(digest), error: None, .. } if digest == expected)
        || !context.current(state)
    {
        return Err(denied());
    }
    record(state, &context.runtime, Arc::clone(&applied))?;
    Ok(Applied {
        policy: applied,
        cached: None,
    })
}

pub(super) async fn admission(
    state: &Arc<ServiceState>,
    context: &Context,
    broker: &ConnectionBroker,
    prove: bool,
) -> Result<Arc<CredentialGrantAdmission>, CredentialError> {
    let capabilities: Vec<_> = broker.configured_capabilities().cloned().collect();
    if capabilities.is_empty() {
        return Ok(Arc::new(CredentialGrantAdmission::deny_all()));
    }
    let applied = if prove {
        confirm(state, context).await?
    } else {
        let known = state
            .connections
            .applied
            .lock()
            .unwrap()
            .get(&(context.runtime.id().into(), context.runtime.generation()))
            .cloned();
        let Some(applied) = known else {
            return Ok(Arc::new(CredentialGrantAdmission::deny_all()));
        };
        applied
    };
    let applied_policy = Arc::clone(&applied.policy);
    let (current, admission) = state
        .off_worker(move |_| {
            let (user, corp) = capsem_core::net::policy_config::load_policy_files().map_err(|_| denied())?;
            let current = Arc::new((user, corp));
            let admission = compute(applied, Arc::clone(&current), &capabilities)?;
            Ok::<_, CredentialError>((current, admission))
        })
        .await
        .map_err(|_| CredentialError(api::CredentialErrorCode::StorageUnavailable, false))??;
    if let Some(applied) = state
        .connections
        .applied
        .lock()
        .unwrap()
        .get_mut(&(context.runtime.id().into(), context.runtime.generation()))
    {
        if Arc::ptr_eq(&applied.policy, &applied_policy) {
            applied.cached = Some(Cached {
                inputs: current,
                admission: Arc::clone(&admission),
            });
        }
    }
    Ok(admission)
}
