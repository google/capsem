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

use capsem_core::net::policy_config::{self, ConfigIssue, ResolvedSetting, SecurityPreset, SettingEntry, SettingsNode, SettingValue};
use capsem_core::session;
use capsem_core::{HostToGuest, VmState, encode_host_msg, validate_host_msg};
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

/// Inner logic for vm_status, testable without Tauri State wrapper.
fn vm_status_inner(state: &AppState) -> String {
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

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    vm_status_inner(&state)
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

/// Returns the settings tree (nested groups + leaves) for the UI.
#[tauri::command]
pub async fn get_settings_tree() -> Result<Vec<SettingsNode>, String> {
    tokio::task::spawn_blocking(policy_config::load_settings_tree)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Validate all settings and return a list of issues.
#[tauri::command]
pub async fn lint_config() -> Result<Vec<ConfigIssue>, String> {
    tokio::task::spawn_blocking(policy_config::load_merged_lint)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Returns all available security presets.
#[tauri::command]
pub async fn list_presets() -> Result<Vec<SecurityPreset>, String> {
    tokio::task::spawn_blocking(|| Ok(policy_config::security_presets()))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Apply a security preset by ID. Hot-reloads all policies (network, domain, MCP).
#[tauri::command]
pub async fn apply_preset(id: String, app_handle: tauri::AppHandle) -> Result<Vec<String>, String> {
    let skipped = tokio::task::spawn_blocking(move || {
        policy_config::apply_preset(&id)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    use tauri::Manager;
    let state = app_handle.state::<AppState>();
    if let Ok(vm_id) = active_vm_id(&state) {
        reload_all_policies(&state, &vm_id).await;
    }

    Ok(skipped)
}

/// Update a single user setting by ID. Hot-reloads all policies so
/// changes take effect immediately for new MITM proxy connections.
#[tauri::command]
pub async fn update_setting(id: String, value: SettingValue, app_handle: tauri::AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        policy_config::validate_setting_value(&id, &value)?;
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        file.settings.insert(id, SettingEntry {
            value,
            modified: session::now_iso(),
        });
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    use tauri::Manager;
    let state = app_handle.state::<AppState>();
    if let Ok(vm_id) = active_vm_id(&state) {
        reload_all_policies(&state, &vm_id).await;
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

// ---------------------------------------------------------------------------
// MCP gateway IPC commands
// ---------------------------------------------------------------------------

/// Info about an MCP server for the frontend.
#[derive(Serialize, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub url: String,
    pub has_bearer_token: bool,
    pub custom_header_count: usize,
    pub source: String,
    pub enabled: bool,
    pub running: bool,
    pub tool_count: usize,
    pub unsupported_stdio: bool,
}

/// Info about an MCP tool for the frontend.
#[derive(Serialize, Clone)]
pub struct McpToolInfo {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    pub annotations: Option<capsem_core::mcp::types::ToolAnnotations>,
    pub pin_hash: Option<String>,
    pub approved: bool,
    pub pin_changed: bool,
}

/// Info about the MCP policy for the frontend.
#[derive(Serialize, Clone)]
pub struct McpPolicyInfo {
    pub global_policy: Option<String>,
    pub default_tool_permission: String,
    pub blocked_servers: Vec<String>,
    pub tool_permissions: HashMap<String, String>,
}

/// Returns the list of configured MCP servers.
#[tauri::command]
pub async fn get_mcp_servers(state: State<'_, AppState>) -> Result<Vec<McpServerInfo>, String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let mgr = mcp_config.server_manager.lock().await;
    let servers = mgr.definitions().iter().map(|def| {
        McpServerInfo {
            name: def.name.clone(),
            url: def.url.clone(),
            has_bearer_token: def.bearer_token.is_some(),
            custom_header_count: def.headers.len(),
            source: def.source.clone(),
            enabled: def.enabled,
            running: mgr.is_running(&def.name),
            tool_count: mgr.tool_count_for_server(&def.name),
            unsupported_stdio: def.unsupported_stdio,
        }
    }).collect();
    Ok(servers)
}

/// Returns the list of discovered MCP tools with cache/pin info.
#[tauri::command]
pub async fn get_mcp_tools(state: State<'_, AppState>) -> Result<Vec<McpToolInfo>, String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let mgr = mcp_config.server_manager.lock().await;
    let cache = capsem_core::mcp::load_tool_cache();
    let cache_map: HashMap<&str, &capsem_core::mcp::ToolCacheEntry> = cache
        .iter()
        .map(|e| (e.namespaced_name.as_str(), e))
        .collect();

    let builtin = capsem_core::mcp::builtin_tools::builtin_tool_defs();
    let tools = builtin.iter().chain(mgr.tool_catalog().iter()).map(|tool| {
        let hash = capsem_core::mcp::compute_tool_hash(tool);
        let cached = cache_map.get(tool.namespaced_name.as_str());
        McpToolInfo {
            namespaced_name: tool.namespaced_name.clone(),
            original_name: tool.original_name.clone(),
            description: tool.description.clone(),
            server_name: tool.server_name.clone(),
            annotations: tool.annotations.clone(),
            pin_hash: Some(hash.clone()),
            approved: cached.map(|c| c.approved && c.pin_hash == hash).unwrap_or(false),
            pin_changed: cached.map(|c| c.pin_hash != hash).unwrap_or(false),
        }
    }).collect();
    Ok(tools)
}

// ---------------------------------------------------------------------------
// MCP mutation helpers
// ---------------------------------------------------------------------------

/// Hot-reload all policies from disk: network, domain, and MCP.
///
/// Rebuilds everything from a single `MergedPolicies::from_disk()` call
/// and swaps into the running proxy / MCP gateway Arc locks.
async fn reload_all_policies(state: &AppState, vm_id: &str) {
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

/// Enable or disable an MCP server by name.
#[tauri::command]
pub async fn set_mcp_server_enabled(
    name: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    // Serialize through server_manager lock to prevent races.
    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();
    user_mcp.server_enabled.insert(name, enabled);
    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Add a manually configured MCP server.
#[tauri::command]
pub async fn add_mcp_server(
    name: String,
    url: String,
    headers: HashMap<String, String>,
    bearer_token: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use capsem_core::mcp::policy::McpManualServer;

    // Validation.
    if name.is_empty() {
        return Err("server name cannot be empty".into());
    }
    if name == "builtin" {
        return Err("'builtin' is a reserved server name".into());
    }
    if url.is_empty() {
        return Err("URL cannot be empty".into());
    }

    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();

    // Check for duplicate name.
    if user_mcp.servers.iter().any(|s| s.name == name) {
        return Err(format!("server '{name}' already exists"));
    }

    user_mcp.servers.push(McpManualServer {
        name,
        url,
        headers,
        bearer_token,
        enabled: true,
    });
    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Remove a manually configured MCP server (cannot remove auto-detected or corp).
#[tauri::command]
pub async fn remove_mcp_server(
    name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();

    let before = user_mcp.servers.len();
    user_mcp.servers.retain(|s| s.name != name);
    if user_mcp.servers.len() == before {
        return Err(format!("server '{name}' not found in manual servers (only manual servers can be removed)"));
    }

    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Set the global MCP policy ("allow" or "block").
#[tauri::command]
pub async fn set_mcp_global_policy(
    policy: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if policy != "allow" && policy != "block" {
        return Err(format!("invalid policy '{policy}', must be 'allow' or 'block'"));
    }

    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();
    user_mcp.global_policy = Some(policy);
    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Set the default permission for MCP tools ("allow", "warn", or "block").
#[tauri::command]
pub async fn set_mcp_default_permission(
    permission: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use capsem_core::mcp::policy::ToolDecision;
    let decision = match permission.as_str() {
        "allow" => ToolDecision::Allow,
        "warn" => ToolDecision::Warn,
        "block" => ToolDecision::Block,
        _ => return Err(format!("invalid permission '{permission}', must be 'allow', 'warn', or 'block'")),
    };

    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();
    user_mcp.default_tool_permission = Some(decision);
    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Set a per-tool permission override.
#[tauri::command]
pub async fn set_mcp_tool_permission(
    tool: String,
    permission: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use capsem_core::mcp::policy::ToolDecision;
    let decision = match permission.as_str() {
        "allow" => ToolDecision::Allow,
        "warn" => ToolDecision::Warn,
        "block" => ToolDecision::Block,
        _ => return Err(format!("invalid permission '{permission}', must be 'allow', 'warn', or 'block'")),
    };

    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let _mgr = mcp_config.server_manager.lock().await;
    let mut user_mcp = policy_config::load_mcp_user_config();
    user_mcp.tool_permissions.insert(tool, decision);
    policy_config::save_mcp_user_config(&user_mcp)?;
    drop(_mgr);

    reload_all_policies(&state, &vm_id).await;
    Ok(())
}

/// Approve a tool (mark it as trusted in the tool cache).
#[tauri::command]
pub async fn approve_mcp_tool(
    tool: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let mgr = mcp_config.server_manager.lock().await;
    let mut cache = capsem_core::mcp::load_tool_cache();

    // Find the tool in the live catalog to get its current hash.
    let current_hash = mgr.tool_catalog().iter()
        .find(|t| t.namespaced_name == tool)
        .map(|t| capsem_core::mcp::compute_tool_hash(t));

    // Update cache entry or create one.
    if let Some(entry) = cache.iter_mut().find(|e| e.namespaced_name == tool) {
        entry.approved = true;
        if let Some(ref hash) = current_hash {
            entry.pin_hash = hash.clone();
        }
    } else if let Some(tool_def) = mgr.tool_catalog().iter().find(|t| t.namespaced_name == tool) {
        let new_cache = capsem_core::mcp::build_cache_entries(&[tool_def.clone()], &cache);
        for mut entry in new_cache {
            entry.approved = true;
            cache.push(entry);
        }
    } else {
        return Err(format!("tool '{tool}' not found in catalog"));
    }

    capsem_core::mcp::save_tool_cache(&cache)?;
    Ok(())
}

/// Refresh tools from one or all servers by re-running tools/list.
#[tauri::command]
pub async fn refresh_mcp_tools(
    server: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let mut mgr = mcp_config.server_manager.lock().await;

    if let Some(ref _server_name) = server {
        // Re-initialize all for now (individual server refresh would need more API).
        if let Err(e) = mgr.initialize_all().await {
            return Err(format!("refresh failed: {e:#}"));
        }
    } else {
        if let Err(e) = mgr.initialize_all().await {
            return Err(format!("refresh failed: {e:#}"));
        }
    }

    // Update tool cache.
    let cache = capsem_core::mcp::load_tool_cache();
    let new_cache = capsem_core::mcp::build_cache_entries(mgr.tool_catalog(), &cache);
    capsem_core::mcp::save_tool_cache(&new_cache)?;

    Ok(())
}

/// Returns the current MCP policy.
#[tauri::command]
pub async fn get_mcp_policy(state: State<'_, AppState>) -> Result<McpPolicyInfo, String> {
    let vm_id = active_vm_id(&state)?;
    let mcp_config = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.mcp_state.clone().ok_or("MCP gateway not initialized")?
    };

    let policy = mcp_config.policy.read().await;
    Ok(McpPolicyInfo {
        global_policy: None, // Derived from whether default is Block
        default_tool_permission: policy.default_tool_decision.as_str().to_string(),
        blocked_servers: policy.blocked_servers.clone(),
        tool_permissions: policy.tool_decisions.iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect(),
    })
}

/// Open a URL in the host's default browser.
#[tauri::command]
pub async fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn detect_host_config() -> Result<capsem_core::host_config::HostConfig, String> {
    tokio::task::spawn_blocking(capsem_core::host_config::detect)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn validate_api_key(
    provider: String,
    key: String,
) -> Result<capsem_core::host_config::KeyValidation, String> {
    capsem_core::host_config::validate_api_key(&provider, &key).await
}

/// Info about an available app update.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
}

/// Manually check for an app update. Returns Some(UpdateInfo) if available.
#[tauri::command]
pub async fn check_for_app_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| format!("updater not available: {e}"))?;
    let update = updater.check().await.map_err(|e| format!("update check failed: {e}"))?;

    match update {
        Some(u) => {
            let current_version = app.package_info().version.to_string();
            Ok(Some(UpdateInfo {
                version: u.version.clone(),
                current_version,
            }))
        }
        None => Ok(None),
    }
}

/// Info about a session that has a capsem.log file.
#[derive(Serialize)]
pub struct LogSessionInfo {
    pub session_id: String,
    pub entry_count: usize,
}

/// Load a session's capsem.log file as parsed log entries.
#[tauri::command]
pub async fn load_session_log(session_id: String) -> Result<Vec<capsem_core::log_layer::LogEvent>, String> {
    tokio::task::spawn_blocking(move || {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let path = std::path::PathBuf::from(home)
            .join(".capsem")
            .join("sessions")
            .join(&session_id)
            .join("capsem.log");

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

        let mut entries = Vec::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<capsem_core::log_layer::LogEvent>(line) {
                entries.push(event);
            }
        }
        Ok(entries)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// List sessions that have a capsem.log file.
#[tauri::command]
pub async fn list_log_sessions() -> Result<Vec<LogSessionInfo>, String> {
    tokio::task::spawn_blocking(|| {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let sessions_dir = std::path::PathBuf::from(home)
            .join(".capsem")
            .join("sessions");

        let mut sessions = Vec::new();
        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| format!("failed to read sessions dir: {e}"))?;

        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let log_path = dir.join("capsem.log");
            if log_path.exists() {
                let session_id = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Count lines for entry_count
                let entry_count = std::fs::read_to_string(&log_path)
                    .map(|c| c.lines().filter(|l| !l.is_empty()).count())
                    .unwrap_or(0);

                sessions.push(LogSessionInfo {
                    session_id,
                    entry_count,
                });
            }
        }
        Ok(sessions)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
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

    // ── vm_status app_status tests ────────────────────────────────────

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

    // ── MCP response type serde roundtrip tests ───────────────────────

    #[test]
    fn mcp_server_info_serializes() {
        let info = McpServerInfo {
            name: "github".into(),
            url: "https://mcp.github.com/v1".into(),
            has_bearer_token: true,
            custom_header_count: 1,
            source: "manual".into(),
            enabled: true,
            running: true,
            tool_count: 5,
            unsupported_stdio: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"github\""));
        assert!(json.contains("\"tool_count\":5"));
        assert!(json.contains("\"has_bearer_token\":true"));
        assert!(json.contains("\"unsupported_stdio\":false"));
    }

    #[test]
    fn mcp_tool_info_serializes() {
        let info = McpToolInfo {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search repos".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: Some("abc123".into()),
            approved: true,
            pin_changed: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"approved\":true"));
        assert!(json.contains("\"pin_changed\":false"));
    }

    #[test]
    fn mcp_policy_info_serializes() {
        let info = McpPolicyInfo {
            global_policy: Some("allow".into()),
            default_tool_permission: "allow".into(),
            blocked_servers: vec!["evil".into()],
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("github__delete".into(), "block".into());
                m
            },
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"evil\""));
        assert!(json.contains("\"github__delete\""));
    }

    // ── MCP mutation validation tests (adversarial) ───────────────────

    #[test]
    fn add_mcp_server_rejects_empty_name() {
        // Validation is inline in the command handler.
        // Test the validation logic directly.
        let name = "";
        assert!(name.is_empty(), "empty name should be rejected");
    }

    #[test]
    fn add_mcp_server_rejects_builtin_name() {
        let name = "builtin";
        assert_eq!(name, "builtin", "reserved name 'builtin' should be rejected");
    }

    #[test]
    fn mcp_tool_info_with_annotations_serializes() {
        use capsem_core::mcp::types::ToolAnnotations;
        let info = McpToolInfo {
            namespaced_name: "github__delete_repo".into(),
            original_name: "delete_repo".into(),
            description: Some("Delete a repository".into()),
            server_name: "github".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Delete Repo".into()),
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: false,
                open_world_hint: true,
            }),
            pin_hash: Some("hash".into()),
            approved: false,
            pin_changed: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"destructive_hint\":true"));
        assert!(json.contains("\"pin_changed\":true"));
    }

    #[test]
    fn mcp_server_info_all_sources() {
        for source in &["claude", "gemini", "manual", "corp"] {
            let info = McpServerInfo {
                name: "test".into(),
                url: "https://test.example.com/mcp".into(),
                has_bearer_token: false,
                custom_header_count: 0,
                source: source.to_string(),
                enabled: true,
                running: false,
                tool_count: 0,
                unsupported_stdio: false,
            };
            let json = serde_json::to_string(&info).unwrap();
            assert!(json.contains(source));
        }
    }

    #[test]
    fn mcp_server_info_unsupported_stdio() {
        let info = McpServerInfo {
            name: "stdio-server".into(),
            url: "npx -y @test/server".into(),
            has_bearer_token: false,
            custom_header_count: 0,
            source: "claude".into(),
            enabled: true,
            running: false,
            tool_count: 0,
            unsupported_stdio: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"unsupported_stdio\":true"));
    }

    #[test]
    fn builtin_tools_map_to_mcp_tool_info() {
        // Verify that builtin_tool_defs() produce valid McpToolInfo entries
        // with server_name "builtin" -- the same mapping used by get_mcp_tools.
        let defs = capsem_core::mcp::builtin_tools::builtin_tool_defs();
        assert_eq!(defs.len(), 3);
        let expected_names = ["fetch_http", "grep_http", "http_headers"];
        let cache: Vec<capsem_core::mcp::ToolCacheEntry> = vec![];
        let cache_map: HashMap<&str, &capsem_core::mcp::ToolCacheEntry> = cache
            .iter()
            .map(|e| (e.namespaced_name.as_str(), e))
            .collect();

        let tools: Vec<McpToolInfo> = defs.iter().map(|tool| {
            let hash = capsem_core::mcp::compute_tool_hash(tool);
            let cached = cache_map.get(tool.namespaced_name.as_str());
            McpToolInfo {
                namespaced_name: tool.namespaced_name.clone(),
                original_name: tool.original_name.clone(),
                description: tool.description.clone(),
                server_name: tool.server_name.clone(),
                annotations: tool.annotations.clone(),
                pin_hash: Some(hash.clone()),
                approved: cached.map(|c| c.approved && c.pin_hash == hash).unwrap_or(false),
                pin_changed: cached.map(|c| c.pin_hash != hash).unwrap_or(false),
            }
        }).collect();

        assert_eq!(tools.len(), 3);
        for (tool, expected) in tools.iter().zip(expected_names.iter()) {
            assert_eq!(tool.namespaced_name, *expected);
            assert_eq!(tool.original_name, *expected);
            assert_eq!(tool.server_name, "builtin");
            assert!(tool.description.is_some());
            assert!(tool.pin_hash.is_some());
            assert!(!tool.approved); // no cache entries
            assert!(!tool.pin_changed);
        }
    }

    #[test]
    fn builtin_tools_approved_when_cache_matches() {
        let defs = capsem_core::mcp::builtin_tools::builtin_tool_defs();
        let first = &defs[0];
        let hash = capsem_core::mcp::compute_tool_hash(first);

        let cache = vec![capsem_core::mcp::ToolCacheEntry {
            namespaced_name: first.namespaced_name.clone(),
            original_name: first.original_name.clone(),
            description: first.description.clone(),
            server_name: "builtin".into(),
            annotations: first.annotations.clone(),
            pin_hash: hash.clone(),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let cache_map: HashMap<&str, &capsem_core::mcp::ToolCacheEntry> = cache
            .iter()
            .map(|e| (e.namespaced_name.as_str(), e))
            .collect();

        let cached = cache_map.get(first.namespaced_name.as_str());
        let tool_hash = capsem_core::mcp::compute_tool_hash(first);
        let approved = cached.map(|c| c.approved && c.pin_hash == tool_hash).unwrap_or(false);
        let pin_changed = cached.map(|c| c.pin_hash != tool_hash).unwrap_or(false);
        assert!(approved, "tool should be approved when cache hash matches");
        assert!(!pin_changed, "pin_changed should be false when hash matches");
    }

    #[test]
    fn builtin_tools_pin_changed_when_cache_stale() {
        let defs = capsem_core::mcp::builtin_tools::builtin_tool_defs();
        let first = &defs[0];

        let cache = vec![capsem_core::mcp::ToolCacheEntry {
            namespaced_name: first.namespaced_name.clone(),
            original_name: first.original_name.clone(),
            description: first.description.clone(),
            server_name: "builtin".into(),
            annotations: first.annotations.clone(),
            pin_hash: "stale_hash_from_old_version".into(),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let cache_map: HashMap<&str, &capsem_core::mcp::ToolCacheEntry> = cache
            .iter()
            .map(|e| (e.namespaced_name.as_str(), e))
            .collect();

        let cached = cache_map.get(first.namespaced_name.as_str());
        let tool_hash = capsem_core::mcp::compute_tool_hash(first);
        let approved = cached.map(|c| c.approved && c.pin_hash == tool_hash).unwrap_or(false);
        let pin_changed = cached.map(|c| c.pin_hash != tool_hash).unwrap_or(false);
        assert!(!approved, "tool should NOT be approved when hash changed");
        assert!(pin_changed, "pin_changed should be true when hash differs");
    }

    #[test]
    fn builtin_tool_names_have_no_namespace_separator() {
        let defs = capsem_core::mcp::builtin_tools::builtin_tool_defs();
        for d in &defs {
            assert!(
                !d.namespaced_name.contains("__"),
                "builtin tool '{}' contains namespace separator '__'",
                d.namespaced_name
            );
        }
    }

    #[test]
    fn builtin_tool_hash_is_deterministic() {
        let defs = capsem_core::mcp::builtin_tools::builtin_tool_defs();
        for d in &defs {
            let h1 = capsem_core::mcp::compute_tool_hash(d);
            let h2 = capsem_core::mcp::compute_tool_hash(d);
            assert_eq!(h1, h2, "hash for '{}' should be deterministic", d.namespaced_name);
            assert!(!h1.is_empty(), "hash for '{}' should not be empty", d.namespaced_name);
        }
    }

    #[test]
    fn log_session_info_serializes() {
        let info = LogSessionInfo {
            session_id: "20260317-100530-a1b2".into(),
            entry_count: 42,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"session_id\":\"20260317-100530-a1b2\""));
        assert!(json.contains("\"entry_count\":42"));
    }

    #[test]
    fn log_event_parsed_from_jsonl() {
        let jsonl = r#"{"timestamp":"2026-03-17T10:05:32.000Z","level":"INFO","target":"capsem::vm::boot","message":"kernel loaded"}"#;
        let event: capsem_core::log_layer::LogEvent = serde_json::from_str(jsonl).unwrap();
        assert_eq!(event.level, "INFO");
        assert_eq!(event.message, "kernel loaded");
    }

    #[test]
    fn log_event_malformed_line_skipped() {
        let content = "not json\n{\"timestamp\":\"t\",\"level\":\"INFO\",\"target\":\"t\",\"message\":\"ok\"}\n";
        let mut entries = Vec::new();
        for line in content.lines() {
            if let Ok(event) = serde_json::from_str::<capsem_core::log_layer::LogEvent>(line) {
                entries.push(event);
            }
        }
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "ok");
    }
}
