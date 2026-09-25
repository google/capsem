//! The `SandboxInfo` a list or info route reports for one VM, and the list.
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

/// The list's lifecycle rows, and each listed VM's session directory in the
/// same order.
pub(super) fn build_list_response(state: &ServiceState) -> (ListResponse, Vec<PathBuf>) {
    let mut sandboxes: Vec<SandboxInfo> = Vec::new();
    let mut session_dirs = Vec::new();

    // Running instances, from their in-memory records.
    {
        let instances = state.instances.lock().unwrap();
        for i in instances.values() {
            sandboxes.push(running_sandbox_info(i));
            session_dirs.push(i.session_dir.clone());
        }
    }

    // Stopped/Suspended/Defunct persistent VMs (not in instances map).
    // `Defunct` surfaces a boot failure so users see the problem in
    // `capsem list` instead of a misleading "Stopped" -- last_error
    // carries the tail of process.log for one-line diagnosis.
    let inactive_persistent: Vec<PersistentVmEntry> = {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        registry
            .list()
            .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
            .cloned()
            .collect()
    };
    for entry in inactive_persistent {
        let vm_id = persistent_entry_vm_id(&entry);
        let (status, can_resume, blocked_reason) = state.persistent_entry_resume_state_cached(&entry);
        sandboxes.push(inactive_sandbox_info(vm_id, &entry, status, can_resume, blocked_reason));
        session_dirs.push(entry.session_dir.clone());
    }

    (ListResponse { sandboxes }, session_dirs)
}

pub(super) async fn handle_list(State(state): State<Arc<ServiceState>>) -> axum::response::Response {
    // The fingerprint stats and hashes files for every inactive entry; the
    // UI polls this route, so it runs off the worker as one unit.
    let (mut response, session_dirs) = match state.off_worker(|state| list_lifecycle(&state)).await {
        Ok(listed) => listed,
        Err(error) => return error.into_response(),
    };
    // Every VM's totals, read together: each is one primary-key lookup,
    // answered from its handle's cache while that ledger has not moved.
    let counters = futures::future::join_all(
        response
            .sandboxes
            .iter()
            .zip(&session_dirs)
            .map(|(info, session_dir)| ledger_routes::activity::counters_if_ready(&state, &info.id, session_dir)),
    )
    .await;
    for (info, counters) in response.sandboxes.iter_mut().zip(counters) {
        if let Some(counters) = counters {
            ledger_routes::activity::apply_totals(info, &counters);
        }
    }
    json_bytes_response(Bytes::from(serde_json::to_vec(&response).unwrap_or_default()))
}

fn list_lifecycle(state: &ServiceState) -> (ListResponse, Vec<PathBuf>) {
    state.reconcile_persistent_defunct_from_logs();
    let fingerprint = list_response_fingerprint(state);
    if let Some(cached) = state.list_response_cache.lock().unwrap().clone() {
        if cached.fingerprint == fingerprint {
            return cached.listed;
        }
    }

    let listed = build_list_response(state);
    *state.list_response_cache.lock().unwrap() = Some(CachedListResponse {
        fingerprint,
        listed: listed.clone(),
    });
    listed
}
