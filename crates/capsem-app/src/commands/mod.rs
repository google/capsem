//! Tauri IPC Commands
//!
//! **CRITICAL SAFETY RULE: Never perform blocking I/O in synchronous Tauri commands.**
//!
//! Tauri processes synchronous commands in a limited thread pool. If a synchronous command
//! performs a blocking operation (e.g., `file.write_all()` to a vsock descriptor whose buffer
//! might be full, or a `rusqlite` database query), it can exhaust this pool. This causes the
//! entire application UI and IPC channel to completely freeze ("barf" or lag on rapid input).
//!
//! **Always** define commands that do I/O as `async fn` and wrap the blocking portions inside
//! `tokio::task::spawn_blocking`. This offloads the work to Tokio's dedicated background
//! thread pool, keeping the Tauri IPC channels responsive.

mod logging;
mod mcp;
mod session;
mod settings;
mod terminal;
mod utilities;
mod vm_state;

pub use logging::*;
pub use mcp::*;
pub use session::*;
pub use settings::*;
pub use terminal::*;
pub use utilities::*;
pub use vm_state::*;

use std::sync::Arc;

use capsem_core::net::policy_config;
use capsem_core::VmState;

use crate::state::AppState;

/// Get the active VM ID from app state, or return an error.
pub(crate) fn active_vm_id(state: &AppState) -> Result<String, String> {
    state
        .active_session_id
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no active session".to_string())
}

/// Inner logic for vm_status, testable without Tauri State wrapper.
pub(crate) fn vm_status_inner(state: &AppState) -> String {
    // Check app-level status first. "downloading" happens before any VM
    // instance exists, so the per-VM lookup below would return "not created".
    let app_status = state.app_status.lock().unwrap().clone();
    if app_status == VmState::Downloading.as_str() {
        return app_status;
    }

    let vm_id = match active_vm_id(state) {
        Ok(id) => id,
        Err(_) => return VmState::NotCreated.to_string(),
    };
    let vms = state.vms.lock().unwrap();
    match vms.get(&vm_id) {
        Some(instance) => format!("{}", instance.state_machine.state()),
        None => VmState::NotCreated.to_string(),
    }
}

/// Hot-reload all policies from disk: network, domain, and MCP.
///
/// Rebuilds everything from a single `MergedPolicies::from_disk()` call
/// and swaps into the running proxy / MCP gateway Arc locks.
pub(crate) async fn reload_all_policies(state: &AppState, vm_id: &str) {
    let policies = policy_config::MergedPolicies::from_disk();
    let (net_lock, mcp_config) = {
        let vms = state.vms.lock().unwrap();
        let Some(inst) = vms.get(vm_id) else { return };
        (
            inst.net_state.as_ref().map(|ns| ns.policy.clone()),
            inst.mcp_state.clone(),
        )
    };
    if let Some(lock) = net_lock {
        *lock.write().unwrap() = Arc::new(policies.network);
        tracing::info!("hot-reloaded network policy");
    }
    if let Some(mcp) = mcp_config {
        *mcp.domain_policy.write().unwrap() = Arc::new(policies.domain);
        *mcp.policy.write().await = Arc::new(policies.mcp);
        tracing::info!("hot-reloaded MCP + domain policy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capsem_core::session::SessionIndex;

    #[test]
    fn active_vm_id_returns_error_when_none() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx, None);
        let result = active_vm_id(&state);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "no active session");
    }

    #[test]
    fn active_vm_id_returns_id_when_set() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx, None);
        *state.active_session_id.lock().unwrap() = Some("20260225-143052-a7f3".to_string());
        let result = active_vm_id(&state);
        assert_eq!(result.unwrap(), "20260225-143052-a7f3");
    }

    // -- vm_status app_status tests --

    #[test]
    fn vm_status_returns_downloading_when_app_status_is_downloading() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx, None);
        *state.app_status.lock().unwrap() = VmState::Downloading.to_string();
        // No VM instances exist, but app_status should take precedence.
        let status = vm_status_inner(&state);
        assert_eq!(status, VmState::Downloading.as_str());
    }

    #[test]
    fn vm_status_returns_not_created_by_default() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx, None);
        let status = vm_status_inner(&state);
        assert_eq!(status, VmState::NotCreated.as_str());
    }

    #[test]
    fn vm_status_falls_through_when_app_status_is_booting() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx, None);
        *state.app_status.lock().unwrap() = VmState::Booting.to_string();
        // No VM exists, so falls through to "not created".
        // In production a VM would exist and return its actual state.
        let status = vm_status_inner(&state);
        assert_eq!(status, VmState::NotCreated.as_str());
    }
}
