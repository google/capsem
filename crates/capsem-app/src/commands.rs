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

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use capsem_core::net::policy_config::{self, ResolvedSetting, SettingEntry, SettingValue};
use capsem_core::session;
use capsem_core::{HostToGuest, encode_host_msg, validate_host_msg};
use capsem_logger::validate_select_only;
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
pub async fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::debug!("Received serial input: {:?}", input.as_bytes());
    let vm_id = active_vm_id(&state)?;

    let tx = state.terminal_input_tx.clone();
    
    // Extract fd while holding the lock
    let fd = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.vsock_terminal_fd.unwrap_or(instance.serial_input_fd)
    };

    // Send the input to the dedicated background thread.
    // This is instant, non-blocking, and avoids spawning a Tokio thread per keystroke.
    tx.send((fd, input))
        .map_err(|e| format!("send to input queue failed: {e}"))
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
pub async fn terminal_resize(
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
    tokio::task::spawn_blocking(move || {
        file.write_all(&frame)
            .map_err(|e| format!("control write failed: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// ---------------------------------------------------------------------------
// IPC commands for Svelte UI
// ---------------------------------------------------------------------------

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

/// Set a guest env var in ~/.capsem/user.toml via settings system.
#[tauri::command]
pub async fn set_guest_env(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        let setting_id = format!("guest.env.{key}");
        file.settings.insert(setting_id, SettingEntry {
            value: SettingValue::Text(value),
            modified: session::now_iso(),
        });
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Remove a guest env var from ~/.capsem/user.toml via settings system.
#[tauri::command]
pub async fn remove_guest_env(key: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        let setting_id = format!("guest.env.{key}");
        file.settings.remove(&setting_id);
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Returns all resolved settings for the UI.
#[tauri::command]
pub async fn get_settings() -> Result<Vec<ResolvedSetting>, String> {
    tokio::task::spawn_blocking(policy_config::load_merged_settings)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Update a single user setting by ID. Hot-reloads the network policy so
/// changes take effect immediately for new MITM proxy connections.
#[tauri::command]
pub async fn update_setting(id: String, value: SettingValue, app_handle: tauri::AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use tauri::Manager;
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
        let new_domain_policy = std::sync::Arc::new(policy_config::load_merged_domain_policy());
        let state = app_handle.state::<AppState>();
        if let Ok(vm_id) = active_vm_id(&state) {
            let vms = state.vms.lock().unwrap();
            if let Some(instance) = vms.get(&vm_id) {
                if let Some(ns) = &instance.net_state {
                    *ns.policy.write().unwrap() = new_policy;
                    tracing::info!("hot-reloaded network policy");
                }
                // Hot-reload MCP domain policy.
                if let Some(mcp) = &instance.mcp_state {
                    *mcp.domain_policy.write().unwrap() = new_domain_policy;
                    tracing::info!("hot-reloaded MCP domain policy");
                }
            }
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
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
    pub error_requests: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub model_call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_usage_details: std::collections::BTreeMap<String, u64>,
    pub total_tool_calls: u64,
    pub total_estimated_cost_usd: f64,
}

/// Returns info about the current active session.
#[tauri::command]
pub async fn get_session_info(state: State<'_, AppState>) -> Result<SessionInfoResponse, String> {
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
    let (mode, disk_gb, ram) = {
        let idx = state.session_index.lock().map_err(|e| format!("session index lock: {e}"))?;
        let records = idx.recent(50).map_err(|e| format!("session index query: {e}"))?;
        let record = records.iter().find(|r| r.id == vm_id);
        (
            record.map(|r| r.mode.clone()).unwrap_or_else(|| "gui".to_string()),
            record.map(|r| r.scratch_disk_size_gb).unwrap_or(16),
            record.map(|r| r.ram_bytes).unwrap_or(4 * 1024 * 1024 * 1024),
        )
    };

    // Get live stats from session DB via spawn_blocking.
    let db = {
        let vms = state.vms.lock().unwrap();
        vms.get(&vm_id)
            .and_then(|i| i.net_state.as_ref())
            .map(|ns| Arc::clone(&ns.db))
    };

    let stats = if let Some(db) = db {
        tokio::task::spawn_blocking(move || {
            db.reader()
                .ok()
                .and_then(|r| r.session_stats().ok())
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
    } else {
        None
    };

    let vm_id_out = vm_id;
    Ok(SessionInfoResponse {
        session_id: vm_id_out,
        mode,
        uptime_ms,
        scratch_disk_size_gb: disk_gb,
        ram_bytes: ram,
        total_requests: stats.as_ref().map(|s| s.net_total).unwrap_or(0),
        allowed_requests: stats.as_ref().map(|s| s.net_allowed).unwrap_or(0),
        denied_requests: stats.as_ref().map(|s| s.net_denied).unwrap_or(0),
        error_requests: stats.as_ref().map(|s| s.net_error).unwrap_or(0),
        bytes_sent: stats.as_ref().map(|s| s.net_bytes_sent).unwrap_or(0),
        bytes_received: stats.as_ref().map(|s| s.net_bytes_received).unwrap_or(0),
        model_call_count: stats.as_ref().map(|s| s.model_call_count).unwrap_or(0),
        total_input_tokens: stats.as_ref().map(|s| s.total_input_tokens).unwrap_or(0),
        total_output_tokens: stats.as_ref().map(|s| s.total_output_tokens).unwrap_or(0),
        total_usage_details: stats.as_ref().map(|s| s.total_usage_details.clone()).unwrap_or_default(),
        total_tool_calls: stats.as_ref().map(|s| s.total_tool_calls).unwrap_or(0),
        total_estimated_cost_usd: stats.as_ref().map(|s| s.total_estimated_cost_usd).unwrap_or(0.0),
    })
}

// ---------------------------------------------------------------------------
// Raw SQL query
// ---------------------------------------------------------------------------

/// Execute a raw SELECT query against the session DB or main.db.
/// Returns a JSON string: `{"columns":[...],"rows":[[...],...]}`
///
/// - `db`: `"session"` (default) or `"main"` -- which database to query
/// - `params`: optional bind parameter values (`?` positional placeholders)
#[tauri::command]
pub async fn query_db(
    sql: String,
    db: Option<String>,
    params: Option<Vec<serde_json::Value>>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let params = params.unwrap_or_default();
    let target = db.unwrap_or_else(|| "session".to_string());

    match target.as_str() {
        "main" => {
            tokio::task::spawn_blocking(move || {
                use tauri::Manager;
                validate_select_only(&sql)?;
                let state = app_handle.state::<AppState>();
                let idx = state.session_index.lock().map_err(|e| format!("lock: {e}"))?;
                idx.query_raw(&sql, &params)
            })
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?
        }
        _ => {
            let vm_id = active_vm_id(&state)?;
            let db_writer = {
                let vms = state.vms.lock().unwrap();
                let instance = vms.get(&vm_id).ok_or("no VM running")?;
                let net_state = instance.net_state.as_ref().ok_or("network not initialized")?;
                Arc::clone(&net_state.db)
            };

            tokio::task::spawn_blocking(move || {
                validate_select_only(&sql)?;
                let reader = db_writer.reader().map_err(|e| format!("db reader: {e}"))?;
                if params.is_empty() {
                    reader.query_raw(&sql)
                } else {
                    reader.query_raw_with_params(&sql, &params)
                }
            })
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?
        }
    }
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
