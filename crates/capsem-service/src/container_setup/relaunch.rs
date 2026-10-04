//! A recorded workload under a new owner: a fork, a `--from` clone, a resume.
//!
//! The guest relaunches a staged image on every boot (capsem-init), so only
//! the host side needs restoring: the live status that routes exec into the
//! workload, and the surface grant. An exposure lives in the VM owner process
//! that made it, so a new owner never inherits one; it is granted again once
//! the workload runs.

use super::*;
use capsem_foundation::unix::contained::ContainedDir;

/// Carry `source`'s launch record into the session cloned from it, without
/// its surface exposure: that belongs to the source's owner.
pub(crate) fn carry_launch_record(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    let Some(mut record) = read_launch_record(source) else {
        return Ok(());
    };
    if let Some(surface) = record.surface.as_mut() {
        surface.exposure_id = None;
    }
    let bytes = serde_json::to_vec(&record).map_err(|e| format!("encode launch record: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(&destination.join(LAUNCH_RECORD), &bytes)
        .map_err(|e| format!("write launch record: {e}"))
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
    let mut status = record.starting();
    if let Some(surface) = status.surface.as_mut() {
        surface.exposure_id = None;
    }
    let generation = state.containers.begin(id, &status.image);
    state.containers.advance(id, generation, |live| *live = status);
    grant_surface_in_background(state, id, generation);
}

/// Forget the staged image a clone carried, so its first boot launches
/// nothing and the image the new session names is the only one staged and
/// run. Its workspace files and the image volumes on its overlay stay. The
/// markers are guest-written, so they are removed without being followed.
pub(crate) fn drop_carried_image(session_dir: &std::path::Path) -> Result<(), String> {
    let not_found = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let root = ContainedDir::open_root(session_dir).map_err(|e| format!("open {}: {e}", session_dir.display()))?;
    match root.remove_private_file(std::ffi::OsStr::new(LAUNCH_RECORD)) {
        Err(error) if !not_found(&error) => return Err(format!("remove launch record: {error}")),
        _ => {}
    }
    let stage = match capsem_core::session::open_workspace(session_dir)
        .and_then(|workspace| workspace.descend(std::ffi::OsStr::new(capsem_core::container::STAGE)))
    {
        Ok(stage) => stage,
        Err(error) if not_found(&error) => return Ok(()),
        Err(error) => return Err(format!("open the carried stage: {error}")),
    };
    for marker in [
        capsem_core::container::STAGE_READY,
        capsem_core::container::STAGE_RUNNING,
        capsem_core::container::STAGE_FAILED,
    ] {
        match stage.remove_non_directory(std::ffi::OsStr::new(marker)) {
            Err(error) if !not_found(&error) => return Err(format!("remove stage marker {marker}: {error}")),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
