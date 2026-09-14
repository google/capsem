//! Child-process reapers that own instance and persistent-registry cleanup.

use super::*;

pub(super) fn kill_and_reap(mut child: tokio::process::Child) {
    let _ = child.start_kill();
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
}

/// The one child reaper: every `capsem-process` exit, from a fresh provision
/// or a resume, passes through here. The resume path used to have its own
/// that removed the instance and nothing else, so a resumed persistent VM
/// that suspended again was never marked suspended and one that crashed was
/// never marked defunct.
pub(super) fn spawn_exit_reaper(
    mut child: tokio::process::Child,
    id: String,
    name: String,
    state: Arc<ServiceState>,
    uds_path: PathBuf,
    session_dir: PathBuf,
) -> tokio::task::JoinHandle<()> {
    let pid = child.id();
    tokio::spawn(async move {
        let exit_status = child.wait().await.ok();
        info!(id, ?exit_status, "capsem-process exited, cleaning up");

        // An ephemeral VM's removal from the instances map below is the
        // trigger for preserve_failed_session_dir; if `removed` is Some, the
        // child exited without an explicit service-side shutdown removing it.
        // A clean process exit is a graceful shutdown regardless of whether
        // the guest or service initiated it; anything else is a crash.
        let removed = {
            let mut instances = state.instances.lock().unwrap();
            if instances.get(&id).is_some_and(|instance| Some(instance.pid) != pid) {
                // A cold fallback can replace a failed restore while holding
                // the exclusive guard. Its registry, DB and sockets are not
                // owned by this delayed reaper.
                tracing::debug!(
                    id,
                    ?pid,
                    "replacement process owns session; skipping stale exit cleanup"
                );
                return;
            }
            instances.remove(&id)
        };
        // Publish the exit before waiting: restore holds the write guard
        // while readiness polls this registry to detect a crashed child.
        // Filesystem/DB cleanup must not overlap the replacement's launch.
        let _lifecycle_guard = state.save_restore_lock.read().await;
        if state.instances.lock().unwrap().contains_key(&id) {
            tracing::debug!(
                id,
                "session replaced while exit cleanup waited; leaving replacement intact"
            );
            return;
        }
        state.unregister_session_db_handle(&id);
        // A session persisted while it ran still lives under sessions/; now
        // that nothing holds it by path, move it home. The bookkeeping below
        // looks for the checkpoint and the process log where the dir is now.
        let session_dir = {
            let settle_state = Arc::clone(&state);
            let settle_name = name.clone();
            let exited_dir = session_dir.clone();
            tokio::task::spawn_blocking(move || {
                crate::vm_lifecycle::settle_persistent_session_dir(&settle_state, &settle_name, &exited_dir)
            })
            .await
            .unwrap_or(session_dir)
        };
        let clean_exit = exit_status.as_ref().is_some_and(|status| status.success());
        let unexpected_exit = removed.is_some() && !clean_exit;
        if removed.is_some() {
            let status = if clean_exit { "stopped" } else { "crashed" };
            if let Err(error) = state.record_session_index_stop(&id, status, Some(&session_dir)) {
                error!(
                    id,
                    status,
                    error = %error,
                    "failed to record main.db session stop after child exit"
                );
            }
        }

        // A checkpoint is registry truth only after save_state + fsync wrote
        // the completion marker; a bare checkpoint may be a failed suspend.
        {
            let mut registry = state.persistent_registry.lock().unwrap();
            if let Some(entry) = registry.data.vms.get_mut(&name) {
                let checkpoint_path = session_dir.join(RESUME_CHECKPOINT_NAME);
                let checkpoint_complete_path = checkpoint_complete_path(&checkpoint_path);
                if checkpoint_path.exists() && checkpoint_complete_path.exists() {
                    info!(id, "Checkpoint file found, marking VM as suspended");
                    entry.suspended = true;
                    entry.checkpoint_path = Some(RESUME_CHECKPOINT_NAME.to_string());
                    entry.defunct = false;
                    entry.last_error = None;
                } else {
                    entry.suspended = false;
                    entry.checkpoint_path = None;
                    if unexpected_exit {
                        entry.defunct = true;
                        let tail = read_process_log_tail(&session_dir, 20);
                        warn!(
                            id,
                            cause = capsem_core::session::boot_failure_summary(&tail),
                            "persistent VM marked defunct after unexpected exit"
                        );
                        entry.last_error = Some(tail);
                    } else {
                        entry.defunct = false;
                        entry.last_error = None;
                    }
                }
                if let Err(error) = registry.save() {
                    error!(id, error = %error, "failed to save persistent registry");
                }
            }
        }

        // Preserve ephemeral crash evidence for `capsem logs`; successful or
        // explicitly owned shutdowns need no post-mortem copy.
        if let Some(info) = removed {
            if unexpected_exit {
                tracing::warn!(id, ?exit_status, "child exited unexpectedly, preserving session dir");
                if !info.persistent {
                    let _ = state.preserve_failed_session_dir(&info.session_dir, &id);
                }
            } else {
                tracing::info!(id, "child exited cleanly (guest-initiated shutdown)");
            }
        } else {
            tracing::debug!(id, "child exited after explicit service-side shutdown");
        }
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
    })
}
