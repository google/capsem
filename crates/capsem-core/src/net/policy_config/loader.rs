use std::collections::HashMap;
use std::path::Path;

use super::{validate_settings_toml_contract, validate_stored_setting_contract, SettingValue, SettingsFile};

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// Local UI settings path: `<capsem_home>/settings.toml`.
pub fn settings_config_path() -> Option<std::path::PathBuf> {
    capsem_foundation::paths::capsem_home_opt().map(|h| h.join("settings.toml"))
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
    if let Some(capsem_home) = capsem_foundation::paths::capsem_home_opt() {
        let user_corp = capsem_home.join("corp.toml");
        if user_corp.exists() {
            paths.push(user_corp);
        }
    }
    paths
}

/// Load a settings file from disk. Returns empty SettingsFile if file missing.
/// Applies automatic migration of old setting IDs to new ones, and merges the
/// rules of referenced `rule_files` into the returned view.
pub fn load_settings_file(path: &Path) -> Result<SettingsFile, String> {
    match load_settings_document(path)? {
        Some(file) => resolve_settings_document(path, file),
        None => Ok(SettingsFile::default()),
    }
}

/// The settings file exactly as written: referenced rule files are named, not
/// merged. A read-modify-write edits this and never the merged view, which
/// would inline the referenced rules and then refuse them as duplicates on
/// the next load. `None` when the file does not exist.
pub fn load_settings_document(path: &Path) -> Result<Option<SettingsFile>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read {}: {}", path.display(), e)),
    };
    // One parse: polled routes load this file on every request. A retired MCP
    // key fails the typed parse (`McpConfig` denies unknown fields), so the
    // untyped look that names it runs only to explain a failure.
    let mut file: SettingsFile = match toml::from_str(&content) {
        Ok(file) => file,
        Err(error) => {
            reject_retired_mcp_policy_keys(path, &content)?;
            reject_retired_ai_setting_ids_in_content(&path.display().to_string(), &content)?;
            return Err(format!("failed to parse {}: {}", path.display(), error));
        }
    };
    reject_retired_ai_setting_id_keys(&path.display().to_string(), file.settings.keys())?;
    migrate_setting_ids(&mut file);
    Ok(Some(file))
}

/// Merge a document's referenced rule files and validate the result, as a
/// load of `path` holding `file` would.
fn resolve_settings_document(path: &Path, mut file: SettingsFile) -> Result<SettingsFile, String> {
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

/// Load the user's settings.toml and reject the sections only corp.toml may
/// define. Every registry setting id is the user's; see `ownership`.
pub fn load_local_settings_file(path: &Path) -> Result<SettingsFile, String> {
    let file = load_settings_file(path)?;
    validate_settings_toml_contract(&file).map_err(|e| format!("failed to validate {}: {e}", path.display()))?;
    Ok(file)
}

/// Validate `document` as the next content of the user's settings.toml at
/// `path` -- referenced rule files merged, the settings contract held -- and
/// write it whole. Returns the merged view the next load will see.
pub fn write_local_settings_document(path: &Path, document: &SettingsFile) -> Result<SettingsFile, String> {
    let resolved = resolve_settings_document(path, document.clone())?;
    validate_settings_toml_contract(&resolved).map_err(|e| format!("failed to validate {}: {e}", path.display()))?;
    write_settings_file(path, document)?;
    Ok(resolved)
}

fn reject_retired_mcp_policy_keys(path: &Path, content: &str) -> Result<(), String> {
    let root: toml::Value =
        toml::from_str(content).map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
    let Some(mcp) = root.get("mcp").and_then(|value| value.as_table()) else {
        return Ok(());
    };
    for retired in ["global_policy", "default_tool_permission", "tool_permissions"] {
        if mcp.contains_key(retired) {
            return Err(format!(
                "failed to validate {}: retired MCP policy key mcp.{retired}; use security rules instead",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn reject_retired_ai_setting_ids_in_content(label: &str, content: &str) -> Result<(), String> {
    let root: toml::Value = toml::from_str(content).map_err(|e| format!("failed to parse {label}: {e}"))?;
    let Some(settings) = root.get("settings").and_then(|value| value.as_table()) else {
        return Ok(());
    };
    reject_retired_ai_setting_id_keys(label, settings.keys())
}

/// The first retired `ai.*` setting id, by name, so the error does not depend
/// on the map's iteration order.
fn reject_retired_ai_setting_id_keys<'a>(label: &str, keys: impl Iterator<Item = &'a String>) -> Result<(), String> {
    match keys.filter(|key| key.starts_with("ai.")).min() {
        Some(key) => Err(format!(
            "failed to validate {label}: retired AI setting id {key}; use settings/corp security rules, provider discovery, and plugins instead",
        )),
        None => Ok(()),
    }
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
    settings_path.parent().unwrap_or_else(|| Path::new(".")).join(path)
}

pub fn load_referenced_enforcement_rules(
    settings_path: &Path,
    settings: &SettingsFile,
) -> Result<Option<super::SecurityRuleProfile>, String> {
    let Some(rule_file) = settings.rule_files.enforcement.as_deref() else {
        return Ok(None);
    };
    let path = resolve_rule_file_path(settings_path, rule_file);
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read enforcement rules {}: {error}", path.display()))?;
    super::SecurityRuleProfile::parse_toml(&content)
        .map(Some)
        .map_err(|error| format!("failed to parse enforcement rules {}: {error}", path.display()))
}

pub fn load_referenced_sigma_rules(
    settings_path: &Path,
    settings: &SettingsFile,
) -> Result<Option<super::SecurityRuleProfile>, String> {
    let Some(rule_file) = settings.rule_files.sigma.as_deref() else {
        return Ok(None);
    };
    let path = resolve_rule_file_path(settings_path, rule_file);
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read Sigma detection rules {}: {error}", path.display()))?;
    super::SecurityRuleProfile::parse_sigma_yaml(&content)
        .map(Some)
        .map_err(|error| format!("failed to parse Sigma detection rules {}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Setting ID migration (old -> new)
// ---------------------------------------------------------------------------

/// Migration map: old setting IDs -> new setting IDs.
const SETTING_ID_MIGRATIONS: &[(&str, &str)] = &[
    ("web.search.google.allow", "security.services.search.google.allow"),
    ("web.search.google.domains", "security.services.search.google.domains"),
    ("web.search.bing.allow", "security.services.search.bing.allow"),
    ("web.search.bing.domains", "security.services.search.bing.domains"),
    (
        "web.search.duckduckgo.allow",
        "security.services.search.duckduckgo.allow",
    ),
    (
        "web.search.duckduckgo.domains",
        "security.services.search.duckduckgo.domains",
    ),
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
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create dir {}: {}", parent.display(), e))?;
    }
    let content = toml::to_string_pretty(file).map_err(|e| format!("failed to serialize settings: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(path, content.as_bytes())
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Load local UI settings and corp constraints from standard locations.
///
/// A settings.toml that does not load is reported and read as empty: this
/// view serves the settings UI, which must still open to fix it. What a VM
/// enforces is built from `load_policy_files`, which fails instead.
pub fn load_settings_and_corp_files() -> (SettingsFile, SettingsFile) {
    let settings = match settings_config_path() {
        Some(path) => load_local_settings_file(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "local settings");
            SettingsFile::default()
        }),
        None => SettingsFile::default(),
    };
    (settings, load_corp_files())
}

/// The user's settings.toml and the corp config a session's policy is built
/// from. settings.toml carries the user's security rules, so one that does
/// not load fails here: reading it as empty would run the VM without them.
pub fn load_policy_files() -> Result<(SettingsFile, SettingsFile), String> {
    let settings = match settings_config_path() {
        Some(path) => load_local_settings_file(&path)?,
        None => SettingsFile::default(),
    };
    Ok((settings, load_corp_files()))
}

/// Corp constraints from every available path (system + user-provisioned).
/// First path wins per-key (/etc/capsem/corp.toml overrides ~/.capsem/corp.toml).
pub fn load_corp_files() -> SettingsFile {
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
                // Image policy: first non-None wins, and replaces the user's.
                if corp.images.is_none() && file.images.is_some() {
                    corp.images = file.images;
                }
                // External rule files: first corp path wins per reference.
                corp.rule_files.merge_first_wins(file.rule_files);
                corp.corp_rule_files.merge_first_wins(file.corp_rule_files);
                if corp.refresh_policy.is_none() {
                    corp.refresh_policy = file.refresh_policy;
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
                // Provider config: first corp path wins per provider.
                for (provider_id, provider) in file.ai {
                    corp.ai.entry(provider_id).or_insert(provider);
                }
                for (plugin_id, plugin) in file.plugins {
                    corp.plugins.entry(plugin_id).or_insert(plugin);
                }
                if corp.network.dns.upstreams.is_empty() && !file.network.dns.upstreams.is_empty() {
                    corp.network.dns.upstreams = file.network.dns.upstreams;
                }
                for (target, override_config) in file.network.upstream_overrides {
                    corp.network.upstream_overrides.entry(target).or_insert(override_config);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "corp settings at {}", path.display());
            }
        }
    }
    corp
}

/// Whether the current process can write corp settings (always false).
pub fn can_write_corp_settings() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Unified settings response
// ---------------------------------------------------------------------------

/// Load the unified settings response (tree + issues) in one call.
pub fn load_settings_response() -> super::types::SettingsResponse {
    let (settings, corp) = load_settings_and_corp_files();
    let resolved = super::resolver::resolve_settings(&settings, &corp);
    super::types::SettingsResponse {
        tree: super::tree::build_settings_tree(&resolved),
        issues: super::lint::config_lint(&resolved),
    }
}

// ---------------------------------------------------------------------------
// Batch update
// ---------------------------------------------------------------------------

/// Batch-update multiple settings atomically.
///
/// Any registry setting id may be written: every one is the user's, unless
/// corp sets it, which locks it. Validates ALL changes upfront. If any change
/// is invalid (unknown id, corp-locked, invalid value), the entire batch is
/// rejected and nothing is written. Returns the applied setting ids.
pub fn batch_update_settings(changes: &HashMap<String, SettingValue>) -> Result<Vec<String>, String> {
    let mut raw = HashMap::new();
    for (id, value) in changes {
        let json = serde_json::to_value(value).map_err(|e| format!("failed to encode setting {id}: {e}"))?;
        raw.insert(id.clone(), json);
    }
    batch_update_settings_json(&raw)
}

pub fn batch_update_settings_json(changes: &HashMap<String, serde_json::Value>) -> Result<Vec<String>, String> {
    batch_update_settings_json_inner(changes)
}

fn batch_update_settings_json_inner(changes: &HashMap<String, serde_json::Value>) -> Result<Vec<String>, String> {
    use super::settings_metadata::setting_definitions;

    if changes.is_empty() {
        return Ok(vec![]);
    }

    let settings_path = settings_config_path().ok_or("HOME not set")?;
    let corp_path = corp_config_path();
    let mut settings_file = load_settings_document(&settings_path)?.unwrap_or_default();
    let corp_file = load_settings_file(&corp_path)?;
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

        // Every registry id is the user's to write; nothing else is.
        if !defs.iter().any(|d| d.id == *id) {
            errors.push(format!("unknown setting: {id}"));
            continue;
        }

        // An id corp sets is corp's, whatever its namespace.
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

    write_local_settings_document(&settings_path, &settings_file)?;
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
        capsem_proto::validate_file_path(path).map_err(|e| format!("invalid path for {id}: {e}"))?;
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
