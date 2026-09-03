use super::*;

/// Claim `entry.name` and register the session where it currently lives, as
/// one step under the registry lock.
///
/// The directory is not moved here. A running capsem-process holds its
/// session directory by path -- the VirtioFS workspace, the auto-snapshot
/// scheduler, the MCP file tools and every lazily opened session.db reader
/// name `sessions/<id>/...` -- so renaming it under a live process left
/// snapshots, file tools and history failing until the next restart. The
/// move to `persistent/<id>` is [`settle_persistent_session_dir`], run once
/// the process has exited.
///
/// Blocking (the registry saves to disk): call from `spawn_blocking`.
pub(super) fn claim_persistent_name(state: &ServiceState, entry: PersistentVmEntry) -> Result<(), AppError> {
    let registry = state.persistent_registry.lock().unwrap();
    if registry.contains(&entry.name) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("persistent VM \"{}\" already exists", entry.name),
        ));
    }
    registry
        .register(entry)
        .map_err(|error| AppError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

/// Move a persisted session that still lives where it ran to
/// `persistent/<id>`, and point its registry entry there. Returns the
/// directory the session is in afterwards: the new one on success, the one
/// it was given when there is nothing to move or the move failed.
///
/// Called by the exit reaper for every child and by resume before it spawns,
/// so a process is never launched from a directory a pending reaper is about
/// to rename. Both take the registry lock, which serializes the two. A
/// failed move is logged and the entry keeps naming the live directory; the
/// session stays reachable instead of vanishing.
///
/// Blocking: call from `spawn_blocking` on the async side.
pub(crate) fn settle_persistent_session_dir(state: &ServiceState, name: &str, exited_dir: &StdPath) -> PathBuf {
    let persistent_root = state.run_dir.join("persistent");
    let (saved, from, to, vm_id) = {
        let mut registry = state.persistent_registry.lock().unwrap();
        let (from, vm_id) = match registry.get(name) {
            Some(entry) if entry.session_dir.parent() == Some(persistent_root.as_path()) => {
                return entry.session_dir.clone();
            }
            Some(entry) => (entry.session_dir.clone(), entry.id.clone()),
            None => return exited_dir.to_path_buf(),
        };
        let to = persistent_root.join(&vm_id);
        let moved = std::fs::create_dir_all(&persistent_root).and_then(|()| std::fs::rename(&from, &to));
        if let Err(error) = moved {
            error!(
                vm_id,
                name,
                from = %from.display(),
                to = %to.display(),
                error = %error,
                "persisted session dir could not be moved under persistent/; it stays where it ran"
            );
            return from;
        }
        if let Some(entry) = registry.get_mut(name) {
            entry.session_dir = to.clone();
        }
        (registry.save(), from, to, vm_id)
    };
    if let Err(error) = saved {
        error!(vm_id, name, error = %error, "failed to save persistent registry after settling session dir");
    }
    info!(vm_id, name, from = %from.display(), to = %to.display(), "settled persisted session dir");
    to
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
