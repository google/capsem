//! Delivery of profile changes to running VMs.
use super::*;

/// Deliver the current profile to every running VM on it: re-materialize each
/// session's active profile from the profile files, then ask capsem-process to
/// reload it. Plugin and rule mutation routes call this so an edit is enforced
/// before the route returns; a rule that only reached the profile file left
/// running VMs on the old policy until someone called the reload route. An
/// instance that does not acknowledge fails the request loudly, because a VM
/// silently left on stale policy is the worse outcome.
///
/// A VM counts as updated only when it reports applying the exact bytes this
/// call wrote. An acknowledgement of some other active profile -- one written
/// by a concurrent edit -- is a failure, not a success.
pub(crate) async fn push_profile_to_running_instances(
    state: &Arc<ServiceState>,
    _mutation: &PolicyMutation<'_>,
    profile_filter: Option<&str>,
) -> Result<usize, AppError> {
    let filter = profile_filter.map(str::to_owned);
    let published = state
        .off_worker(move |state| state.refresh_active_profiles(filter.as_deref()))
        .await?
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;

    let targets = {
        let instances = state.instances.lock().unwrap();
        published
            .into_iter()
            .filter_map(|(id, digest)| {
                let uds_path = instances.get(&id)?.uds_path.clone();
                Some((id, uds_path, digest))
            })
            .collect::<Vec<_>>()
    };

    let results = futures::future::join_all(targets.iter().map(|(id, uds_path, expected)| async move {
        let request = ServiceToProcess::ReloadConfig {
            id: state.next_job_id(),
        };
        match send_ipc_command(uds_path, request, Some(5)).await {
            Ok(ProcessToService::ConfigReloadResult {
                active_profile_digest: Some(applied),
                error: None,
                ..
            }) if &applied == expected => None,
            Ok(ProcessToService::ConfigReloadResult {
                active_profile_digest: Some(applied),
                error: None,
                ..
            }) => Some(format!("{id}: applied active profile {applied}, expected {expected}")),
            Ok(ProcessToService::ConfigReloadResult { error: Some(error), .. }) => {
                Some(format!("{id}: reload refused: {error}"))
            }
            Ok(_) => Some(format!("{id}: unexpected response")),
            Err(e) => Some(format!("{id}: {e}")),
        }
    }))
    .await;
    let failures: Vec<String> = results.into_iter().flatten().collect();

    if failures.is_empty() {
        Ok(targets.len())
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reload config in some instances: {}", failures.join(", ")),
        ))
    }
}
