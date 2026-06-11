use std::collections::HashMap;
use std::path::Path;

use super::types::{McpServerDef, McpTransport, PolicySource};
use super::{
    setting_id_owner, validate_corp_toml_contract, validate_settings_toml_contract,
    validate_stored_setting_contract, ConfigOwner, SettingValue, SettingsFile,
};

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// Local UI settings path: `<capsem_home>/settings.toml`.
pub fn settings_config_path() -> Option<std::path::PathBuf> {
    crate::paths::capsem_home_opt().map(|h| h.join("settings.toml"))
}

/// Corporate config path: returns the first available corp config path.
///
/// Priority: CAPSEM_CORP_CONFIG env > /etc/capsem/corp.toml > ~/.capsem/corp.toml
pub fn corp_config_path() -> std::path::PathBuf {
    corp_config_paths()
        .into_iter()
        .next()
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
            reject_retired_mcp_policy_keys(path, &content)?;
            reject_retired_ai_setting_ids(path, &content)?;
            let mut file: SettingsFile = toml::from_str(&content)
                .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
            migrate_setting_ids(&mut file);
            if let Some(profile) = load_referenced_enforcement_rules(path, &file)? {
                merge_referenced_security_rule_profile(&mut file, profile)?;
            }
            if let Some(profile) = load_referenced_sigma_rules(path, &file)? {
                merge_referenced_security_rule_profile(&mut file, profile)?;
            }
            file.validate_metadata_contract()
                .map_err(|e| format!("failed to validate {}: {e}", path.display()))?;
            Ok(file)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SettingsFile::default()),
        Err(e) => Err(format!("failed to read {}: {}", path.display(), e)),
    }
}

/// Load a local UI/application settings file and reject profile-owned behavior.
pub fn load_local_settings_file(path: &Path) -> Result<SettingsFile, String> {
    let file = load_settings_file(path)?;
    validate_settings_toml_contract(&file)
        .map_err(|e| format!("failed to validate {}: {e}", path.display()))?;
    Ok(file)
}

/// Load a corporate constraint file and reject UI preferences.
pub fn load_corp_settings_file(path: &Path) -> Result<SettingsFile, String> {
    let file = load_settings_file(path)?;
    validate_corp_toml_contract(&file)
        .map_err(|e| format!("failed to validate {}: {e}", path.display()))?;
    Ok(file)
}

fn reject_retired_mcp_policy_keys(path: &Path, content: &str) -> Result<(), String> {
    let root: toml::Value = toml::from_str(content)
        .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
    let Some(mcp) = root.get("mcp").and_then(|value| value.as_table()) else {
        return Ok(());
    };
    for retired in [
        "global_policy",
        "default_tool_permission",
        "tool_permissions",
    ] {
        if mcp.contains_key(retired) {
            return Err(format!(
                "failed to validate {}: retired MCP policy key mcp.{retired}; use profile security rules instead",
                path.display()
            ));
        }
    }
    Ok(())
}

fn reject_retired_ai_setting_ids(path: &Path, content: &str) -> Result<(), String> {
    reject_retired_ai_setting_ids_in_content(&path.display().to_string(), content)
}

pub(super) fn reject_retired_ai_setting_ids_in_content(
    label: &str,
    content: &str,
) -> Result<(), String> {
    let root: toml::Value =
        toml::from_str(content).map_err(|e| format!("failed to parse {label}: {e}"))?;
    let Some(settings) = root.get("settings").and_then(|value| value.as_table()) else {
        return Ok(());
    };
    for key in settings.keys() {
        if key.starts_with("ai.") {
            return Err(format!(
                "failed to validate {label}: retired AI setting id {key}; use profile/corp security rules, provider discovery, and plugins instead",
            ));
        }
    }
    Ok(())
}

fn merge_referenced_security_rule_profile(
    settings: &mut SettingsFile,
    profile: super::SecurityRuleProfile,
) -> Result<(), String> {
    merge_security_rule_group("profiles", &mut settings.profiles, profile.profiles)?;
    merge_security_rule_group("corp", &mut settings.corp, profile.corp)?;
    if !profile.ai.is_empty() {
        return Err("referenced rule files must use corp.rules or profiles.rules, not ai.*".into());
    }
    Ok(())
}

fn merge_security_rule_group(
    namespace: &str,
    target: &mut super::SecurityRuleGroup,
    source: super::SecurityRuleGroup,
) -> Result<(), String> {
    for (rule_id, rule) in source.rules {
        if target.rules.insert(rule_id.clone(), rule).is_some() {
            return Err(format!("duplicate referenced {namespace}.rules.{rule_id}"));
        }
    }
    Ok(())
}

pub fn resolve_rule_file_path(settings_path: &Path, rule_file: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(rule_file);
    if path.is_absolute() {
        return path;
    }
    settings_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

pub fn load_referenced_enforcement_rules(
    settings_path: &Path,
    settings: &SettingsFile,
) -> Result<Option<super::SecurityRuleProfile>, String> {
    let Some(rule_file) = settings.rule_files.enforcement.as_deref() else {
        return Ok(None);
    };
    let path = resolve_rule_file_path(settings_path, rule_file);
    let content = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "failed to read enforcement rules {}: {error}",
            path.display()
        )
    })?;
    super::SecurityRuleProfile::parse_toml(&content)
        .map(Some)
        .map_err(|error| {
            format!(
                "failed to parse enforcement rules {}: {error}",
                path.display()
            )
        })
}

pub fn load_referenced_sigma_rules(
    settings_path: &Path,
    settings: &SettingsFile,
) -> Result<Option<super::SecurityRuleProfile>, String> {
    let Some(rule_file) = settings.rule_files.sigma.as_deref() else {
        return Ok(None);
    };
    let path = resolve_rule_file_path(settings_path, rule_file);
    let content = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "failed to read Sigma detection rules {}: {error}",
            path.display()
        )
    })?;
    super::SecurityRuleProfile::parse_sigma_yaml(&content)
        .map(Some)
        .map_err(|error| {
            format!(
                "failed to parse Sigma detection rules {}: {error}",
                path.display()
            )
        })
}

// ---------------------------------------------------------------------------
// Setting ID migration (old -> new)
// ---------------------------------------------------------------------------

/// Migration map: old setting IDs -> new setting IDs.
const SETTING_ID_MIGRATIONS: &[(&str, &str)] = &[
    (
        "web.search.google.allow",
        "security.services.search.google.allow",
    ),
    (
        "web.search.google.domains",
        "security.services.search.google.domains",
    ),
    (
        "web.search.bing.allow",
        "security.services.search.bing.allow",
    ),
    (
        "web.search.bing.domains",
        "security.services.search.bing.domains",
    ),
    (
        "web.search.duckduckgo.allow",
        "security.services.search.duckduckgo.allow",
    ),
    (
        "web.search.duckduckgo.domains",
        "security.services.search.duckduckgo.domains",
    ),
    (
        "registry.debian.allow",
        "security.services.registry.debian.allow",
    ),
    (
        "registry.debian.domains",
        "security.services.registry.debian.domains",
    ),
    ("registry.npm.allow", "security.services.registry.npm.allow"),
    (
        "registry.npm.domains",
        "security.services.registry.npm.domains",
    ),
    (
        "registry.pypi.allow",
        "security.services.registry.pypi.allow",
    ),
    (
        "registry.pypi.domains",
        "security.services.registry.pypi.domains",
    ),
    (
        "registry.crates.allow",
        "security.services.registry.crates.allow",
    ),
    (
        "registry.crates.domains",
        "security.services.registry.crates.domains",
    ),
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
    let content =
        toml::to_string_pretty(file).map_err(|e| format!("failed to serialize settings: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Load local UI settings and corp constraints from standard locations.
///
/// Corp config merges all available paths (system + user-provisioned).
/// First path wins per-key (/etc/capsem/corp.toml overrides ~/.capsem/corp.toml).
pub fn load_settings_and_corp_files() -> (SettingsFile, SettingsFile) {
    let settings = match settings_config_path() {
        Some(path) => load_local_settings_file(&path).unwrap_or_else(|e| {
            tracing::warn!("local settings: {e}");
            SettingsFile::default()
        }),
        None => SettingsFile::default(),
    };

    let mut corp = SettingsFile::default();
    for path in corp_config_paths() {
        match load_corp_settings_file(&path) {
            Ok(file) => {
                // First path wins per-key: only insert if not already present
                for (id, entry) in file.settings {
                    corp.settings.entry(id).or_insert(entry);
                }
                // MCP config: first non-None wins
                if corp.mcp.is_none() && file.mcp.is_some() {
                    corp.mcp = file.mcp;
                }
                // External rule files: first corp path wins per reference.
                corp.rule_files.merge_first_wins(file.rule_files);
                corp.corp_rule_files.merge_first_wins(file.corp_rule_files);
                if corp.refresh_interval_hours.is_none() {
                    corp.refresh_interval_hours = file.refresh_interval_hours;
                }
                for (rule_id, rule) in file.default {
                    corp.default.entry(rule_id).or_insert(rule);
                }
                for (rule_id, rule) in file.profiles.rules {
                    corp.profiles.rules.entry(rule_id).or_insert(rule);
                }
                for (rule_id, rule) in file.corp.rules {
                    corp.corp.rules.entry(rule_id).or_insert(rule);
                }
                // Provider profile config: first corp path wins per provider.
                for (provider_id, provider) in file.ai {
                    corp.ai.entry(provider_id).or_insert(provider);
                }
                for (plugin_id, plugin) in file.plugins {
                    corp.plugins.entry(plugin_id).or_insert(plugin);
                }
            }
            Err(e) => {
                tracing::warn!("corp settings at {}: {e}", path.display());
            }
        }
    }

    (settings, corp)
}

/// Write local UI settings to `<capsem_home>/settings.toml`.
pub fn write_local_settings(file: &SettingsFile) -> Result<(), String> {
    let path = settings_config_path().ok_or("HOME not set")?;
    write_settings_file(&path, file)
}

/// Whether the current process can write corp settings (always false).
pub fn can_write_corp_settings() -> bool {
    false
}

/// Load the corp MCP config.
pub fn load_mcp_corp_config() -> crate::mcp::policy::McpUserConfig {
    let (_, corp) = load_settings_and_corp_files();
    corp.mcp.unwrap_or_default()
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
        // Skip global config keys that aren't server definitions
        if key == "health_check_interval_secs" || key == "server_enabled" {
            continue;
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
        // Skip global config keys that aren't server definitions
        if key == "health_check_interval_secs" || key == "server_enabled" {
            continue;
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

/// Load and merge MCP server definitions from defaults and corp configs.
///
/// Resolution: corp > defaults (per key). Corp entries are corp_locked.
pub fn load_mcp_servers() -> Vec<McpServerDef> {
    use super::registry::DEFAULTS_JSON;

    let mut by_key: HashMap<String, McpServerDef> = HashMap::new();

    // 1. Defaults from JSON (lowest priority)
    for s in parse_mcp_section_json(DEFAULTS_JSON, PolicySource::Default) {
        by_key.insert(s.key.clone(), s);
    }

    // 2. Corp overrides (highest priority, corp_locked)
    let corp_toml = std::fs::read_to_string(corp_config_path()).unwrap_or_default();
    for mut s in parse_mcp_section(&corp_toml, PolicySource::Corp) {
        s.corp_locked = true;
        by_key.insert(s.key.clone(), s);
    }

    // Also mark defaults as corp_locked if corp has the same key (already
    // handled by overwrite above -- corp entry replaces default).

    let mut servers: Vec<McpServerDef> = by_key.into_values().collect();
    servers.sort_by(|a, b| a.key.cmp(&b.key));
    servers
}

// ---------------------------------------------------------------------------
// Unified settings response
// ---------------------------------------------------------------------------

/// Load the unified settings response (tree + issues) in one call.
pub fn load_settings_response() -> super::types::SettingsResponse {
    let (settings, corp) = load_settings_and_corp_files();
    let resolved = super::resolver::resolve_settings(&settings, &corp);
    let mcp_servers = load_mcp_servers();
    super::types::SettingsResponse {
        tree: super::tree::build_settings_tree_with_mcp(&resolved, &mcp_servers),
        issues: super::lint::config_lint(&resolved),
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
    let mut raw = HashMap::new();
    for (id, value) in changes {
        let json = serde_json::to_value(value)
            .map_err(|e| format!("failed to encode setting {id}: {e}"))?;
        raw.insert(id.clone(), json);
    }
    batch_update_settings_json(&raw)
}

pub fn batch_update_settings_json(
    changes: &HashMap<String, serde_json::Value>,
) -> Result<Vec<String>, String> {
    batch_update_settings_json_inner(changes)
}

fn batch_update_settings_json_inner(
    changes: &HashMap<String, serde_json::Value>,
) -> Result<Vec<String>, String> {
    use super::registry::setting_definitions;

    if changes.is_empty() {
        return Ok(vec![]);
    }

    let settings_path = settings_config_path().ok_or("HOME not set")?;
    let corp_path = corp_config_path();
    let mut settings_file = load_local_settings_file(&settings_path)?;
    let corp_file = load_corp_settings_file(&corp_path)?;
    let defs = setting_definitions();
    let mut setting_changes = HashMap::new();

    // Validate all changes upfront
    let mut errors = Vec::new();
    for (id, value) in changes {
        if id.starts_with("policy.") {
            errors.push(format!(
                "unknown setting: {id}; use profiles.rules, corp.rules, ai.<provider>.rules, or rule_files"
            ));
            continue;
        }

        let value = match serde_json::from_value::<SettingValue>(value.clone()) {
            Ok(value) => value,
            Err(e) => {
                errors.push(format!("invalid value for {id}: {e}"));
                continue;
            }
        };

        // Check known setting ID (allow dynamic guest.env.*)
        let is_dynamic = id.starts_with("guest.env.");
        let def = defs.iter().find(|d| d.id == *id);
        if def.is_none() && !is_dynamic {
            errors.push(format!("unknown setting: {id}"));
            continue;
        }

        let actual_owner = setting_id_owner(id);
        if actual_owner != ConfigOwner::Settings {
            errors.push(format!(
                "{} update cannot write {}-owned setting: {id}",
                ConfigOwner::Settings.as_str(),
                actual_owner.as_str()
            ));
            continue;
        }

        // Corp-locked check
        if corp_file.settings.contains_key(id) {
            errors.push(format!("corp-locked: {id}"));
            continue;
        }

        // Validate file values
        if let Err(e) = validate_setting_value(id, &value) {
            errors.push(e);
        }
        setting_changes.insert(id.clone(), value);
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    // All valid -- write to local settings.toml
    let now = crate::session::now_iso();
    let mut applied = Vec::new();
    for (id, value) in setting_changes {
        settings_file.settings.insert(
            id.clone(),
            super::types::SettingEntry {
                value,
                modified: now.clone(),
            },
        );
        applied.push(id.clone());
    }

    write_settings_file(&settings_path, &settings_file)?;
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
    validate_stored_setting_contract(id, value)?;
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
