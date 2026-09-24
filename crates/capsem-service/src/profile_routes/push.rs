//! Delivery of profile changes to running VMs.
use super::*;

/// Deliver the current profile to every running VM on it: re-materialize each
/// session's active profile from the profile files, then ask capsem-process to
/// reload it. Mutation routes call this so an edit is enforced before the route
/// returns; a rule that only reached the profile file left running VMs on the
/// old policy until someone called the reload route.
///
/// A VM counts as updated only when it reports applying the exact bytes this
/// call wrote; an acknowledgement of some other active profile -- one written
/// by a concurrent edit -- is a failure. One VM that fails does not stop the
/// others: the edit is applied everywhere it can be, and the error names the
/// VMs left on their previous policy. A VM silently left on stale policy is
/// the worse outcome, so any such VM fails the request.
pub(crate) async fn push_profile_to_running_instances(
    state: &Arc<ServiceState>,
    _mutation: &PolicyMutation<'_>,
    profile_filter: Option<&str>,
) -> Result<usize, AppError> {
    let filter = profile_filter.map(str::to_owned);
    let published = state
        .off_worker(move |state| state.refresh_active_profiles(filter.as_deref()))
        .await?;
    let total = published.len();

    let mut failures = Vec::new();
    let targets = {
        let instances = state.instances.lock().unwrap();
        published
            .into_iter()
            .filter_map(|(id, digest)| match digest {
                Err(error) => {
                    failures.push(format!("{id}: {error}"));
                    None
                }
                // A VM that stopped since it was listed has nothing to reload.
                Ok(digest) => Some((id.clone(), instances.get(&id)?.uds_path.clone(), digest)),
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
    failures.extend(results.into_iter().flatten());

    if failures.is_empty() {
        Ok(total)
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "policy applied to {} of {total} running VMs; not applied: {}",
                total - failures.len(),
                failures.join(", ")
            ),
        ))
    }
}
