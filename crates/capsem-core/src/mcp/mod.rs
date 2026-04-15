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
use tracing::{debug, warn};

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
    let mut servers = Vec::new();
    let mut seen = std::collections::HashSet::new();

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
                headers: manual.headers.clone(),
                bearer_token: manual.bearer_token.clone(),
                enabled: manual.enabled,
                source: "manual".to_string(),
                unsupported_stdio: false,
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
                headers: corp_server.headers.clone(),
                bearer_token: corp_server.bearer_token.clone(),
                enabled: corp_server.enabled,
                source: "corp".to_string(),
                unsupported_stdio: false,
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

/// Tool cache file path: ~/.capsem/mcp_tool_cache.json
fn tool_cache_path() -> Option<std::path::PathBuf> {
    dirs_home().map(|h| h.join(".capsem").join("mcp_tool_cache.json"))
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
/// - Stdio servers: `{ "command": "npx", "args": [...] }` -> flagged as unsupported_stdio
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
                headers,
                bearer_token,
                enabled: true,
                source: source.to_string(),
                unsupported_stdio: false,
            });
            continue;
        }

        // Check for stdio server (command field) -- flag as unsupported
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

            // Store the command in url field for display purposes
            let display_command = if args.is_empty() {
                command.to_string()
            } else {
                format!("{} {}", command, args.join(" "))
            };

            debug!(name, source, command, "detected stdio MCP server (unsupported)");
            defs.push(McpServerDef {
                name: name.clone(),
                url: display_command,
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
                source: source.to_string(),
                unsupported_stdio: true,
            });
        }
    }

    Some(defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use crate::mcp::policy::{McpManualServer, McpUserConfig};

    fn make_tool(ns_name: &str, orig_name: &str, server: &str, desc: Option<&str>) -> McpToolDef {
        McpToolDef {
            namespaced_name: ns_name.into(),
            original_name: orig_name.into(),
            description: desc.map(String::from),
            input_schema: serde_json::json!({"type": "object"}),
            server_name: server.into(),
            annotations: None,
        }
    }

    // ── compute_tool_hash tests ─────────────────────────────────────

    #[test]
    fn compute_tool_hash_deterministic() {
        let tool = make_tool("github__search", "search", "github", Some("Search repos"));
        let h1 = compute_tool_hash(&tool);
        let h2 = compute_tool_hash(&tool);
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_tool_hash_changes_on_description() {
        let mut tool = make_tool("github__search", "search", "github", Some("Search repos"));
        let h1 = compute_tool_hash(&tool);
        tool.description = Some("Search all repos".into());
        let h2 = compute_tool_hash(&tool);
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_tool_hash_changes_on_schema() {
        let mut tool = make_tool("github__search", "search", "github", Some("Search"));
        let h1 = compute_tool_hash(&tool);
        tool.input_schema = serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let h2 = compute_tool_hash(&tool);
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_tool_hash_changes_on_annotations() {
        let mut tool = make_tool("github__search", "search", "github", Some("Search"));
        tool.annotations = Some(ToolAnnotations { read_only_hint: true, ..Default::default() });
        let h1 = compute_tool_hash(&tool);
        tool.annotations = Some(ToolAnnotations { read_only_hint: false, ..Default::default() });
        let h2 = compute_tool_hash(&tool);
        assert_ne!(h1, h2);
    }

    // ── detect_pin_changes tests ────────────────────────────────────

    #[test]
    fn detect_pin_changes_no_change() {
        let tool = make_tool("github__search", "search", "github", Some("Search"));
        let hash = compute_tool_hash(&tool);
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: hash,
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[tool], &cache);
        assert!(changes.is_empty());
    }

    #[test]
    fn detect_pin_changes_description_changed() {
        let tool = make_tool("github__search", "search", "github", Some("New description"));
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Old description".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: "oldhash".into(),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[tool], &cache);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], PinChange::Changed { .. }));
    }

    #[test]
    fn detect_pin_changes_new_tool() {
        let tool = make_tool("github__new_tool", "new_tool", "github", None);
        let changes = detect_pin_changes(&[tool], &[]);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], PinChange::New { .. }));
    }

    #[test]
    fn detect_pin_changes_tool_removed() {
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__removed".into(),
            original_name: "removed".into(),
            description: None,
            server_name: "github".into(),
            annotations: None,
            pin_hash: "hash".into(),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[], &cache);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], PinChange::Removed { .. }));
    }

    #[test]
    fn rug_pull_subtle_description_change() {
        // Single character change must be detected
        let tool = make_tool("github__search", "search", "github", Some("Search repo"));
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search repos".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: compute_tool_hash(&make_tool("github__search", "search", "github", Some("Search repos"))),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[tool], &cache);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], PinChange::Changed { .. }));
    }

    #[test]
    fn rug_pull_schema_injection() {
        let mut tool = make_tool("github__search", "search", "github", Some("Search"));
        // Add a hidden parameter
        tool.input_schema = serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}, "hidden": {"type": "string"}}});
        let original = make_tool("github__search", "search", "github", Some("Search"));
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: compute_tool_hash(&original),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[tool], &cache);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn rug_pull_annotation_flip() {
        let mut tool = make_tool("github__delete", "delete", "github", Some("Delete"));
        tool.annotations = Some(ToolAnnotations { read_only_hint: false, ..Default::default() });
        let mut original = make_tool("github__delete", "delete", "github", Some("Delete"));
        original.annotations = Some(ToolAnnotations { read_only_hint: true, ..Default::default() });
        let cache = vec![ToolCacheEntry {
            namespaced_name: "github__delete".into(),
            original_name: "delete".into(),
            description: Some("Delete".into()),
            server_name: "github".into(),
            annotations: Some(ToolAnnotations { read_only_hint: true, ..Default::default() }),
            pin_hash: compute_tool_hash(&original),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let changes = detect_pin_changes(&[tool], &cache);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn cross_server_name_collision() {
        let tools = vec![
            make_tool("a__search", "search", "a", None),
            make_tool("b__search", "search", "b", None),
        ];
        let collisions = detect_name_collisions(&tools);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].0, "search");
        assert_eq!(collisions[0].1.len(), 2);
    }

    // ── tool cache I/O tests ────────────────────────────────────────

    #[test]
    fn tool_cache_roundtrip() {
        let entries = vec![ToolCacheEntry {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search".into()),
            server_name: "github".into(),
            annotations: None,
            pin_hash: "abc123".into(),
            first_seen: "2025-01-01".into(),
            last_seen: "2025-01-01".into(),
            approved: true,
        }];
        let json = serde_json::to_string(&entries).unwrap();
        let decoded: Vec<ToolCacheEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(entries, decoded);
    }

    #[test]
    fn tool_cache_missing_file_returns_empty() {
        // load_tool_cache with nonexistent HOME
        std::env::set_var("HOME", "/nonexistent_test_dir_xyz");
        let cache = load_tool_cache();
        assert!(cache.is_empty());
    }

    // ── build_server_list tests ─────────────────────────────────────

    #[test]
    fn build_server_list_empty() {
        let user = McpUserConfig::default();
        let corp = McpUserConfig::default();
        // No auto-detected servers in test env, no manual, no corp
        let list = build_server_list(&user, &corp);
        // May have auto-detected servers from local dev env, but at least no crash
        assert!(list.iter().all(|s| s.name != "builtin"));
    }

    #[test]
    fn build_server_list_manual_servers() {
        let user = McpUserConfig {
            servers: vec![McpManualServer {
                name: "myserver".into(),
                url: "https://mcp.example.com/v1".into(),
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
            }],
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let list = build_server_list(&user, &corp);
        assert!(list.iter().any(|s| s.name == "myserver" && s.source == "manual"));
    }

    #[test]
    fn build_server_list_corp_servers_added() {
        let user = McpUserConfig::default();
        let corp = McpUserConfig {
            servers: vec![McpManualServer {
                name: "corp-server".into(),
                url: "https://corp.internal/mcp".into(),
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
            }],
            ..Default::default()
        };
        let list = build_server_list(&user, &corp);
        assert!(list.iter().any(|s| s.name == "corp-server" && s.source == "corp"));
    }

    #[test]
    fn build_server_list_reject_builtin_name() {
        let user = McpUserConfig {
            servers: vec![McpManualServer {
                name: "builtin".into(),
                url: "https://evil.com/mcp".into(),
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
            }],
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let list = build_server_list(&user, &corp);
        assert!(!list.iter().any(|s| s.name == "builtin"));
    }

    #[test]
    fn build_server_list_empty_name_rejected() {
        let user = McpUserConfig {
            servers: vec![McpManualServer {
                name: "".into(),
                url: "https://test.com/mcp".into(),
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
            }],
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let list = build_server_list(&user, &corp);
        assert!(!list.iter().any(|s| s.name.is_empty()));
    }

    #[test]
    fn build_server_list_enabled_override() {
        let user = McpUserConfig {
            servers: vec![McpManualServer {
                name: "myserver".into(),
                url: "https://mcp.example.com/v1".into(),
                headers: HashMap::new(),
                bearer_token: None,
                enabled: true,
            }],
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("myserver".into(), false);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let list = build_server_list(&user, &corp);
        let s = list.iter().find(|s| s.name == "myserver").unwrap();
        assert!(!s.enabled);
    }

    // ── original parse tests ────────────────────────────────────────

    #[test]
    fn parse_claude_settings_stdio_flagged_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"{{
            "mcpServers": {{
                "github": {{
                    "command": "npx",
                    "args": ["-y", "@github/mcp-server"],
                    "env": {{"GITHUB_TOKEN": "ghp_secret"}}
                }},
                "capsem": {{
                    "command": "/run/capsem-mcp-server"
                }}
            }}
        }}"#
        )
        .unwrap();

        let defs = parse_mcp_servers_from_file(&path, "claude").unwrap();
        assert_eq!(defs.len(), 1); // capsem filtered out
        assert_eq!(defs[0].name, "github");
        assert!(defs[0].unsupported_stdio);
        assert_eq!(defs[0].url, "npx -y @github/mcp-server"); // display command
        assert_eq!(defs[0].source, "claude");
    }

    #[test]
    fn parse_http_server_from_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"api": {"url": "https://mcp.example.com/v1", "bearerToken": "tok_123"}}}"#,
        )
        .unwrap();

        let defs = parse_mcp_servers_from_file(&path, "claude").unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "api");
        assert_eq!(defs[0].url, "https://mcp.example.com/v1");
        assert_eq!(defs[0].bearer_token.as_deref(), Some("tok_123"));
        assert!(!defs[0].unsupported_stdio);
    }

    #[test]
    fn parse_mixed_stdio_and_http_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {
                "http-server": {"url": "https://mcp.example.com/v1"},
                "stdio-server": {"command": "npx", "args": ["-y", "@test/server"]}
            }}"#,
        )
        .unwrap();

        let defs = parse_mcp_servers_from_file(&path, "test").unwrap();
        assert_eq!(defs.len(), 2);
        let http = defs.iter().find(|d| d.name == "http-server").unwrap();
        let stdio = defs.iter().find(|d| d.name == "stdio-server").unwrap();
        assert!(!http.unsupported_stdio);
        assert!(stdio.unsupported_stdio);
    }

    #[test]
    fn parse_missing_file_returns_none() {
        let result = parse_mcp_servers_from_file(Path::new("/nonexistent/settings.json"), "test");
        assert!(result.is_none());
    }

    #[test]
    fn parse_no_mcp_servers_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"other": "stuff"}"#).unwrap();
        let result = parse_mcp_servers_from_file(&path, "test");
        assert!(result.is_none());
    }

    #[test]
    fn parse_server_without_url_or_command_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"bad": {"name": "bad"}}}"#,
        )
        .unwrap();
        let defs = parse_mcp_servers_from_file(&path, "test").unwrap();
        assert_eq!(defs.len(), 0);
    }

    #[test]
    fn build_server_list_rejects_names_with_separator() {
        let mut user = McpUserConfig::default();
        user.servers.push(crate::mcp::policy::McpManualServer {
            name: "bad__name".to_string(),
            url: "http://localhost".to_string(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        });
        user.servers.push(crate::mcp::policy::McpManualServer {
            name: "goodname".to_string(),
            url: "http://localhost".to_string(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        });

        let mut corp = McpUserConfig::default();
        corp.servers.push(crate::mcp::policy::McpManualServer {
            name: "corp__bad".to_string(),
            url: "http://localhost".to_string(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        });

        let servers = build_server_list(&user, &corp);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "goodname");
    }

    // ------------------------------------------------------------------
    // Binary coverage: ensure every [[bin]] in capsem-agent/Cargo.toml
    // appears in Dockerfile.rootfs and justfile _pack-initrd.
    // ------------------------------------------------------------------

    /// Parse [[bin]] name entries from a Cargo.toml file.
    fn parse_cargo_bin_names(path: &std::path::Path) -> Vec<String> {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let doc: toml::Value = toml::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
        doc.get("bin")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| {
                        entry.get("name").and_then(|n| n.as_str()).map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn repo_root() -> std::path::PathBuf {
        // CARGO_MANIFEST_DIR is crates/capsem-core
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn all_guest_binaries_in_dockerfile_rootfs() {
        let root = repo_root();
        let bins = parse_cargo_bin_names(&root.join("crates/capsem-agent/Cargo.toml"));
        assert!(!bins.is_empty(), "no [[bin]] entries found in capsem-agent");

        let template = std::fs::read_to_string(
            root.join("src/capsem/builder/templates/Dockerfile.rootfs.j2"),
        )
        .expect("cannot read Dockerfile.rootfs.j2");

        // The Jinja template uses a loop over guest_binaries to COPY each binary.
        // Verify the loop pattern exists -- the Python build context test
        // (test_docker.py) verifies the actual binary list matches.
        assert!(
            template.contains("{% for binary in guest_binaries %}"),
            "Dockerfile.rootfs.j2 missing guest_binaries loop"
        );
        assert!(
            template.contains("COPY {{ binary }} /usr/local/bin/{{ binary }}"),
            "Dockerfile.rootfs.j2 missing COPY template for guest binaries"
        );

        // Also verify that prepare_build_context includes all agent binaries
        // by checking the Python build context function lists them.
        let docker_py = std::fs::read_to_string(root.join("src/capsem/builder/docker.py"))
            .expect("cannot read docker.py");
        for bin in &bins {
            assert!(
                docker_py.contains(bin),
                "docker.py missing guest binary '{bin}' in build context"
            );
        }
    }

    #[test]
    fn all_guest_binaries_in_pack_initrd() {
        let root = repo_root();
        let bins = parse_cargo_bin_names(&root.join("crates/capsem-agent/Cargo.toml"));
        assert!(!bins.is_empty(), "no [[bin]] entries found in capsem-agent");

        let justfile = std::fs::read_to_string(root.join("justfile"))
            .expect("cannot read justfile");

        // Extract the _pack-initrd recipe section (from "_pack-initrd:" to next recipe)
        let start = justfile
            .find("_pack-initrd:")
            .expect("_pack-initrd recipe not found in justfile");
        let section = &justfile[start..];
        let end = section[1..]
            .find("\n\n")
            .map(|i| i + 1)
            .unwrap_or(section.len());
        let recipe = &section[..end];

        for bin in &bins {
            assert!(
                recipe.contains(bin),
                "justfile _pack-initrd missing guest binary '{bin}'. \
                 Add cp + chmod lines for {bin}."
            );
        }
    }
}
