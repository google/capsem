//! Child-process reapers that own instance and persistent-registry cleanup.

use super::*;
mod retirement;
pub(crate) use retirement::{Registration, Retirements};

pub(super) fn kill_and_reap(
    mut child: tokio::process::Child,
    mut retirement: Registration,
) -> tokio::task::JoinHandle<()> {
    let _ = child.start_kill();
    tokio::spawn(async move {
        if retirement.wait_child(&mut child).await.is_ok() {
            retirement.complete();
        }
    })
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
    mut retirement: Registration,
) -> tokio::task::JoinHandle<()> {
    let pid = child.id();
    // Registration precedes this call. Retain the spawn identity before the
    // task can be delayed across an exit/replacement or PID reuse.
    let generation = retirement.generation();
    tokio::spawn(async move {
        let Ok(exit_status) = retirement.wait_child(&mut child).await else {
            return;
        };
        info!(id, ?exit_status, "capsem-process exited, cleaning up");
        // Even a delayed reaper must fence its exact old runtime. A resumed
        // replacement may already own the map, but never owns these handles.
        state
            .connections
            .fence_runtime(&id, generation, capsem_credentials::GrantStopPolicy::Preserve);

        // Native wait is positive exit authority. Persist the fenced receipt
        // before moving a named session, so a later host boot can reprepare
        // a new generation without interpreting an old numeric PID.
        let receipt_id = id.clone();
        let receipt_dir = session_dir.clone();
        let receipt = state
            .off_worker(move |state| {
                let binding = capsem_core::managed_sessions::VmBinding::new(receipt_id, generation)?;
                state.persist_native_exit(&receipt_dir, &binding, false)
            })
            .await;
        match receipt {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(id, %error, "durable native exit receipt remains unconfirmed"),
            Err(error) => warn!(id, error = %error.1, "native exit receipt worker interrupted"),
        }

        // An ephemeral VM's removal from the instances map below is the
        // trigger for preserve_failed_session_dir; if `removed` is Some, the
        // child exited without an explicit service-side shutdown removing it.
        // A clean process exit is a graceful shutdown regardless of whether
        // the guest or service initiated it; anything else is a crash.
        let removed = {
            let mut instances = state.instances.lock().unwrap();
            if instances
                .get(&id)
                .is_some_and(|instance| Some(instance.pid) != pid || instance.generation != generation)
            {
                // A cold fallback can replace a failed restore while holding
                // the exclusive guard. Its registry, DB and sockets are not
                // owned by this delayed reaper.
                tracing::debug!(
                    id,
                    ?pid,
                    "replacement process owns session; skipping stale exit cleanup"
                );
                retirement.complete();
                return;
            }
            instances.remove(&id)
        };
        if let Some(info) = &removed {
            // The VM is gone: a container setup for it has nothing left to do.
            let policy = connection_routes::lifecycle::stop_policy(info.persistent, true);
            if let Err(error) = state.connections.stop_runtime(&id, generation, policy).await {
                warn!(id, error = %error.1, "exited session credential metadata remains pending");
            }
            state.containers.cancel_and_wait(&id).await;
        }
        // Publish the exit before waiting: restore holds the write guard
        // while readiness polls this registry to detect a crashed child.
        // Filesystem/DB cleanup must not overlap the replacement's launch.
        let _lifecycle_guard = state.lifecycle.vz.read().await;
        if state.instances.lock().unwrap().contains_key(&id) {
            tracing::debug!(
                id,
                "session replaced while exit cleanup waited; leaving replacement intact"
            );
            retirement.complete();
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
        let clean_exit = exit_status.success();
        let unexpected_exit = removed.is_some() && !clean_exit;
        if removed.is_some() {
            let status = if clean_exit { "stopped" } else { "crashed" };
            if let Err(error) = state.record_host_session_stopped(&id, status, true).await {
                error!(
                    id,
                    status,
                    error = %error,
                    "failed to record the session stop in the host ledger after child exit"
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
        retirement.complete();
    })
}
