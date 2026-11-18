//! One acknowledged publication path for startup and later policy edits.
use super::*;
use crate::owner_connection::OwnerConnection;

pub(crate) async fn apply_to_owner(
    state: &Arc<ServiceState>,
    owner: &OwnerConnection,
    published: &PublishedActivePolicy,
    _mutation: &PolicyMutation<'_>,
) -> Result<(), String> {
    owner.validate(state, false)?;
    let expected = &published.digest;
    let response = crate::vm_files::send_owner_command(
        state,
        owner,
        ServiceToProcess::ReloadConfig {
            id: state.next_job_id(),
            active_policy: published.bytes.clone(),
        },
        Some(5),
    )
    .await
    .map_err(|error| error.to_string())?;
    match response {
        ProcessToService::ConfigReloadResult {
            active_policy_digest: Some(applied),
            error: None,
            ..
        } if &applied == expected => {}
        ProcessToService::ConfigReloadResult {
            active_policy_digest: Some(applied),
            error: None,
            ..
        } => return Err(format!("applied active policy {applied}, expected {expected}")),
        ProcessToService::ConfigReloadResult { error: Some(error), .. } => {
            return Err(format!("reload refused: {error}"))
        }
        _ => return Err("owner did not acknowledge the exact published policy".into()),
    }
    owner.validate(state, false)?;
    let publisher = state
        .instances
        .lock()
        .unwrap()
        .get(owner.id())
        .filter(|instance| instance.generation == owner.generation())
        .map(|instance| instance.upstream_policy.clone())
        .ok_or("policy owner changed")?;
    let proxy = state.proxy_worker(owner.id(), owner.generation())?;
    let applied = proxy
        .apply_policy(published.bytes.clone())
        .await
        .map_err(|error| format!("proxy reload refused: {error:#}"))?;
    if applied != *expected {
        return Err(format!("proxy applied {applied}, expected {expected}"));
    }
    owner.validate(state, false)?;
    publisher.publish(published.broker_policy()).await?;
    owner.validate(state, false)?;
    connection_routes::policy::acknowledge(state, owner.id(), owner.generation(), expected)
        .await
        .map_err(|_| "applied credential policy ownership unavailable")?;
    owner.validate(state, false)?;
    let mut instances = state.instances.lock().unwrap();
    let instance = instances
        .get_mut(owner.id())
        .filter(|instance| instance.generation == owner.generation())
        .ok_or("policy owner changed")?;
    if let Some(binding) = &mut instance.environment {
        binding.policy_digest.clone_from(expected);
    }
    drop(instances);
    Ok(())
}
