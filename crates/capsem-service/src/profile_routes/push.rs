//! Delivery of profile changes to running VMs.
use super::*;

/// Deliver the current profile to every running VM on it: re-materialize each
/// session's active profile from the profile files, then ask capsem-process to
/// reload it. Plugin and rule mutation routes call this so an edit is enforced
/// before the route returns; a rule that only reached the profile file left
/// running VMs on the old policy until someone called the reload route. An
/// instance that does not acknowledge fails the request loudly, because a VM
/// silently left on stale policy is the worse outcome.
pub(crate) async fn push_profile_to_running_instances(
    state: &Arc<ServiceState>,
    profile_filter: Option<&str>,
) -> Result<usize, AppError> {
    let filter = profile_filter.map(str::to_owned);
    state
        .off_worker(move |state| state.refresh_active_profiles(filter.as_deref()))
        .await?
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .filter(|(_, info)| {
                profile_filter
                    .map(|profile_id| info.profile_id == profile_id)
                    .unwrap_or(true)
            })
            .map(|(id, info)| (id.clone(), info.uds_path.clone()))
            .collect::<Vec<_>>()
    };

    let results = futures::future::join_all(uds_paths.iter().map(|(id, uds_path)| {
        let id = id.clone();
        async move {
            match send_ipc_command(uds_path, ServiceToProcess::ReloadConfig, Some(5)).await {
                Ok(ProcessToService::Pong) => None,
                Ok(_) => Some(format!("{id}: unexpected response")),
                Err(e) => Some(format!("{id}: {e}")),
            }
        }
    }))
    .await;
    let failures: Vec<String> = results.into_iter().flatten().collect();

    if failures.is_empty() {
        Ok(uds_paths.len())
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reload config in some instances: {}", failures.join(", ")),
        ))
    }
}
