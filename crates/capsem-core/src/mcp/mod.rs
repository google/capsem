pub mod aggregator;
pub mod builtin_tools;
pub mod file_tools;
pub mod gateway;
pub mod policy;
pub mod server_manager;
pub mod types;

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::mcp::policy::McpUserConfig;
use crate::mcp::types::{McpServerDef, McpToolDef, ToolAnnotations};

/// Read MCP server definitions from the user's existing AI CLI configs.
/// Scans ~/.claude/settings.json and ~/.gemini/settings.json for mcpServers.
pub fn detect_host_mcp_servers() -> Vec<McpServerDef> {
    let home = match dirs_home() {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut servers = Vec::new();

    // Claude Code: ~/.claude/settings.json
    let claude_path = home.join(".claude").join("settings.json");
    if let Some(mut defs) = parse_mcp_servers_from_file(&claude_path, "claude") {
        servers.append(&mut defs);
    }

    // Gemini CLI: ~/.gemini/settings.json
    let gemini_path = home.join(".gemini").join("settings.json");
    if let Some(mut defs) = parse_mcp_servers_from_file(&gemini_path, "gemini") {
        servers.append(&mut defs);
    }

    // Deduplicate by name (first occurrence wins)
    let mut seen = std::collections::HashSet::new();
    servers.retain(|s| seen.insert(s.name.clone()));

    debug!(count = servers.len(), "auto-detected MCP servers");
    servers
}

// ---------------------------------------------------------------------------
// Unified server list builder
// ---------------------------------------------------------------------------

/// Build the unified server list: auto-detected + manual + corp-injected.
/// Deduplicates by name (first occurrence wins). Applies enabled overrides.
pub fn build_server_list(
    user_config: &McpUserConfig,
    corp_config: &McpUserConfig,
) -> Vec<McpServerDef> {
    build_server_list_with_builtin(user_config, corp_config, None, HashMap::new())
}

/// Build the server list, optionally including the local builtin server.
///
/// When `builtin_binary` is Some, a "local" server entry is prepended that
/// spawns the capsem-mcp-builtin binary via stdio transport.
///
/// `builtin_env` passes environment variables to the subprocess (session dir,
/// domain policy, DB path).
pub fn build_server_list_with_builtin(
    user_config: &McpUserConfig,
    corp_config: &McpUserConfig,
    builtin_binary: Option<&std::path::Path>,
    builtin_env: HashMap<String, String>,
) -> Vec<McpServerDef> {
    let mut servers = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 0. Local builtin server (stdio subprocess)
    if let Some(bin) = builtin_binary {
        if bin.exists() {
            servers.push(McpServerDef {
                name: "local".to_string(),
                url: String::new(),
                command: Some(bin.to_string_lossy().to_string()),
                args: vec![],
                env: builtin_env,
                headers: std::collections::HashMap::new(),
                bearer_token: None,
                enabled: true,
                source: "builtin".to_string(),
            });
            seen.insert("local".to_string());
            info!(bin = %bin.display(), "added local builtin MCP server");
        } else {
            warn!(bin = %bin.display(), "builtin MCP server binary not found, skipping");
        }
    }

    // 1. Auto-detected servers (claude, gemini configs)
    for mut def in detect_host_mcp_servers() {
        if def.name.is_empty() {
            continue;
        }
        // Reject reserved names
        if def.name == "builtin" {
            warn!(name = %def.name, "auto-detected server uses reserved name, skipping");
            continue;
        }
        // Reject names containing the namespace separator
        if def.name.contains(crate::mcp::types::NS_SEP) {
            warn!(name = %def.name, "auto-detected server name contains namespace separator '{}', skipping to prevent ambiguity", crate::mcp::types::NS_SEP);
            continue;
        }
        // Apply enabled overrides: corp > user
        if let Some(&enabled) = corp_config.server_enabled.get(&def.name) {
            def.enabled = enabled;
        } else if let Some(&enabled) = user_config.server_enabled.get(&def.name) {
            def.enabled = enabled;
        }
        if seen.insert(def.name.clone()) {
            servers.push(def);
        }
    }

    // 2. User manual servers
    for manual in &user_config.servers {
        if manual.name.is_empty() {
            warn!("manual server has empty name, skipping");
            continue;
        }
        if manual.name == "builtin" {
            warn!("manual server uses reserved name 'builtin', skipping");
            continue;
        }
        if manual.name.contains(crate::mcp::types::NS_SEP) {
            warn!(name = %manual.name, "manual server name contains namespace separator '{}', skipping to prevent ambiguity", crate::mcp::types::NS_SEP);
            continue;
        }
        if seen.insert(manual.name.clone()) {
            let mut def = McpServerDef {
                name: manual.name.clone(),
                url: manual.url.clone(),
                command: None,
                args: vec![],
                env: HashMap::new(),
                headers: manual.headers.clone(),
                bearer_token: manual.bearer_token.clone(),
                enabled: manual.enabled,
                source: "manual".to_string(),
            };
            // Apply enabled overrides
            if let Some(&enabled) = corp_config.server_enabled.get(&def.name) {
                def.enabled = enabled;
            } else if let Some(&enabled) = user_config.server_enabled.get(&def.name) {
                def.enabled = enabled;
            }
            servers.push(def);
        }
    }

    // 3. Corp-injected servers
    for corp_server in &corp_config.servers {
        if corp_server.name.is_empty() {
            continue;
        }
        if corp_server.name.contains(crate::mcp::types::NS_SEP) {
            warn!(name = %corp_server.name, "corp server name contains namespace separator '{}', skipping to prevent ambiguity", crate::mcp::types::NS_SEP);
            continue;
        }
        if seen.insert(corp_server.name.clone()) {
            servers.push(McpServerDef {
                name: corp_server.name.clone(),
                url: corp_server.url.clone(),
                command: None,
                args: vec![],
                env: HashMap::new(),
                headers: corp_server.headers.clone(),
                bearer_token: corp_server.bearer_token.clone(),
                enabled: corp_server.enabled,
                source: "corp".to_string(),
            });
        }
    }

    servers
}

// ---------------------------------------------------------------------------
// Tool cache with pinning (rug pull protection)
// ---------------------------------------------------------------------------

/// A cached tool entry with its pin hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCacheEntry {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    pub annotations: Option<ToolAnnotations>,
    pub pin_hash: String,
    pub first_seen: String,
    pub last_seen: String,
    pub approved: bool,
}

/// A detected change in a tool's definition.
#[derive(Debug, Clone, PartialEq)]
pub enum PinChange {
    /// Tool exists in cache but hash changed (rug pull).
    Changed {
        namespaced_name: String,
        old_hash: String,
        new_hash: String,
    },
    /// New tool not previously seen.
    New { namespaced_name: String },
    /// Tool was in cache but no longer provided by server.
    Removed { namespaced_name: String },
}

/// Compute a deterministic BLAKE3 hash of a tool's definition.
/// Includes name, description, inputSchema, and annotations.
pub fn compute_tool_hash(tool: &McpToolDef) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(tool.original_name.as_bytes());
    hasher.update(b"|");
    if let Some(desc) = &tool.description {
        hasher.update(desc.as_bytes());
    }
    hasher.update(b"|");
    hasher.update(tool.input_schema.to_string().as_bytes());
    hasher.update(b"|");
    if let Some(ann) = &tool.annotations {
        if let Ok(json) = serde_json::to_string(ann) {
            hasher.update(json.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// Detect changes between newly discovered tools and the cache.
pub fn detect_pin_changes(
    new_tools: &[McpToolDef],
    cache: &[ToolCacheEntry],
) -> Vec<PinChange> {
    let mut changes = Vec::new();
    let cache_map: HashMap<&str, &ToolCacheEntry> = cache
        .iter()
        .map(|e| (e.namespaced_name.as_str(), e))
        .collect();

    let mut seen = std::collections::HashSet::new();

    for tool in new_tools {
        let hash = compute_tool_hash(tool);
        seen.insert(tool.namespaced_name.as_str());

        match cache_map.get(tool.namespaced_name.as_str()) {
            Some(cached) => {
                if cached.pin_hash != hash {
                    changes.push(PinChange::Changed {
                        namespaced_name: tool.namespaced_name.clone(),
                        old_hash: cached.pin_hash.clone(),
                        new_hash: hash,
                    });
                }
            }
            None => {
                changes.push(PinChange::New {
                    namespaced_name: tool.namespaced_name.clone(),
                });
            }
        }
    }

    // Check for removed tools
    for cached in cache {
        if !seen.contains(cached.namespaced_name.as_str()) {
            changes.push(PinChange::Removed {
                namespaced_name: cached.namespaced_name.clone(),
            });
        }
    }

    changes
}

/// Detect cross-server name collisions (same original_name from different servers).
pub fn detect_name_collisions(tools: &[McpToolDef]) -> Vec<(String, Vec<String>)> {
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    for tool in tools {
        by_name
            .entry(&tool.original_name)
            .or_default()
            .push(&tool.server_name);
    }
    by_name
        .into_iter()
        .filter(|(_, servers)| servers.len() > 1)
        .map(|(name, servers)| (name.to_string(), servers.into_iter().map(String::from).collect()))
        .collect()
}

/// Tool cache file path inside the capsem home dir.
fn tool_cache_path() -> Option<std::path::PathBuf> {
    crate::paths::capsem_home_opt().map(|h| h.join("mcp_tool_cache.json"))
}

/// Save tool cache to disk.
pub fn save_tool_cache(entries: &[ToolCacheEntry]) -> Result<(), String> {
    let path = tool_cache_path().ok_or("HOME not set")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(entries)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("write: {e}"))
}

/// Load tool cache from disk. Returns empty vec if file missing.
pub fn load_tool_cache() -> Vec<ToolCacheEntry> {
    let path = match tool_cache_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            warn!("failed to parse tool cache: {e}");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

/// Build cache entries from current tool catalog.
pub fn build_cache_entries(tools: &[McpToolDef], existing: &[ToolCacheEntry]) -> Vec<ToolCacheEntry> {
    let now = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
    let existing_map: HashMap<&str, &ToolCacheEntry> = existing
        .iter()
        .map(|e| (e.namespaced_name.as_str(), e))
        .collect();

    tools
        .iter()
        .map(|tool| {
            let hash = compute_tool_hash(tool);
            let prev = existing_map.get(tool.namespaced_name.as_str());
            ToolCacheEntry {
                namespaced_name: tool.namespaced_name.clone(),
                original_name: tool.original_name.clone(),
                description: tool.description.clone(),
                server_name: tool.server_name.clone(),
                annotations: tool.annotations.clone(),
                pin_hash: hash.clone(),
                first_seen: prev.map(|p| p.first_seen.clone()).unwrap_or_else(|| now.clone()),
                last_seen: now.clone(),
                approved: prev.map(|p| {
                    // Stay approved only if hash hasn't changed
                    if p.pin_hash == hash { p.approved } else { false }
                }).unwrap_or(false),
            }
        })
        .collect()
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Parse mcpServers from a settings.json file.
/// Returns None if the file doesn't exist or can't be parsed.
///
/// Handles two formats:
/// - HTTP servers: `{ "url": "https://..." }` -> connectable MCP server
/// - Stdio servers: `{ "command": "npx", "args": [...] }` -> stdio transport
fn parse_mcp_servers_from_file(path: &Path, source: &str) -> Option<Vec<McpServerDef>> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let servers_obj = json.get("mcpServers")?.as_object()?;
    let mut defs = Vec::new();

    for (name, config) in servers_obj {
        // Skip the capsem server itself (we inject that)
        if name == "capsem" {
            continue;
        }

        // Check for HTTP server (url field)
        if let Some(url) = config.get("url").and_then(|v| v.as_str()) {
            let headers: HashMap<String, String> = config
                .get("headers")
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            let bearer_token = config
                .get("bearer_token")
                .or_else(|| config.get("bearerToken"))
                .and_then(|v| v.as_str())
                .map(String::from);

            debug!(name, source, url, "detected HTTP MCP server");
            defs.push(McpServerDef {
                name: name.clone(),
                url: url.to_string(),
                command: None,
                args: vec![],
                env: HashMap::new(),
                headers,
                bearer_token,
                enabled: true,
                source: source.to_string(),
            });
            continue;
        }

        // Check for stdio server (command field)
        if let Some(command) = config.get("command").and_then(|v| v.as_str()) {
            let args: Vec<String> = config
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let env: HashMap<String, String> = config
                .get("env")
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            debug!(name, source, command, "detected stdio MCP server");
            defs.push(McpServerDef {
                name: name.clone(),
                url: String::new(),
                command: Some(command.to_string()),
                args,
                env,
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
                source: source.to_string(),
            });
        }
    }

    Some(defs)
}

#[cfg(test)]
mod tests;
