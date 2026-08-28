//! Child-process reapers that own instance and persistent-registry cleanup.

use super::*;

pub(super) fn kill_and_reap(mut child: tokio::process::Child) {
    let _ = child.start_kill();
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
}

pub(super) fn spawn_provision(
    mut child: tokio::process::Child,
    id: String,
    name: String,
    state: Arc<ServiceState>,
    uds_path: PathBuf,
    session_dir: PathBuf,
) {
    tokio::spawn(async move {
        let exit_status = child.wait().await.ok();
        info!(id, ?exit_status, "capsem-process exited, cleaning up");

        // An ephemeral VM's removal from the instances map below is the
        // trigger for preserve_failed_session_dir; if `removed` is Some, the
        // child exited without an explicit service-side shutdown removing it.
        // A clean process exit is a graceful shutdown regardless of whether
        // the guest or service initiated it; anything else is a crash.
        let removed = state.instances.lock().unwrap().remove(&id);
        state.unregister_session_db_handle(&id);
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
                tracing::warn!(
                    id,
                    ?exit_status,
                    "child exited unexpectedly, preserving session dir"
                );
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
    });
}

pub(super) fn spawn_resume(
    mut child: tokio::process::Child,
    vm_id: String,
    state: Arc<ServiceState>,
    uds_path: PathBuf,
) {
    tokio::spawn(async move {
        let exit_status = child.wait().await;
        info!(vm_id, exit_status = ?exit_status, "capsem-process (resume) exited, cleaning up");
        tracing::warn!(
            vm_id,
            exit_status = ?exit_status,
            "resume_sandbox child exit handler removing instance"
        );
        state.instances.lock().unwrap().remove(&vm_id);
        state.unregister_session_db_handle(&vm_id);
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
    });
}
