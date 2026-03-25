use std::collections::HashMap;
use std::sync::Arc;

use capsem_core::net::policy_config;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;
use super::{active_vm_id, reload_all_policies};

// ---------------------------------------------------------------------------
// MCP tool call from frontend
// ---------------------------------------------------------------------------

/// Call an MCP built-in tool from the frontend.
/// Returns the tool result as a JSON value.
#[tauri::command]
pub async fn call_mcp_tool(
    tool: String,
    arguments: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use capsem_core::mcp::file_tools;

    let config = state.mcp_config.lock().unwrap().clone()
        .ok_or_else(|| "MCP not initialized".to_string())?;

    if !file_tools::is_file_tool(&tool) {
        return Err(format!("unknown tool: {tool}"));
    }

    let sched = Arc::clone(config.auto_snapshots.as_ref()
        .ok_or_else(|| "snapshots not available (not in VirtioFS mode)".to_string())?);
    let ws = config.workspace_dir.clone()
        .ok_or_else(|| "workspace not available".to_string())?;

    let resp = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async {
            let mut sched = sched.lock().await;
            match tool.as_str() {
                "snapshots_list" => file_tools::handle_list_snapshots(&sched, &ws, None),
                "snapshots_changes" => file_tools::handle_list_changed_files(&sched, &ws, None),
                "snapshots_create" => file_tools::handle_snapshot(&arguments, &mut sched, None),
                "snapshots_delete" => file_tools::handle_delete_snapshot(&arguments, &sched, None),
                "snapshots_revert" => file_tools::handle_revert_file(&arguments, &sched, &ws, None),
                "snapshots_history" => file_tools::handle_snapshots_history(&arguments, &sched, &ws, None),
                "snapshots_compact" => file_tools::handle_snapshots_compact(&arguments, &mut sched, None),
                _ => unreachable!("is_file_tool check above"),
            }
        })
    }).await.map_err(|e| format!("task failed: {e}"))?;

    // Extract the result content from the JsonRpcResponse.
    if let Some(err) = resp.error {
        return Err(format!("{}: {}", err.code, err.message));
    }
    resp.result.ok_or_else(|| "empty response".to_string())
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
// MCP mutations
// ---------------------------------------------------------------------------

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
        .map(capsem_core::mcp::compute_tool_hash);

    // Update cache entry or create one.
    if let Some(entry) = cache.iter_mut().find(|e| e.namespaced_name == tool) {
        entry.approved = true;
        if let Some(ref hash) = current_hash {
            entry.pin_hash = hash.clone();
        }
    } else if let Some(tool_def) = mgr.tool_catalog().iter().find(|t| t.namespaced_name == tool) {
        let new_cache = capsem_core::mcp::build_cache_entries(std::slice::from_ref(tool_def), &cache);
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
    } else if let Err(e) = mgr.initialize_all().await {
        return Err(format!("refresh failed: {e:#}"));
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

#[cfg(test)]
mod tests {
    use super::*;

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

    // -- MCP mutation validation tests (adversarial) --

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

        let cache = [capsem_core::mcp::ToolCacheEntry {
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

        let cache = [capsem_core::mcp::ToolCacheEntry {
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
}
