//! A recorded workload under a new owner: a fork, a `--from` clone, a resume.
//!
//! The guest relaunches a `ready` stage on every boot (capsem-init), so mostly
//! the host side needs restoring: the live status that routes exec into the
//! workload, and the surface grant. A stage whose first launch died before
//! `ready` is the one case the owner starts itself (`needs_relaunch`). An exposure lives in the VM owner process
//! that made it, so a new owner never inherits one; it is granted again once
//! the workload runs.
//!
//! The image itself needs nothing from here. A relaunch unpacks from the
//! session's own image share, which is host-only and survives a stop, or not
//! at all when the unpacked root is already on its overlay; a clone's share is
//! linked from its source's (`capsem_core::session::clone_sandbox_state`).
//! Neither reads the workspace, and neither depends on the host's blob cache,
//! which may have pruned the image since.

use super::*;
use capsem_foundation::unix::contained::ContainedDir;

/// Carry `source`'s launch record into the session cloned from it, without
/// its surface exposure: that belongs to the source's owner. The record pins
/// the manifest the clone's carried image share holds.
pub(crate) fn carry_launch_record(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    let Some(mut record) = read_launch_record(source) else {
        return Ok(());
    };
    if let Some(surface) = record.surface.as_mut() {
        surface.exposure_id = None;
    }
    let bytes = serde_json::to_vec(&record).map_err(|e| format!("encode launch record: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(&destination.join(LAUNCH_RECORD), &bytes)
        .map_err(|e| format!("write launch record: {e}"))?;
    // A clone of a running session carries its live markers; the clone has
    // not booted, and its own launcher has not run.
    forget_previous_run(destination)
}

/// Forget how a previous boot's workload ran, before session `session_dir`
/// boots again: its markers would otherwise report a workload running (or
/// exited) until this boot's launcher clears them, and exec would be routed
/// to a workload that is not there. The stage is guest-written, so it is
/// reached and cleared without following links. Never call on a booted VM.
pub(crate) fn forget_previous_run(session_dir: &std::path::Path) -> Result<(), String> {
    use capsem_core::container::{STAGE, STAGE_EXITED, STAGE_FAILED, STAGE_RUNNING};
    use std::ffi::OsStr;
    let absent = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let workspace = match capsem_core::session::open_workspace(session_dir) {
        Ok(workspace) => workspace,
        Err(e) if absent(&e) => return Ok(()),
        Err(e) => return Err(format!("open workspace of {}: {e}", session_dir.display())),
    };
    let stage = match workspace.descend(OsStr::new(STAGE)) {
        Ok(stage) => stage,
        Err(e) if absent(&e) => return Ok(()),
        Err(e) => return Err(format!("open the workload stage of {}: {e}", session_dir.display())),
    };
    for marker in [STAGE_RUNNING, STAGE_FAILED, STAGE_EXITED] {
        match stage.remove_non_directory(OsStr::new(marker)) {
            Err(e) if !absent(&e) => return Err(format!("clear the {marker} marker: {e}")),
            _ => {}
        }
    }
    Ok(())
}

/// Put VM `id`'s recorded workload back under this owner: live again, with
/// no exposure, and its surface granted once the workload reports running.
/// A VM without a record has no workload and is left alone.
pub(crate) fn restore(state: &Arc<ServiceState>, id: &str) {
    let Ok(session_dir) = resolve_session_dir(state, id) else {
        return;
    };
    let Some(record) = read_launch_record(&session_dir) else {
        return;
    };
    let manifest = record.manifest.clone();
    let owner = match record
        .cache_key
        .as_deref()
        .map(capsem_assets::oci::CacheKey::parse)
        .transpose()
    {
        Ok(Some(key)) if manifest.is_some() => {
            let instances = state.instances.lock().unwrap();
            instances.get(id).and_then(|instance| {
                capsem_core::managed_sessions::VmBinding::new(id.to_owned(), instance.generation)
                    .ok()
                    .map(|vm| cache_owners::CacheOwner { key, vm })
            })
        }
        Err(error) => {
            warn!(vm_id = id, %error, "recorded cache identity is invalid");
            None
        }
        _ => None,
    };
    let mut status = record.starting();
    if let Some(surface) = status.surface.as_mut() {
        surface.exposure_id = None;
    }
    let generation = state.containers.begin(id, &status.image);
    state.containers.advance(id, generation, |live| *live = status);
    state.containers.pin_image(id, generation, manifest, owner);
    if needs_relaunch(state, id) {
        relaunch_in_background(state, id, generation);
    }
    grant_surface_in_background(state, id, generation);
}

/// A restored guest may be ready while its workload is still starting. Keep
/// launch-record I/O off the async worker and await this boot's workload marker.
pub(crate) async fn restore_ready(state: &Arc<ServiceState>, id: &str) -> Result<(), AppError> {
    let restore_id = id.to_owned();
    let has_workload = state
        .off_worker(move |state| {
            restore(&state, &restore_id);
            state.containers.status(&restore_id).is_some()
        })
        .await?;
    if has_workload {
        require_ready(state, id).await?;
    }
    Ok(())
}

/// Whether VM `id` holds a staged workload its boot did not start: capsem-init
/// relaunches only a stage the launcher marked `ready`, which it does once the
/// image is unpacked. A first launch that died before then left a stage with
/// its options and no marker, which nothing else would ever start. A boot that
/// found `ready` launched it already, so this never starts a second workload.
pub(crate) fn needs_relaunch(state: &ServiceState, id: &str) -> bool {
    staged_marker(state, id, "options.json") && !staged_marker(state, id, capsem_core::container::STAGE_READY)
}

/// Start VM `id`'s staged workload the way a create does, detached.
fn relaunch_in_background(state: &Arc<ServiceState>, id: &str, generation: u64) {
    let Some(lease) = state.containers.work_lease(id, generation) else {
        return;
    };
    let Ok(uds_path) = running_uds_path(state, id) else {
        return;
    };
    let (state, id) = (Arc::clone(state), id.to_owned());
    tokio::spawn(async move {
        let reply = tokio::select! {
            reply = send_ipc_command(
            &state,
            &uds_path,
            ServiceToProcess::Exec {
                id: state.next_job_id(),
                command: capsem_core::container::detached_launch_command(),
                target: capsem_proto::ipc::ExecTarget::Vm,
            },
            Some(30),
        ) => reply,
            () = lease.cancelled() => return,
        };
        match reply {
            Ok(ProcessToService::ExecResult { exit_code: 0, .. }) => {
                info!(
                    vm_id = id.as_str(),
                    "relaunched a staged workload its boot never started"
                );
            }
            other => warn!(vm_id = id.as_str(), reply = ?other, "relaunch of a staged workload failed"),
        }
    });
}

/// Forget the staged image a clone carried, so its first boot launches
/// nothing and the image the new session names is the only one staged and
/// run. Its workspace files and the image volumes on its overlay stay.
///
/// The carried image share is emptied, so not one blob of the source's image
/// is left for the new one. The whole stage goes too, not only its markers:
/// the launcher left its own copy read-only. The stage is flat and
/// guest-written, so each entry is unlinked without being followed; a
/// directory a guest put there is left in place, unread.
pub(crate) fn drop_carried_image(session_dir: &std::path::Path) -> Result<(), String> {
    let not_found = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let root = ContainedDir::open_root(session_dir).map_err(|e| format!("open {}: {e}", session_dir.display()))?;
    match root.remove_private_file(std::ffi::OsStr::new(LAUNCH_RECORD)) {
        Err(error) if !not_found(&error) => return Err(format!("remove launch record: {error}")),
        _ => {}
    }
    capsem_core::session::clear_image_share(session_dir).map_err(|e| format!("empty the carried image share: {e}"))?;
    let stage = match capsem_core::session::open_workspace(session_dir)
        .and_then(|workspace| workspace.descend(std::ffi::OsStr::new(capsem_core::container::STAGE)))
    {
        Ok(stage) => stage,
        Err(error) if not_found(&error) => return Ok(()),
        Err(error) => return Err(format!("open the carried stage: {error}")),
    };
    let entries = stage.entries().map_err(|e| format!("list the carried stage: {e}"))?;
    for entry in entries.into_iter().filter(|entry| entry.kind != EntryKind::Directory) {
        match stage.remove_non_directory(&entry.name) {
            Err(error) if !not_found(&error) => {
                return Err(format!("remove carried {}: {error}", entry.name.to_string_lossy()))
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
