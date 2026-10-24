pub mod aggregator_driver;
pub mod builtin_ledger;
pub mod builtin_tools;
pub mod policy;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::mcp::policy::McpConfig;
use capsem_proto::mcp_contracts::{
    McpServerDef, McpToolDef, ToolAnnotations, BUILTIN_SERVER_NAME, BUILTIN_SERVER_SOURCE,
};

/// Compute a CPU-proportional default for framed MCP in-flight handlers.
///
/// Rule: `host_parallelism * 4`. This matches the empirically selected cap
/// from the legacy gateway and is now shared by the MITM MCP endpoint.
pub fn default_inflight_cap() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    cores * 4
}

/// Resolve the framed MCP in-flight cap from the environment, falling back
/// to the CPU-proportional default.
pub fn resolve_inflight_cap() -> usize {
    std::env::var("CAPSEM_MCP_INFLIGHT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(default_inflight_cap)
}

/// Server names no configured server may take: the builtin's own, and the name it had
/// before it was `local`. Reserved whether or not the builtin binary is
/// installed -- a configured server named `local` on a host without it would
/// otherwise own every `local__*` tool name, and nothing downstream could tell
/// its tools from the builtin's by name.
const RESERVED_SERVER_NAMES: &[&str] = &[BUILTIN_SERVER_NAME, "builtin"];

/// The names of the servers in `servers` that are Capsem's own builtin.
pub fn builtin_server_names(servers: &[McpServerDef]) -> BTreeSet<String> {
    servers
        .iter()
        .filter(|server| server.source == BUILTIN_SERVER_SOURCE)
        .map(|server| server.name.clone())
        .collect()
}

fn local_builtin_server_def(bin: &Path, builtin_env: HashMap<String, String>, enabled: bool) -> McpServerDef {
    // Stateless builtin tools that are safe to round-robin across pool
    // peers. A tool missing from this list pins to peers[0].
    let pool_safe_tools: Vec<String> = ["echo", "fetch_http", "grep_http", "http_headers"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    // A session's builtin runs one peer. It once wrote the session ledger
    // itself, which is why this used to key on the ledger's path; it no
    // longer does, and widening the pool is a separate decision.
    let pool_size = if builtin_env.contains_key("CAPSEM_SESSION_DIR") {
        Some(1)
    } else {
        let default_pool = std::thread::available_parallelism()
            .ok()
            .map(|n| (n.get() as u32).clamp(1, 4));
        std::env::var("CAPSEM_MCP_BUILTIN_POOL")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .map(|n| n.clamp(1, 16))
            .or(default_pool)
    };

    McpServerDef {
        name: BUILTIN_SERVER_NAME.to_string(),
        url: String::new(),
        command: Some(bin.to_string_lossy().to_string()),
        args: vec![],
        env: builtin_env,
        headers: std::collections::HashMap::new(),
        auth: None,
        enabled,
        source: BUILTIN_SERVER_SOURCE.to_string(),
        pool_size,
        pool_safe_tools,
    }
}

/// The source label of a server declared in settings or corp `[mcp]`.
pub const CONFIGURED_SERVER_SOURCE: &str = "config";

/// Build a session's MCP server list from its merged `[mcp]` configuration:
/// the builtin `local` server unless turned off, then each declared server.
///
/// This does not auto-detect host AI CLI MCP configs.
pub fn build_server_list(
    config: &McpConfig,
    builtin_binary: Option<&Path>,
    builtin_env: HashMap<String, String>,
) -> Vec<McpServerDef> {
    let admitted = builtin_binary.filter(|bin| {
        let available = bin.exists();
        if !available {
            warn!(bin = %bin.display(), "builtin MCP server binary not found, skipping");
        }
        available
    });
    build_server_list_from_admitted_binary(config, admitted, builtin_env)
}

/// Rebuild settings using executable availability admitted before confinement.
/// No filesystem access occurs here; the service still owns process creation.
pub fn build_server_list_from_admitted_binary(
    config: &McpConfig,
    builtin_binary: Option<&Path>,
    builtin_env: HashMap<String, String>,
) -> Vec<McpServerDef> {
    let mut servers = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(bin) = builtin_binary {
        let enabled = config.server_enabled.get(BUILTIN_SERVER_NAME).copied().unwrap_or(true);
        servers.push(local_builtin_server_def(bin, builtin_env, enabled));
        seen.insert(BUILTIN_SERVER_NAME.to_string());
        info!(bin = %bin.display(), "added admitted local builtin MCP server");
    }

    for manual in &config.servers {
        if manual.name.is_empty() {
            warn!("configured MCP server has empty name, skipping");
            continue;
        }
        if RESERVED_SERVER_NAMES.contains(&manual.name.as_str()) {
            warn!(name = %manual.name, "configured MCP server uses a name reserved for the builtin server, skipping");
            continue;
        }
        if manual.name.contains(capsem_proto::mcp_contracts::NS_SEP) {
            warn!(name = %manual.name, "configured MCP server name contains namespace separator '{}', skipping to prevent ambiguity", capsem_proto::mcp_contracts::NS_SEP);
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
                auth: manual.auth.clone(),
                enabled: manual.enabled,
                source: CONFIGURED_SERVER_SOURCE.to_string(),
                pool_size: None,
                pool_safe_tools: Vec::new(),
            };
            if let Some(&enabled) = config.server_enabled.get(&def.name) {
                def.enabled = enabled;
            }
            servers.push(def);
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
    let mut hasher = capsem_crypto::blake3::Hasher::new();
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
pub fn detect_pin_changes(new_tools: &[McpToolDef], cache: &[ToolCacheEntry]) -> Vec<PinChange> {
    let mut changes = Vec::new();
    let cache_map: HashMap<&str, &ToolCacheEntry> = cache.iter().map(|e| (e.namespaced_name.as_str(), e)).collect();

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
        by_name.entry(&tool.original_name).or_default().push(&tool.server_name);
    }
    by_name
        .into_iter()
        .filter(|(_, servers)| servers.len() > 1)
        .map(|(name, servers)| (name.to_string(), servers.into_iter().map(String::from).collect()))
        .collect()
}

/// Tool cache file path inside the capsem home dir.
fn tool_cache_path() -> Option<std::path::PathBuf> {
    capsem_foundation::paths::capsem_home_opt().map(|h| h.join("mcp_tool_cache.json"))
}

/// Save tool cache to disk.
pub fn save_tool_cache(entries: &[ToolCacheEntry]) -> Result<(), String> {
    let path = tool_cache_path().ok_or("HOME not set")?;
    if let Some(parent) = path.parent() {
        capsem_foundation::unix::fs::ensure_private_dir_restrict_existing(parent)
            .map_err(|e| format!("prepare private cache directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(entries).map_err(|e| format!("serialize: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(&path, json.as_bytes()).map_err(|e| format!("write: {e}"))
}

/// Load tool cache from disk. Returns empty vec if file missing.
pub fn load_tool_cache() -> Vec<ToolCacheEntry> {
    let path = match tool_cache_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            warn!(error = %e, "failed to parse tool cache");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

/// Build cache entries from current tool catalog.
pub fn build_cache_entries(tools: &[McpToolDef], existing: &[ToolCacheEntry]) -> Vec<ToolCacheEntry> {
    let now = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
    let existing_map: HashMap<&str, &ToolCacheEntry> =
        existing.iter().map(|e| (e.namespaced_name.as_str(), e)).collect();

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
                approved: prev
                    .map(|p| {
                        // Stay approved only if hash hasn't changed
                        if p.pin_hash == hash {
                            p.approved
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
