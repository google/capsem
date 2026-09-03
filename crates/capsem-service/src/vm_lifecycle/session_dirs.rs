use super::*;

/// Move an ephemeral session directory under `persistent/` and register it,
/// as one step under the registry lock.
///
/// The name check, the rename and the registration used to be three separate
/// lock acquisitions. Two persists racing on one name both passed the check;
/// the loser's directory was renamed under `persistent/<id>`, `register`
/// refused it, and nothing moved it back: no registry entry, and an
/// `InstanceInfo` still saying `persistent: false` and pointing at a
/// directory that no longer existed.
///
/// Blocking: call from `spawn_blocking`. The registry mutex is held across
/// the rename on purpose -- that is the atomicity.
pub(super) fn claim_persistent_session(
    state: &ServiceState,
    entry: PersistentVmEntry,
    from: &StdPath,
) -> Result<(), AppError> {
    let vm_id = entry.id.clone();
    let to = entry.session_dir.clone();
    let mut registry = state.persistent_registry.lock().unwrap();
    if registry.contains(&entry.name) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("persistent VM \"{}\" already exists", entry.name),
        ));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create persistent root {}: {e}", parent.display()),
            )
        })?;
    }
    std::fs::rename(from, &to).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to move session dir: {e}"),
        )
    })?;
    let registered = registry.register(entry);
    drop(registry);
    if let Err(error) = registered {
        match std::fs::rename(&to, from) {
            Ok(()) => warn!(
                vm_id,
                from = %from.display(),
                to = %to.display(),
                error = %error,
                "persistent registration refused; session dir moved back"
            ),
            Err(rollback) => error!(
                vm_id,
                from = %from.display(),
                to = %to.display(),
                error = %error,
                rollback_error = %rollback,
                "persistent registration refused and the session dir could not be moved back"
            ),
        }
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()));
    }
    Ok(())
}

/// Remove a purged session directory through the service's delete contract,
/// which refuses anything outside the run roots. Returns whether it is gone.
///
/// The registry is a JSON file under the user's home: a `session_dir` it
/// reports is data, not authority, and used to go straight to
/// `remove_dir_all`.
pub(super) async fn remove_purged_session_dir(state: &Arc<ServiceState>, vm_id: &str, session_dir: PathBuf) -> bool {
    let delete_state = Arc::clone(state);
    let dir = session_dir.clone();
    let outcome = tokio::task::spawn_blocking(move || delete_state.delete_session_dir(&dir)).await;
    let error = match outcome {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => error.to_string(),
        Err(join_error) => format!("purge delete task failed: {join_error}"),
    };
    warn!(
        vm_id,
        session_dir = %session_dir.display(),
        error = %error,
        "purge refused to remove session dir; leaving its registry entry in place"
    );
    false
}
