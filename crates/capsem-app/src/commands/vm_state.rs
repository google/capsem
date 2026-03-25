use std::collections::HashMap;

use capsem_core::net::policy_config;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;
use super::active_vm_id;

/// Response for get_guest_config.
#[derive(Serialize)]
pub struct GuestConfigResponse {
    pub env: HashMap<String, String>,
}

/// Returns merged guest config (env vars from user.toml + corp.toml). No VM required.
#[tauri::command]
pub async fn get_guest_config() -> Result<GuestConfigResponse, String> {
    tokio::task::spawn_blocking(|| {
        let config = policy_config::load_merged_guest_config();
        GuestConfigResponse {
            env: config.env.unwrap_or_default(),
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Response for get_network_policy.
#[derive(Serialize)]
pub struct NetworkPolicyResponse {
    pub allow: Vec<String>,
    pub block: Vec<String>,
    pub default_action: String,
    pub corp_managed: bool,
    pub conflicts: Vec<String>,
}

/// Returns the merged network policy. No VM required.
#[tauri::command]
pub async fn get_network_policy() -> Result<NetworkPolicyResponse, String> {
    tokio::task::spawn_blocking(|| {
        let (_user, corp) = policy_config::load_settings_files();
        let corp_managed = !corp.settings.is_empty();
        let policy = policy_config::load_merged_policy();
        let dp = policy.domain_policy();
        // Probe the default action by evaluating a domain that won't match any rule.
        let (default_act, _) = dp.evaluate("__capsem_probe_nonexistent__.invalid");
        let allow_set = dp.allowed_patterns();
        let block_set = dp.blocked_patterns();
        let conflicts: Vec<String> = allow_set.iter()
            .filter(|d| block_set.contains(d))
            .cloned()
            .collect();
        NetworkPolicyResponse {
            allow: allow_set,
            block: block_set,
            default_action: if default_act == capsem_core::net::domain_policy::Action::Allow {
                "allow".to_string()
            } else {
                "deny".to_string()
            },
            corp_managed,
            conflicts,
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
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
