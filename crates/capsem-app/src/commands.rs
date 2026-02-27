use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use capsem_core::net::policy_config::{self, ResolvedSetting, SettingEntry, SettingValue};
use capsem_core::net::telemetry::NetEvent;
use capsem_core::session::{self, SessionRecord};
use capsem_core::{HostToGuest, encode_host_msg, validate_host_msg};
use serde::Serialize;
use tauri::State;

use crate::clone_fd;
use crate::state::AppState;

/// Get the active VM ID from app state, or return an error.
fn active_vm_id(state: &AppState) -> Result<String, String> {
    state
        .active_session_id
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no active session".to_string())
}

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    let vm_id = match active_vm_id(&state) {
        Ok(id) => id,
        Err(_) => return "not created".to_string(),
    };
    let vms = state.vms.lock().unwrap();
    match vms.get(&vm_id) {
        Some(instance) => format!("{}", instance.state_machine.state()),
        None => "not created".to_string(),
    }
}

#[tauri::command]
pub fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::debug!("Received serial input: {:?}", input.as_bytes());
    let vm_id = active_vm_id(&state)?;

    // Extract fd while holding the lock, then release before the blocking write.
    // Holding the vms mutex during write_all() would block ALL other IPC commands
    // (vm_status, terminal_resize, etc.) if the vsock buffer is full.
    let fd = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.vsock_terminal_fd.unwrap_or(instance.serial_input_fd)
    };

    let mut file = clone_fd(fd)
        .map_err(|e| format!("clone fd failed: {e}"))?;
    file.write_all(input.as_bytes())
        .map_err(|e| format!("write failed: {e}"))
}

/// Poll for terminal output data. Blocks until data is available or the
/// terminal is closed. Returns bytes as a JSON array (Tauri serialization).
#[tauri::command]
pub async fn terminal_poll(state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    let queue = Arc::clone(&state.terminal_output);
    queue.poll().await
        .ok_or_else(|| "terminal closed".to_string())
}

#[tauri::command]
pub fn terminal_resize(
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;

    // Extract fd and state while holding the lock, then release before writing.
    // Same pattern as serial_input: avoid holding the mutex during blocking I/O.
    let (control_fd, host_state) = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        let fd = instance.vsock_control_fd.ok_or("vsock control not connected")?;
        (fd, instance.state_machine.state())
    };

    let msg = HostToGuest::Resize { cols, rows };
    validate_host_msg(&msg, host_state)
        .map_err(|e| format!("{e}"))?;
    let frame = encode_host_msg(&msg).map_err(|e| format!("{e}"))?;

    let mut file = clone_fd(control_fd)
        .map_err(|e| format!("clone control fd failed: {e}"))?;
    file.write_all(&frame)
        .map_err(|e| format!("control write failed: {e}"))
}

/// Query the most recent N network events from the VM's web.db.
#[tauri::command]
pub fn net_events(limit: Option<usize>, state: State<'_, AppState>) -> Result<Vec<NetEvent>, String> {
    let vm_id = active_vm_id(&state)?;
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(&vm_id).ok_or("no VM running")?;
    let net_state = instance.net_state.as_ref().ok_or("network not initialized")?;
    let db = net_state.web_db.lock().map_err(|e| format!("web.db lock: {e}"))?;
    db.recent(limit.unwrap_or(100)).map_err(|e| format!("web.db query: {e}"))
}

// ---------------------------------------------------------------------------
// New IPC commands for Svelte UI
// ---------------------------------------------------------------------------

/// Response for get_guest_config.
#[derive(Serialize)]
pub struct GuestConfigResponse {
    pub env: HashMap<String, String>,
}

/// Returns merged guest config (env vars from user.toml + corp.toml). No VM required.
#[tauri::command]
pub fn get_guest_config() -> GuestConfigResponse {
    let config = policy_config::load_merged_guest_config();
    GuestConfigResponse {
        env: config.env.unwrap_or_default(),
    }
}

/// Response for get_network_policy.
#[derive(Serialize)]
pub struct NetworkPolicyResponse {
    pub allow: Vec<String>,
    pub block: Vec<String>,
    pub default_action: String,
    pub corp_managed: bool,
}

/// Returns the merged network policy. No VM required.
#[tauri::command]
pub fn get_network_policy() -> NetworkPolicyResponse {
    let (_user, corp) = policy_config::load_settings_files();
    let corp_managed = !corp.settings.is_empty();
    let policy = policy_config::load_merged_policy();
    let dp = policy.domain_policy();
    // Probe the default action by evaluating a domain that won't match any rule.
    let (default_act, _) = dp.evaluate("__capsem_probe_nonexistent__.invalid");
    NetworkPolicyResponse {
        allow: dp.allowed_patterns(),
        block: dp.blocked_patterns(),
        default_action: if default_act == capsem_core::net::domain_policy::Action::Allow {
            "allow".to_string()
        } else {
            "deny".to_string()
        },
        corp_managed,
    }
}

/// Set a guest env var in ~/.capsem/user.toml via settings system.
#[tauri::command]
pub fn set_guest_env(key: String, value: String) -> Result<(), String> {
    let path = policy_config::user_config_path()
        .ok_or("HOME not set")?;
    let mut file = policy_config::load_settings_file(&path)?;
    let setting_id = format!("guest.env.{key}");
    file.settings.insert(setting_id, SettingEntry {
        value: SettingValue::Text(value),
        modified: session::now_iso(),
    });
    policy_config::write_settings_file(&path, &file)
}

/// Remove a guest env var from ~/.capsem/user.toml via settings system.
#[tauri::command]
pub fn remove_guest_env(key: String) -> Result<(), String> {
    let path = policy_config::user_config_path()
        .ok_or("HOME not set")?;
    let mut file = policy_config::load_settings_file(&path)?;
    let setting_id = format!("guest.env.{key}");
    file.settings.remove(&setting_id);
    policy_config::write_settings_file(&path, &file)
}

/// Returns all resolved settings for the UI.
#[tauri::command]
pub fn get_settings() -> Result<Vec<ResolvedSetting>, String> {
    Ok(policy_config::load_merged_settings())
}

/// Update a single user setting by ID. Hot-reloads the network policy so
/// changes take effect immediately for new MITM proxy connections.
#[tauri::command]
pub fn update_setting(id: String, value: SettingValue, state: State<'_, AppState>) -> Result<(), String> {
    // Validate before persisting (e.g. JSON check for File-type settings).
    policy_config::validate_setting_value(&id, &value)?;

    let path = policy_config::user_config_path()
        .ok_or("HOME not set")?;
    let mut file = policy_config::load_settings_file(&path)?;
    file.settings.insert(id, SettingEntry {
        value,
        modified: session::now_iso(),
    });
    policy_config::write_settings_file(&path, &file)?;

    // Hot-reload: rebuild the network policy from disk and swap it into the
    // running MITM proxy. New connections will use the updated policy.
    let new_policy = std::sync::Arc::new(policy_config::load_merged_network_policy());
    if let Ok(vm_id) = active_vm_id(&state) {
        let vms = state.vms.lock().unwrap();
        if let Some(instance) = vms.get(&vm_id) {
            if let Some(ns) = &instance.net_state {
                *ns.policy.write().unwrap() = new_policy;
                tracing::info!("hot-reloaded network policy");
            }
        }
    }

    Ok(())
}

/// Response for get_vm_state.
#[derive(Serialize)]
pub struct VmStateResponse {
    pub state: String,
    pub elapsed_ms: u64,
    pub history: Vec<TransitionEntry>,
}

/// A single transition in the state machine history.
#[derive(Serialize)]
pub struct TransitionEntry {
    pub from: String,
    pub to: String,
    pub trigger: String,
    pub duration_ms: f64,
}

/// Returns full state machine info for the active VM.
#[tauri::command]
pub fn get_vm_state(state: State<'_, AppState>) -> Result<VmStateResponse, String> {
    let vm_id = active_vm_id(&state)?;
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(&vm_id).ok_or("no VM running")?;
    let sm = &instance.state_machine;
    Ok(VmStateResponse {
        state: format!("{}", sm.state()),
        elapsed_ms: sm.elapsed().as_millis() as u64,
        history: sm.history().iter().map(|t| TransitionEntry {
            from: format!("{:?}", t.from),
            to: format!("{:?}", t.to),
            trigger: t.trigger.to_string(),
            duration_ms: t.duration_in_from.as_secs_f64() * 1000.0,
        }).collect(),
    })
}

// ---------------------------------------------------------------------------
// Session info commands
// ---------------------------------------------------------------------------

/// Response for get_session_info.
#[derive(Serialize)]
pub struct SessionInfoResponse {
    pub session_id: String,
    pub mode: String,
    pub uptime_ms: u64,
    pub scratch_disk_size_gb: u32,
    pub ram_bytes: u64,
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
}

/// Returns info about the current active session.
#[tauri::command]
pub fn get_session_info(state: State<'_, AppState>) -> Result<SessionInfoResponse, String> {
    let vm_id = active_vm_id(&state)?;

    // Get uptime from state machine.
    let uptime_ms = {
        let vms = state.vms.lock().unwrap();
        match vms.get(&vm_id) {
            Some(instance) => instance.state_machine.elapsed().as_millis() as u64,
            None => 0,
        }
    };

    // Get session record from index.
    let idx = state.session_index.lock().map_err(|e| format!("session index lock: {e}"))?;
    let records = idx.recent(50).map_err(|e| format!("session index query: {e}"))?;
    let record = records.iter().find(|r| r.id == vm_id);

    // Get live request counts from web.db if available.
    let (total, allowed, denied) = {
        let vms = state.vms.lock().unwrap();
        if let Some(instance) = vms.get(&vm_id) {
            if let Some(ns) = &instance.net_state {
                let db = ns.web_db.lock().map_err(|e| format!("web.db lock: {e}"))?;
                db.count_by_decision().map_err(|e| format!("web.db count: {e}"))?
            } else {
                (0, 0, 0)
            }
        } else {
            (0, 0, 0)
        }
    };

    Ok(SessionInfoResponse {
        session_id: vm_id,
        mode: record.map(|r| r.mode.clone()).unwrap_or_else(|| "gui".to_string()),
        uptime_ms,
        scratch_disk_size_gb: record.map(|r| r.scratch_disk_size_gb).unwrap_or(8),
        ram_bytes: record.map(|r| r.ram_bytes).unwrap_or(512 * 1024 * 1024),
        total_requests: total as u64,
        allowed_requests: allowed as u64,
        denied_requests: denied as u64,
    })
}

/// Returns session history from main.db.
#[tauri::command]
pub fn get_session_history(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    let idx = state.session_index.lock().map_err(|e| format!("session index lock: {e}"))?;
    idx.recent(limit.unwrap_or(50))
        .map_err(|e| format!("session index query: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use capsem_core::session::SessionIndex;

    #[test]
    fn active_vm_id_returns_error_when_none() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        let result = active_vm_id(&state);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "no active session");
    }

    #[test]
    fn active_vm_id_returns_id_when_set() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let state = AppState::new(idx);
        *state.active_session_id.lock().unwrap() = Some("20260225-143052-a7f3".to_string());
        let result = active_vm_id(&state);
        assert_eq!(result.unwrap(), "20260225-143052-a7f3");
    }
}
