//! The `SandboxInfo` a list or info route reports for one VM.
use super::*;

/// The list/info row of a running VM, from its in-memory record alone.
pub(super) fn running_sandbox_info(i: &InstanceInfo) -> SandboxInfo {
    let mut info = SandboxInfo::new(
        i.id.clone(),
        i.profile_id.clone(),
        i.pid,
        VmLifecycleState::Running,
        i.persistent,
    );
    info.name = Some(i.name.clone());
    info.ram_mb = Some(i.ram_mb);
    info.cpus = Some(i.cpus);
    info.version = Some(i.base_version.clone());
    info.forked_from = i.forked_from.clone();
    info.private_address = Some(i.private_address);
    info.uptime_secs = Some(i.start_time.elapsed().as_secs());
    info.can_resume = false;
    info.refresh_available_actions();
    info
}

/// The list/info row of a stopped, suspended or defunct persistent VM. A
/// blocked resume explains itself: as the crash's last error for a defunct
/// VM, as the reason otherwise.
pub(super) fn inactive_sandbox_info(
    vm_id: String,
    entry: &PersistentVmEntry,
    status: VmLifecycleState,
    can_resume: bool,
    blocked_reason: Option<String>,
) -> SandboxInfo {
    let mut info = SandboxInfo::new(vm_id, entry.profile_id.clone(), 0, status, true);
    info.name = Some(entry.name.clone());
    info.ram_mb = Some(entry.ram_mb);
    info.cpus = Some(entry.cpus);
    info.version = Some(entry.base_version.clone());
    info.forked_from = entry.forked_from.clone();
    info.description = entry.description.clone();
    info.private_address = entry.private_address;
    info.can_resume = can_resume;
    if can_resume {
        info.resume_blocked_reason = None;
    } else if entry.defunct {
        info.last_error = blocked_reason;
    } else {
        info.resume_blocked_reason = blocked_reason;
    }
    info.refresh_available_actions();
    info
}
