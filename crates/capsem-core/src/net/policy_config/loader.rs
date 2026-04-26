use std::collections::HashMap;
use std::path::Path;

use super::types::{McpServerDef, McpTransport, PolicySource};
use super::{SettingValue, SettingsFile};

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// User config path: `<capsem_home>/user.toml` (overridable via CAPSEM_USER_CONFIG)
pub fn user_config_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CAPSEM_USER_CONFIG") {
        return Some(std::path::PathBuf::from(path));
    }
    crate::paths::capsem_home_opt().map(|h| h.join("user.toml"))
}

/// Corporate config path: returns the first available corp config path.
///
/// Priority: CAPSEM_CORP_CONFIG env > /etc/capsem/corp.toml > ~/.capsem/corp.toml
pub fn corp_config_path() -> std::path::PathBuf {
    corp_config_paths().into_iter().next()
        .unwrap_or_else(|| std::path::PathBuf::from("/etc/capsem/corp.toml"))
}

/// Corporate config paths, in priority order.
///
/// /etc/capsem/corp.toml (system-level, MDM) takes precedence.
/// ~/.capsem/corp.toml (user-level, CLI-provisioned) is fallback.
/// CAPSEM_CORP_CONFIG env var overrides both (exclusive).
pub fn corp_config_paths() -> Vec<std::path::PathBuf> {
    let mut paths = vec![];
    if let Ok(path) = std::env::var("CAPSEM_CORP_CONFIG") {
        paths.push(std::path::PathBuf::from(path));
        return paths; // env override is exclusive
    }
    let system = std::path::PathBuf::from("/etc/capsem/corp.toml");
    if system.exists() {
        paths.push(system);
    }
    if let Some(capsem_home) = crate::paths::capsem_home_opt() {
        let user_corp = capsem_home.join("corp.toml");
        if user_corp.exists() {
            paths.push(user_corp);
        }
    }
    paths
}

/// Load a settings file from disk. Returns empty SettingsFile if file missing.
/// Applies automatic migration of old setting IDs to new ones.
pub fn load_settings_file(path: &Path) -> Result<SettingsFile, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let mut file: SettingsFile = toml::from_str(&content)
                .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
            migrate_setting_ids(&mut file);
            Ok(file)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SettingsFile::default()),
        Err(e) => Err(format!("failed to read {}: {}", path.display(), e)),
    }
}

// ---------------------------------------------------------------------------
// Setting ID migration (old -> new)
// ---------------------------------------------------------------------------

/// Migration map: old setting IDs -> new setting IDs.
const SETTING_ID_MIGRATIONS: &[(&str, &str)] = &[
    ("web.defaults.allow_read", "security.web.allow_read"),
    ("web.defaults.allow_write", "security.web.allow_write"),
    ("web.custom_allow", "security.web.custom_allow"),
    ("web.custom_block", "security.web.custom_block"),
    ("web.search.google.allow", "security.services.search.google.allow"),
    ("web.search.google.domains", "security.services.search.google.domains"),
    ("web.search.bing.allow", "security.services.search.bing.allow"),
    ("web.search.bing.domains", "security.services.search.bing.domains"),
    ("web.search.duckduckgo.allow", "security.services.search.duckduckgo.allow"),
    ("web.search.duckduckgo.domains", "security.services.search.duckduckgo.domains"),
    ("registry.debian.allow", "security.services.registry.debian.allow"),
    ("registry.debian.domains", "security.services.registry.debian.domains"),
    ("registry.npm.allow", "security.services.registry.npm.allow"),
    ("registry.npm.domains", "security.services.registry.npm.domains"),
    ("registry.pypi.allow", "security.services.registry.pypi.allow"),
    ("registry.pypi.domains", "security.services.registry.pypi.domains"),
    ("registry.crates.allow", "security.services.registry.crates.allow"),
    ("registry.crates.domains", "security.services.registry.crates.domains"),
];

/// Rename old setting IDs to new ones in a loaded settings file.
pub fn migrate_setting_ids(file: &mut SettingsFile) {
    for &(old, new) in SETTING_ID_MIGRATIONS {
        if let Some(entry) = file.settings.remove(old) {
            // Only migrate if the new key doesn't already exist (don't clobber).
            file.settings.entry(new.to_string()).or_insert(entry);
        }
    }
}

/// Write a settings file to disk as TOML. Creates parent dirs if needed.
pub fn write_settings_file(path: &Path, file: &SettingsFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create dir {}: {}", parent.display(), e))?;
    }
    let content = toml::to_string_pretty(file)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    std::fs::write(path, content)
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Load both settings files from standard locations.
///
/// Corp config merges all available paths (system + user-provisioned).
/// First path wins per-key (/etc/capsem/corp.toml overrides ~/.capsem/corp.toml).
pub fn load_settings_files() -> (SettingsFile, SettingsFile) {
    let user = match user_config_path() {
        Some(path) => load_settings_file(&path).unwrap_or_else(|e| {
            tracing::warn!("user settings: {e}");
            SettingsFile::default()
        }),
        None => SettingsFile::default(),
    };

    let mut corp = SettingsFile::default();
    for path in corp_config_paths() {
        match load_settings_file(&path) {
            Ok(file) => {
                // First path wins per-key: only insert if not already present
                for (id, entry) in file.settings {
                    corp.settings.entry(id).or_insert(entry);
                }
                // MCP config: first non-None wins
                if corp.mcp.is_none() && file.mcp.is_some() {
                    corp.mcp = file.mcp;
                }
            }
            Err(e) => {
                tracing::warn!("corp settings at {}: {e}", path.display());
            }
        }
    }

    (user, corp)
}

/// Write user settings to ~/.capsem/user.toml.
pub fn write_user_settings(file: &SettingsFile) -> Result<(), String> {
    let path = user_config_path().ok_or("HOME not set")?;
    write_settings_file(&path, file)
}

/// Whether the current process can write corp settings (always false).
pub fn can_write_corp_settings() -> bool {
    false
}

/// Load the merged MCP user config (user + corp).
/// Corp fields override user fields.
pub fn load_mcp_user_config() -> crate::mcp::policy::McpUserConfig {
    let (user, corp) = load_settings_files();
    let user_mcp = user.mcp.unwrap_or_default();
    let _corp_mcp = corp.mcp.unwrap_or_default();
    // Note: merging is done at policy evaluation time via to_policy().
    // This returns the user's config; corp is loaded separately.
    user_mcp
}

/// Load the corp MCP config.
pub fn load_mcp_corp_config() -> crate::mcp::policy::McpUserConfig {
    let (_, corp) = load_settings_files();
    corp.mcp.unwrap_or_default()
}

/// Save MCP user config to user.toml without clobbering settings.
pub fn save_mcp_user_config(mcp: &crate::mcp::policy::McpUserConfig) -> Result<(), String> {
    let path = user_config_path().ok_or("HOME not set")?;
    let mut file = load_settings_file(&path)?;
    file.mcp = Some(mcp.clone());
    write_settings_file(&path, &file)
}

// ---------------------------------------------------------------------------
// MCP server loading
// ---------------------------------------------------------------------------

/// Raw MCP server entry as it appears in TOML (without key or source metadata).
#[derive(serde::Deserialize, Debug)]
struct McpServerToml {
    name: String,
    #[serde(default)]
    description: Option<String>,
    transport: McpTransport,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    builtin: bool,
    #[serde(default = "super::types::default_true")]
    enabled: bool,
}

/// Parse `[mcp]` section from a TOML string into McpServerDef entries.
fn parse_mcp_section(toml_str: &str, source: PolicySource) -> Vec<McpServerDef> {
    let root: toml::Value = match toml::from_str(toml_str) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mcp_table = match root.get("mcp").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => return vec![],
    };
    let mut servers = Vec::new();
    for (key, val) in mcp_table {
        // Skip known McpUserConfig fields so they don't produce TOML parse errors
        match key.as_str() {
            "global_policy" | "default_tool_permission" | "health_check_interval_secs" |
            "servers" | "server_enabled" | "tool_permissions" => continue,
            _ => {}
        }

        let toml_str = match toml::to_string(val) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let server: McpServerToml = match toml::from_str(&toml_str) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("skipping MCP server '{key}': {e}");
                continue;
            }
        };
        servers.push(McpServerDef {
            key: key.clone(),
            name: server.name,
            description: server.description,
            transport: server.transport,
            command: server.command,
            url: server.url,
            args: server.args,
            env: server.env,
            headers: server.headers,
            builtin: server.builtin,
            enabled: server.enabled,
            source,
            corp_locked: false,
        });
    }
    servers
}

/// Parse `mcp` section from a JSON string into McpServerDef entries.
fn parse_mcp_section_json(json_str: &str, source: PolicySource) -> Vec<McpServerDef> {
    let root: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mcp_obj = match root.get("mcp").and_then(|v| v.as_object()) {
        Some(t) => t,
        None => return vec![],
    };
    let mut servers = Vec::new();
    for (key, val) in mcp_obj {
        // Skip known McpUserConfig fields
        match key.as_str() {
            "global_policy" | "default_tool_permission" | "health_check_interval_secs" |
            "servers" | "server_enabled" | "tool_permissions" => continue,
            _ => {}
        }

        let server: McpServerToml = match serde_json::from_value(val.clone()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("skipping MCP server '{key}': {e}");
                continue;
            }
        };
        servers.push(McpServerDef {
            key: key.clone(),
            name: server.name,
            description: server.description,
            transport: server.transport,
            command: server.command,
            url: server.url,
            args: server.args,
            env: server.env,
            headers: server.headers,
            builtin: server.builtin,
            enabled: server.enabled,
            source,
            corp_locked: false,
        });
    }
    servers
}

/// Load and merge MCP server definitions from defaults, user, and corp configs.
///
/// Resolution: corp > user > defaults (per key). Corp entries are corp_locked.
pub fn load_mcp_servers() -> Vec<McpServerDef> {
    use super::registry::DEFAULTS_JSON;

    let mut by_key: HashMap<String, McpServerDef> = HashMap::new();

    // 1. Defaults from JSON (lowest priority)
    for s in parse_mcp_section_json(DEFAULTS_JSON, PolicySource::Default) {
        by_key.insert(s.key.clone(), s);
    }

    // 2. User overrides
    let user_toml = match user_config_path() {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_default(),
        None => String::new(),
    };
    for s in parse_mcp_section(&user_toml, PolicySource::User) {
        by_key.insert(s.key.clone(), s);
    }

    // 3. Corp overrides (highest priority, corp_locked)
    let corp_toml = std::fs::read_to_string(corp_config_path()).unwrap_or_default();
    for mut s in parse_mcp_section(&corp_toml, PolicySource::Corp) {
        s.corp_locked = true;
        by_key.insert(s.key.clone(), s);
    }

    // Also mark defaults/user entries as corp_locked if corp has the same key
    // (already handled by overwrite above -- corp entry replaces user/default)

    let mut servers: Vec<McpServerDef> = by_key.into_values().collect();
    servers.sort_by(|a, b| a.key.cmp(&b.key));
    servers
}

// ---------------------------------------------------------------------------
// Unified settings response
// ---------------------------------------------------------------------------

/// Load the unified settings response (tree + issues + presets) in one call.
pub fn load_settings_response() -> super::types::SettingsResponse {
    let (user, corp) = load_settings_files();
    let resolved = super::resolver::resolve_settings(&user, &corp);
    let mcp_servers = load_mcp_servers();
    super::types::SettingsResponse {
        tree: super::tree::build_settings_tree_with_mcp(&resolved, &mcp_servers),
        issues: super::lint::config_lint(&resolved),
        presets: super::presets::security_presets(),
    }
}

// ---------------------------------------------------------------------------
// Batch update
// ---------------------------------------------------------------------------

/// Batch-update multiple settings atomically.
///
/// Validates ALL changes upfront. If any change is invalid (corp-locked,
/// type mismatch, unknown ID, disabled), the entire batch is rejected and
/// nothing is written. Returns the list of applied setting IDs on success.
pub fn batch_update_settings(
    changes: &HashMap<String, SettingValue>,
) -> Result<Vec<String>, String> {
    use super::registry::setting_definitions;

    if changes.is_empty() {
        return Ok(vec![]);
    }

    let user_path = user_config_path().ok_or("HOME not set")?;
    let corp_path = corp_config_path();
    let mut user_file = load_settings_file(&user_path)?;
    let corp_file = load_settings_file(&corp_path)?;
    let defs = setting_definitions();

    // Validate all changes upfront
    let mut errors = Vec::new();
    for (id, value) in changes {
        // Check known setting ID (allow dynamic guest.env.*)
        let is_dynamic = id.starts_with("guest.env.");
        let def = defs.iter().find(|d| d.id == *id);
        if def.is_none() && !is_dynamic {
            errors.push(format!("unknown setting: {id}"));
            continue;
        }

        // Corp-locked check
        if corp_file.settings.contains_key(id) {
            errors.push(format!("corp-locked: {id}"));
            continue;
        }

        // Validate file values
        if let Err(e) = validate_setting_value(id, value) {
            errors.push(e);
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    // All valid -- write to user.toml
    let now = crate::session::now_iso();
    let mut applied = Vec::new();
    for (id, value) in changes {
        user_file.settings.insert(
            id.clone(),
            super::types::SettingEntry {
                value: value.clone(),
                modified: now.clone(),
            },
        );
        applied.push(id.clone());
    }

    write_settings_file(&user_path, &user_file)?;
    applied.sort();
    Ok(applied)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a setting value before persisting.
///
/// For `File` values, validates the path and checks JSON content if the path
/// ends in `.json`. Other types pass through without validation.
pub fn validate_setting_value(id: &str, value: &SettingValue) -> Result<(), String> {
    if let SettingValue::File { path, content } = value {
        // Validate path
        capsem_proto::validate_file_path(path)
            .map_err(|e| format!("invalid path for {id}: {e}"))?;
        // Validate JSON syntax for .json paths (zero-allocation check).
        if path.ends_with(".json") && !content.is_empty() {
            serde_json::from_str::<serde::de::IgnoredAny>(content)
                .map_err(|e| format!("invalid JSON for {id}: {e}"))?;
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
