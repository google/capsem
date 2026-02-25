use std::collections::HashMap;
use std::io::Write;

use capsem_core::net::policy_config;
use capsem_core::net::telemetry::NetEvent;
use capsem_core::{HostToGuest, encode_host_msg, validate_host_msg};
use serde::Serialize;
use tauri::State;

use crate::clone_fd;
use crate::state::AppState;

/// Default VM ID for the single-VM case.
const DEFAULT_VM_ID: &str = "default";

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    let vms = state.vms.lock().unwrap();
    match vms.get(DEFAULT_VM_ID) {
        Some(instance) => format!("{}", instance.state_machine.state()),
        None => "not created".to_string(),
    }
}

#[tauri::command]
pub fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::debug!("Received serial input: {:?}", input.as_bytes());
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;

    // Prefer vsock terminal if connected, fall back to serial.
    let fd = instance.vsock_terminal_fd.unwrap_or(instance.serial_input_fd);

    let mut file = clone_fd(fd)
        .map_err(|e| format!("clone fd failed: {e}"))?;
    file.write_all(input.as_bytes())
        .map_err(|e| format!("write failed: {e}"))
}

#[tauri::command]
pub fn terminal_resize(
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;

    let control_fd = instance.vsock_control_fd.ok_or("vsock control not connected")?;

    let msg = HostToGuest::Resize { cols, rows };
    validate_host_msg(&msg, instance.state_machine.state())
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
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;
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
    let (_user, corp) = policy_config::load_policy_files();
    let corp_managed = corp.network.is_some();
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

/// Set a guest env var in ~/.capsem/user.toml [guest].env.
#[tauri::command]
pub fn set_guest_env(key: String, value: String) -> Result<(), String> {
    let path = policy_config::user_config_path()
        .ok_or("HOME not set")?;
    let mut policy = policy_config::load_policy_file(&path)?;
    let guest = policy.guest.get_or_insert_with(Default::default);
    let env = guest.env.get_or_insert_with(Default::default);
    env.insert(key, value);
    policy_config::write_policy_file(&path, &policy)
}

/// Remove a guest env var from ~/.capsem/user.toml [guest].env.
#[tauri::command]
pub fn remove_guest_env(key: String) -> Result<(), String> {
    let path = policy_config::user_config_path()
        .ok_or("HOME not set")?;
    let mut policy = policy_config::load_policy_file(&path)?;
    if let Some(ref mut guest) = policy.guest {
        if let Some(ref mut env) = guest.env {
            env.remove(&key);
        }
    }
    policy_config::write_policy_file(&path, &policy)
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

/// Returns full state machine info for the default VM.
#[tauri::command]
pub fn get_vm_state(state: State<'_, AppState>) -> Result<VmStateResponse, String> {
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;
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
