/// Generic typed settings system with corp override.
///
/// Each setting has an id, name, description, type, category, default value,
/// and optional `enabled_by` pointer to a parent toggle. Settings are stored
/// in TOML files at:
///   - User: ~/.capsem/user.toml
///   - Corporate: /etc/capsem/corp.toml
///
/// Merge semantics: corp settings override user settings per-key.
/// User can only write user.toml. Corp file is read-only (MDM-distributed).
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::domain_policy::{Action, DomainPolicy};
use super::http_policy::{HttpPolicy, HttpRule};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// The data type of a setting (drives UI rendering).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    Text,
    Number,
    Password,
    Url,
    Email,
    ApiKey,
    Bool,
    /// File to write to a guest path. Value is `{ path, content }`.
    /// JSON files (.json extension) are validated on save.
    File,
}

/// A setting value (untagged for clean TOML serialization).
///
/// Variant order matters: `#[serde(untagged)]` tries variants top-to-bottom.
/// `File` (a table with `path` + `content`) must come before `Text` (a plain
/// string) so TOML tables like `{ path = "...", content = "..." }` deserialize
/// as `File` rather than failing on `Text`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(i64),
    File { path: String, content: String },
    Text(String),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            SettingValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            SettingValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_file(&self) -> Option<(&str, &str)> {
        match self {
            SettingValue::File { path, content } => Some((path, content)),
            _ => None,
        }
    }
}

/// Per-rule HTTP method permissions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[derive(Default)]
pub struct HttpMethodPermissions {
    /// Optional per-rule domain subset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// Path pattern (e.g., "/repos/*").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub get: bool,
    #[serde(default)]
    pub post: bool,
    #[serde(default)]
    pub put: bool,
    #[serde(default)]
    pub delete: bool,
    /// All methods not listed above.
    #[serde(default)]
    pub other: bool,
}


/// Structured metadata for a setting.
///
/// Note: `skip_serializing_if` is intentionally NOT used here. The frontend
/// accesses fields like `metadata.choices.length` directly, so omitting empty
/// fields from JSON would cause `undefined.length` TypeErrors in the UI.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingMetadata {
    /// Domain patterns for network settings.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Valid values for text choice settings.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Minimum for number settings.
    #[serde(default)]
    pub min: Option<i64>,
    /// Maximum for number settings.
    #[serde(default)]
    pub max: Option<i64>,
    /// HTTP rules (keyed by rule name).
    #[serde(default)]
    pub rules: HashMap<String, HttpMethodPermissions>,
    /// Env var name(s) to inject in the guest when this setting is non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    /// Whether this setting or section starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
}

/// Schema definition for a setting (loaded from defaults.toml at compile time).
pub struct SettingDef {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    /// Parent toggle ID (child is greyed out when parent is off).
    pub enabled_by: Option<String>,
    pub metadata: SettingMetadata,
}

/// A single stored setting entry in TOML.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SettingEntry {
    pub value: SettingValue,
    pub modified: String,
}

/// TOML file format for settings files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub settings: HashMap<String, SettingEntry>,
}

/// Where a setting's effective value came from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicySource {
    Default,
    User,
    Corp,
}

/// A fully resolved setting (for UI consumption).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolvedSetting {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    pub effective_value: SettingValue,
    pub source: PolicySource,
    pub modified: Option<String>,
    pub corp_locked: bool,
    pub enabled_by: Option<String>,
    /// Computed: is the parent toggle on? (true if no parent).
    pub enabled: bool,
    pub metadata: SettingMetadata,
    /// Whether this setting starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
}

// ---------------------------------------------------------------------------
// Guest config and VM settings
// ---------------------------------------------------------------------------

/// A file to write into the guest filesystem at boot.
#[derive(Debug, Clone)]
pub struct GuestFile {
    pub path: String,
    pub content: String,
    pub mode: u32,
}

/// Guest VM configuration (extracted from settings).
#[derive(Debug, Default, Clone)]
pub struct GuestConfig {
    pub env: Option<HashMap<String, String>>,
    pub files: Option<Vec<GuestFile>>,
}

/// VM resource settings (extracted from settings).
#[derive(Debug, Default, Clone)]
pub struct VmSettings {
    pub cpu_count: Option<u32>,
    pub scratch_disk_size_gb: Option<u32>,
    pub ram_gb: Option<u32>,
}

// ---------------------------------------------------------------------------
// Setting registry
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// TOML registry parser
// ---------------------------------------------------------------------------

/// A setting leaf as it appears in TOML. Core fields at top level,
/// metadata under `meta` sub-table.
#[derive(Deserialize, Debug)]
struct SettingDefToml {
    name: String,
    description: String,
    #[serde(rename = "type")]
    setting_type: SettingType,
    default: SettingValue,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    meta: SettingMetaToml,
}

#[derive(Deserialize, Debug, Default)]
struct SettingMetaToml {
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    min: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
    #[serde(default)]
    rules: HashMap<String, HttpMethodPermissions>,
    #[serde(default)]
    env_vars: Vec<String>,
}

/// Category/group metadata from TOML grouping nodes.
#[derive(Debug, Clone, Default)]
struct GroupMeta {
    /// Display name from nearest ancestor group with a `name` key.
    category: String,
    /// Parent toggle ID -- propagated to all child settings except the toggle.
    enabled_by: Option<String>,
    /// Whether the group starts collapsed in the UI.
    collapsed: bool,
}

/// Recursively walk the TOML table, collecting setting leaves.
///
/// A table with a `type` key is a leaf setting; otherwise it is a group node
/// whose `name`, `description`, `enabled_by`, and `collapsed` are group metadata.
fn collect_settings(
    path: &str,
    table: &toml::value::Table,
    parent: &GroupMeta,
    out: &mut Vec<SettingDef>,
) {
    if table.contains_key("type") {
        // Leaf setting -- deserialize the table into SettingDefToml
        let val = toml::Value::Table(table.clone());
        let def: SettingDefToml = val
            .try_into()
            .unwrap_or_else(|e| panic!("bad setting '{path}': {e}"));
        // Inherit enabled_by from parent group, unless this IS the toggle itself
        let enabled_by = if parent.enabled_by.as_deref() == Some(path) {
            None
        } else {
            parent.enabled_by.clone()
        };
        out.push(SettingDef {
            id: path.to_string(),
            category: parent.category.clone(),
            name: def.name,
            description: def.description,
            setting_type: def.setting_type,
            default_value: def.default,
            enabled_by,
            metadata: SettingMetadata {
                domains: def.meta.domains,
                choices: def.meta.choices,
                min: def.meta.min,
                max: def.meta.max,
                rules: def.meta.rules,
                env_vars: def.meta.env_vars,
                collapsed: def.collapsed,
            },
        });
        return;
    }

    // Group node -- extract category metadata, recurse into children
    let group = GroupMeta {
        category: table
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| parent.category.clone()),
        enabled_by: table
            .get("enabled_by")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| parent.enabled_by.clone()),
        collapsed: table
            .get("collapsed")
            .and_then(|v| v.as_bool())
            .unwrap_or(parent.collapsed),
    };

    for (key, val) in table {
        // Skip group metadata keys -- they are not child settings
        if matches!(
            key.as_str(),
            "name" | "description" | "enabled_by" | "collapsed"
        ) {
            continue;
        }
        if let Some(child) = val.as_table() {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            collect_settings(&child_path, child, &group, out);
        }
    }
}

const DEFAULTS_TOML: &str = include_str!("../../../../config/defaults.toml");

/// Returns the setting definitions parsed from the embedded defaults.toml.
pub fn setting_definitions() -> Vec<SettingDef> {
    let root: toml::Value =
        toml::from_str(DEFAULTS_TOML).expect("built-in defaults.toml is invalid");
    let settings = root
        .get("settings")
        .and_then(|v| v.as_table())
        .expect("defaults.toml missing [settings]");
    let mut defs = Vec::new();
    let root_group = GroupMeta::default();
    collect_settings("", settings, &root_group, &mut defs);
    defs
}

/// Returns an empty settings file (all defaults).
pub fn default_settings_file() -> SettingsFile {
    SettingsFile::default()
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// User config path: ~/.capsem/user.toml (overridable via CAPSEM_USER_CONFIG)
pub fn user_config_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CAPSEM_USER_CONFIG") {
        return Some(std::path::PathBuf::from(path));
    }
    dirs_path("HOME").map(|h| h.join(".capsem").join("user.toml"))
}

/// Corporate config path: /etc/capsem/corp.toml (overridable via CAPSEM_CORP_CONFIG)
pub fn corp_config_path() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("CAPSEM_CORP_CONFIG") {
        return std::path::PathBuf::from(path);
    }
    std::path::PathBuf::from("/etc/capsem/corp.toml")
}

fn dirs_path(env_var: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var).ok().map(std::path::PathBuf::from)
}

/// Load a settings file from disk. Returns empty SettingsFile if file missing.
pub fn load_settings_file(path: &Path) -> Result<SettingsFile, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|e| format!("failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SettingsFile::default()),
        Err(e) => Err(format!("failed to read {}: {}", path.display(), e)),
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
pub fn load_settings_files() -> (SettingsFile, SettingsFile) {
    let user = match user_config_path() {
        Some(path) => load_settings_file(&path).unwrap_or_else(|e| {
            tracing::warn!("user settings: {e}");
            SettingsFile::default()
        }),
        None => SettingsFile::default(),
    };

    let corp = load_settings_file(&corp_config_path()).unwrap_or_else(|e| {
        tracing::warn!("corp settings: {e}");
        SettingsFile::default()
    });

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

// ---------------------------------------------------------------------------
// Merge / resolve
// ---------------------------------------------------------------------------

/// Check if a setting is locked by corp.
pub fn is_setting_corp_locked(id: &str, corp: &SettingsFile) -> bool {
    corp.settings.contains_key(id)
}

/// Resolve all settings from user + corp files against the registry.
///
/// For each registered definition + any dynamic keys (guest.env.*),
/// corp overrides user, user overrides default.
/// Computes `enabled` from parent toggle.
pub fn resolve_settings(user: &SettingsFile, corp: &SettingsFile) -> Vec<ResolvedSetting> {
    let defs = setting_definitions();
    let mut resolved = Vec::new();

    for def in &defs {
        let (effective_value, source, modified) = resolve_value(&def.id, &def.default_value, user, corp);
        let corp_locked = corp.settings.contains_key(&def.id);

        resolved.push(ResolvedSetting {
            id: def.id.clone(),
            category: def.category.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            setting_type: def.setting_type,
            default_value: def.default_value.clone(),
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: def.enabled_by.clone(),
            enabled: true, // computed below
            metadata: def.metadata.clone(),
            collapsed: def.metadata.collapsed,
        });
    }

    // Dynamic settings: guest.env.* (not in registry)
    let dynamic_keys = collect_dynamic_keys(user, corp);
    for key in dynamic_keys {
        let default = SettingValue::Text(String::new());
        let (effective_value, source, modified) = resolve_value(&key, &default, user, corp);
        let corp_locked = corp.settings.contains_key(&key);

        resolved.push(ResolvedSetting {
            id: key.clone(),
            category: "Guest Environment".to_string(),
            name: key.strip_prefix("guest.env.").unwrap_or(&key).to_string(),
            description: format!("Guest environment variable: {}", key.strip_prefix("guest.env.").unwrap_or(&key)),
            setting_type: SettingType::Text,
            default_value: default,
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: None,
            enabled: true,
            metadata: SettingMetadata::default(),
            collapsed: false,
        });
    }

    // Compute enabled_by: look up parent toggle value
    compute_enabled(&mut resolved);

    resolved
}

/// Resolve a single setting value: corp > user > default.
fn resolve_value(
    id: &str,
    default: &SettingValue,
    user: &SettingsFile,
    corp: &SettingsFile,
) -> (SettingValue, PolicySource, Option<String>) {
    if let Some(entry) = corp.settings.get(id) {
        (entry.value.clone(), PolicySource::Corp, Some(entry.modified.clone()))
    } else if let Some(entry) = user.settings.get(id) {
        (entry.value.clone(), PolicySource::User, Some(entry.modified.clone()))
    } else {
        (default.clone(), PolicySource::Default, None)
    }
}

/// Collect all dynamic keys (guest.env.*) from both files.
fn collect_dynamic_keys(user: &SettingsFile, corp: &SettingsFile) -> Vec<String> {
    let mut keys: Vec<String> = user
        .settings
        .keys()
        .chain(corp.settings.keys())
        .filter(|k| k.starts_with("guest.env."))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Compute the `enabled` flag for each setting based on its parent toggle.
fn compute_enabled(settings: &mut [ResolvedSetting]) {
    // Build a lookup of id -> effective bool value
    let values: HashMap<String, bool> = settings
        .iter()
        .filter_map(|s| s.effective_value.as_bool().map(|b| (s.id.clone(), b)))
        .collect();

    for s in settings.iter_mut() {
        if let Some(ref parent_id) = s.enabled_by {
            s.enabled = values.get(parent_id.as_str()).copied().unwrap_or(false);
        }
        // else enabled stays true (set during construction)
    }
}

// ---------------------------------------------------------------------------
// Translation: settings -> policy objects
// ---------------------------------------------------------------------------

/// Parse a comma-separated domain list into trimmed individual entries.
fn parse_domain_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

/// Check if a candidate domain matches any corp-blocked pattern.
/// Uses the same wildcard logic as DomainPattern: suffix match for `*.foo.com`,
/// exact match otherwise.
fn corp_blocked_matches(candidate: &str, corp_blocked: &[String]) -> bool {
    let candidate = candidate.to_lowercase();
    for pattern in corp_blocked {
        let pattern = pattern.to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            if candidate.ends_with(&format!(".{suffix}")) || candidate == suffix {
                return true;
            }
        } else if candidate == pattern {
            return true;
        }
    }
    false
}

/// Build a DomainPolicy from resolved settings.
///
/// - Bool toggles with domain metadata (registries) -> allow/block those domains
/// - `.domains` Text settings -> allow/block parsed domain patterns
/// - Corp-locked-off services use UNION of default + effective domains for blocking
/// - Default action from network.default_action
pub fn settings_to_domain_policy(resolved: &[ResolvedSetting]) -> DomainPolicy {
    let mut allow_list: Vec<String> = Vec::new();
    let mut block_list: Vec<String> = Vec::new();

    // Existing: Bool toggles with domain metadata (registries)
    for s in resolved {
        if s.metadata.domains.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if enabled {
            allow_list.extend(s.metadata.domains.clone());
        } else {
            block_list.extend(s.metadata.domains.clone());
        }
    }

    // Pass 1: collect corp-blocked domain patterns from .domains settings.
    // When corp locks .allow to false, use UNION of default + effective so
    // user can't shrink the block list below defaults.
    let mut corp_blocked: Vec<String> = Vec::new();
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            let defaults = parse_domain_list(s.default_value.as_text().unwrap_or(""));
            let effective = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
            let mut all: Vec<String> = defaults;
            for d in effective {
                if !all.contains(&d) {
                    all.push(d);
                }
            }
            block_list.extend(all.clone());
            corp_blocked.extend(all);
        }
    }

    // Pass 2: process non-corp-locked .domains settings
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            continue; // Already handled in pass 1
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        if toggle_on {
            // Filter: don't allow domains that corp has blocked
            for d in domains {
                if corp_blocked_matches(&d, &corp_blocked) {
                    block_list.push(d); // Override: corp says no
                } else {
                    allow_list.push(d);
                }
            }
        } else {
            block_list.extend(domains);
        }
    }

    // Custom allow/block lists from network.custom_allow / network.custom_block.
    // Block takes priority over allow for overlapping domains.
    let custom_allow = resolved
        .iter()
        .find(|s| s.id == "network.custom_allow")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_block = resolved
        .iter()
        .find(|s| s.id == "network.custom_block")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_allow_domains = parse_domain_list(custom_allow);
    let custom_block_domains = parse_domain_list(custom_block);

    // Block beats allow: any domain in custom_block goes to block_list only.
    for d in &custom_allow_domains {
        if corp_blocked_matches(d, &corp_blocked) || corp_blocked_matches(d, &custom_block_domains) {
            block_list.push(d.clone());
        } else {
            allow_list.push(d.clone());
        }
    }
    block_list.extend(custom_block_domains);

    let default_action = resolved
        .iter()
        .find(|s| s.id == "network.default_action")
        .and_then(|s| s.effective_value.as_text())
        .and_then(|s| match s {
            "allow" => Some(Action::Allow),
            "deny" => Some(Action::Deny),
            _ => None,
        })
        .unwrap_or(Action::Deny);

    DomainPolicy::new(&allow_list, &block_list, default_action)
}

/// Build an HttpPolicy from resolved settings.
///
/// Generates HttpRules from setting metadata.rules for enabled toggles.
pub fn settings_to_http_policy(resolved: &[ResolvedSetting]) -> HttpPolicy {
    let domain_policy = settings_to_domain_policy(resolved);

    let mut http_rules: Vec<HttpRule> = Vec::new();

    for s in resolved {
        if s.metadata.rules.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if !enabled {
            continue;
        }

        // For each rule in metadata, generate HttpRules for the setting's domains
        let rule_domains: Vec<&str> = s.metadata.domains.iter().map(|d| d.as_str()).collect();

        for perms in s.metadata.rules.values() {
            let domains_for_rule = if perms.domains.is_empty() {
                rule_domains.clone()
            } else {
                perms.domains.iter().map(|d| d.as_str()).collect()
            };

            let path_pattern = perms.path.as_deref().unwrap_or("*").to_string();

            for domain in &domains_for_rule {
                // Skip wildcard domains for HTTP rules (they apply at domain level only)
                if domain.starts_with("*.") {
                    continue;
                }
                // Generate allow rules for each enabled method
                for (method, allowed) in [
                    ("GET", perms.get),
                    ("POST", perms.post),
                    ("PUT", perms.put),
                    ("DELETE", perms.delete),
                ] {
                    if allowed {
                        http_rules.push(HttpRule {
                            domain: domain.to_lowercase(),
                            method: method.to_string(),
                            path_pattern: path_pattern.clone(),
                            action: Action::Allow,
                        });
                    }
                }
            }
        }
    }

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "vm.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "vm.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    HttpPolicy::new(domain_policy, http_rules, log_bodies, max_body_capture)
}

/// Extract guest config from resolved settings.
///
/// Dynamic keys with prefix `guest.env.` become environment variables.
/// AI provider API keys and boot files are always injected when the key/value
/// is non-empty, regardless of the provider toggle. The toggle controls network
/// access (domain policy), not whether credentials are available in the VM.
/// This ensures the user can enable a provider at runtime without rebooting.
pub fn settings_to_guest_config(resolved: &[ResolvedSetting]) -> GuestConfig {
    use capsem_proto::{validate_env_key, validate_env_value, validate_file_path};

    let mut env = HashMap::new();
    let mut files = Vec::new();

    for s in resolved {
        let text_value = s.effective_value.as_text().unwrap_or("");

        // Provider allow toggles: inject CAPSEM_<PROVIDER>_ALLOWED=1|0
        // so the guest banner can show which AI tools are enabled.
        if s.setting_type == SettingType::Bool {
            let provider_env = match s.id.as_str() {
                "ai.anthropic.allow" => Some("CAPSEM_ANTHROPIC_ALLOWED"),
                "ai.openai.allow" => Some("CAPSEM_OPENAI_ALLOWED"),
                "ai.google.allow" => Some("CAPSEM_GOOGLE_ALLOWED"),
                _ => None,
            };
            if let Some(var_name) = provider_env {
                let val = if s.effective_value.as_bool().unwrap_or(false) { "1" } else { "0" };
                env.insert(var_name.to_string(), val.to_string());
            }
        }

        // Metadata-driven env var injection: if the setting declares env_vars
        // and the effective value is non-empty text, inject each env var.
        // For File values, the content is used as the env value.
        let env_text = match &s.effective_value {
            SettingValue::Text(t) => Some(t.as_str()),
            SettingValue::File { content, .. } => Some(content.as_str()),
            _ => None,
        };
        if let Some(ev) = env_text {
            if !s.metadata.env_vars.is_empty() && !ev.is_empty() {
                for var_name in &s.metadata.env_vars {
                    if let Err(e) = validate_env_key(var_name) {
                        tracing::warn!("skipping invalid env var from metadata: {e}");
                        continue;
                    }
                    if let Err(e) = validate_env_value(ev) {
                        tracing::warn!("skipping env var {var_name}: invalid value: {e}");
                        continue;
                    }
                    env.insert(var_name.clone(), ev.to_string());
                }
            }
        }

        // Boot files: File values with non-empty content.
        // Always inject if non-empty -- the allow toggle controls network
        // policy, not file availability.
        if let SettingValue::File { path: file_path, content: file_content } = &s.effective_value {
            if !file_content.is_empty() {
                if let Err(e) = validate_file_path(file_path) {
                    tracing::warn!("skipping boot file: {e}");
                    continue;
                }

                // Inject capsem MCP server into Claude/Gemini settings.json.
                // Pattern-match on the guest path (not the setting ID) since
                // the path is the source of truth for what the file represents.
                //
                // For Claude state (.claude.json), inject API key approval
                // and project trust so Claude starts without onboarding prompts.
                let content = if file_path.ends_with("/settings.json") {
                    inject_capsem_mcp_server(file_content)
                } else if file_path == "/root/.claude.json" {
                    if let Some(api_key) = env.get("ANTHROPIC_API_KEY") {
                        inject_api_key_approval(file_content, api_key)
                    } else {
                        file_content.clone()
                    }
                } else {
                    file_content.clone()
                };

                // Settings files may contain API keys or sensitive config --
                // restrict to owner-only (0o600) rather than world-readable.
                files.push(GuestFile {
                    path: file_path.clone(),
                    content,
                    mode: 0o600,
                });
            }
        }

        // Dynamic guest.env.* settings (not in registry)
        if let Some(var_name) = s.id.strip_prefix("guest.env.") {
            if !text_value.is_empty() {
                if let Err(e) = validate_env_key(var_name) {
                    tracing::warn!("skipping dynamic env var: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(text_value) {
                    tracing::warn!("skipping dynamic env var {var_name}: invalid value: {e}");
                    continue;
                }
                env.insert(var_name.to_string(), text_value.to_string());
            }
        }
    }

    GuestConfig {
        env: if env.is_empty() { None } else { Some(env) },
        files: if files.is_empty() { None } else { Some(files) },
    }
}

/// Inject the capsem MCP server entry into a settings.json string.
///
/// Parses the JSON, inserts `{"capsem": {"command": "/run/capsem-mcp-server"}}`
/// under `mcpServers`, preserving any user-provided entries. Returns the
/// original string unchanged if parsing fails.
fn inject_capsem_mcp_server(json_str: &str) -> String {
    let mut json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return json_str.to_string(),
    };

    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return json_str.to_string(),
    };

    let mcp_servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if let Some(servers) = mcp_servers.as_object_mut() {
        servers.insert(
            "capsem".to_string(),
            serde_json::json!({"command": "/run/capsem-mcp-server"}),
        );
    }

    serde_json::to_string(&json).unwrap_or_else(|_| json_str.to_string())
}

/// Inject `customApiKeyResponses` into Claude state JSON.
///
/// Pre-approves the last 20 characters of the API key so Claude Code doesn't
/// prompt the user to "trust" it on first use. Returns the original string
/// unchanged if parsing fails.
fn inject_api_key_approval(json_str: &str, api_key: &str) -> String {
    let mut json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return json_str.to_string(),
    };

    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return json_str.to_string(),
    };

    let key_suffix: String = if api_key.len() > 20 {
        api_key[api_key.len() - 20..].to_string()
    } else {
        api_key.to_string()
    };

    let responses = obj
        .entry("customApiKeyResponses")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(r) = responses.as_object_mut() {
        let approved = r
            .entry("approved")
            .or_insert_with(|| serde_json::json!([]));
        if let Some(arr) = approved.as_array_mut() {
            if !arr.iter().any(|v| v.as_str() == Some(&key_suffix)) {
                arr.push(serde_json::json!(key_suffix));
            }
        }
        r.entry("rejected").or_insert_with(|| serde_json::json!([]));
    }

    serde_json::to_string(&json).unwrap_or_else(|_| json_str.to_string())
}

/// Extract VM settings from resolved settings.
pub fn settings_to_vm_settings(resolved: &[ResolvedSetting]) -> VmSettings {
    let cpu_count = resolved
        .iter()
        .find(|s| s.id == "vm.cpu_count")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    let scratch_disk_size_gb = resolved
        .iter()
        .find(|s| s.id == "vm.scratch_disk_size_gb")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    let ram_gb = resolved
        .iter()
        .find(|s| s.id == "vm.ram_gb")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    VmSettings {
        cpu_count: Some(cpu_count.unwrap_or(4)),
        scratch_disk_size_gb: Some(scratch_disk_size_gb.unwrap_or(16)),
        ram_gb: Some(ram_gb.unwrap_or(4)),
    }
}

// ---------------------------------------------------------------------------
// High-level entry points
// ---------------------------------------------------------------------------

/// Load and merge settings, then build an HttpPolicy.
pub fn load_merged_policy() -> HttpPolicy {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_http_policy(&resolved)
}

/// Build a `DomainPolicy` from merged settings.
///
/// Convenience wrapper matching the `load_merged_network_policy()` pattern.
/// Used by the MCP gateway to check built-in HTTP tool domains.
pub fn load_merged_domain_policy() -> DomainPolicy {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_domain_policy(&resolved)
}

/// Build a `NetworkPolicy` (new policy engine) from merged settings.
///
/// Bridges settings into per-domain read/write rules:
/// - Disabled toggles with domains get read=false, write=false
/// - Enabled toggles with domains get read=true, write=true
/// - Default action maps to default_allow_read and default_allow_write
pub fn load_merged_network_policy() -> super::policy::NetworkPolicy {
    use super::policy::{DomainMatcher, NetworkPolicy, PolicyRule};

    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);

    let mut rules = Vec::new();

    // Build rules from settings with domain metadata (registries)
    for s in &resolved {
        if s.metadata.domains.is_empty() || s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        for domain in &s.metadata.domains {
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    // Build rules from .domains text settings (AI providers)
    // Corp block enforcement: same two-pass approach as settings_to_domain_policy
    let mut corp_blocked: Vec<String> = Vec::new();
    for s in &resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            let defaults = parse_domain_list(s.default_value.as_text().unwrap_or(""));
            let effective = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
            let mut all: Vec<String> = defaults;
            for d in effective {
                if !all.contains(&d) {
                    all.push(d);
                }
            }
            for domain in &all {
                rules.push(PolicyRule {
                    matcher: DomainMatcher::parse(domain),
                    allow_read: false,
                    allow_write: false,
                });
            }
            corp_blocked.extend(all);
        }
    }
    for s in &resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            continue;
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        for domain in &domains {
            let blocked = corp_blocked_matches(domain, &corp_blocked);
            let enabled = toggle_on && !blocked;
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    // Custom allow/block lists: same pattern as settings_to_domain_policy
    let custom_allow_text = resolved
        .iter()
        .find(|s| s.id == "network.custom_allow")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_block_text = resolved
        .iter()
        .find(|s| s.id == "network.custom_block")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_allow_domains = parse_domain_list(custom_allow_text);
    let custom_block_domains = parse_domain_list(custom_block_text);

    for domain in &custom_allow_domains {
        let blocked = corp_blocked_matches(domain, &corp_blocked)
            || corp_blocked_matches(domain, &custom_block_domains);
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(domain),
            allow_read: !blocked,
            allow_write: !blocked,
        });
    }
    for domain in &custom_block_domains {
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(domain),
            allow_read: false,
            allow_write: false,
        });
    }

    let default_action = resolved
        .iter()
        .find(|s| s.id == "network.default_action")
        .and_then(|s| s.effective_value.as_text())
        .map(|s| s == "allow")
        .unwrap_or(false);

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "vm.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(true);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "vm.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    let max_body_size = resolved
        .iter()
        .find(|s| s.id == "network.max_body_size_mb")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(1024) as u64;

    let mut policy = NetworkPolicy::new(rules, default_action, default_action);
    policy.log_bodies = log_bodies;
    policy.max_body_capture = max_body_capture;
    policy.max_body_size_mb = max_body_size;
    policy
}

/// Load and merge guest config from standard locations.
pub fn load_merged_guest_config() -> GuestConfig {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_guest_config(&resolved)
}

/// Load and merge VM settings from standard locations.
pub fn load_merged_vm_settings() -> VmSettings {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_vm_settings(&resolved)
}

/// Load all resolved settings (for UI).
pub fn load_merged_settings() -> Vec<ResolvedSetting> {
    let (user, corp) = load_settings_files();
    resolve_settings(&user, &corp)
}

// ---------------------------------------------------------------------------
// Config lint
// ---------------------------------------------------------------------------

/// A single config validation issue.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ConfigIssue {
    /// Setting ID (e.g. "ai.anthropic.api_key").
    pub id: String,
    /// "error" | "warning".
    pub severity: String,
    /// Human-readable message shown in the UI.
    pub message: String,
}

/// Validate all resolved settings and return a list of issues.
///
/// Checks: number ranges, choice validity, JSON file content, API key format,
/// enabled-provider-with-empty-key, nul bytes in text.
pub fn config_lint(resolved: &[ResolvedSetting]) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();

    // Build a lookup for toggle values (for enabled-provider checks).
    let toggle_values: HashMap<String, bool> = resolved
        .iter()
        .filter(|s| s.setting_type == SettingType::Bool)
        .filter_map(|s| s.effective_value.as_bool().map(|b| (s.id.clone(), b)))
        .collect();

    for s in resolved {
        let text_value = match &s.effective_value {
            SettingValue::Text(t) => Some(t.as_str()),
            _ => None,
        };

        // -- Nul byte check (all text values) --
        if let Some(text) = text_value {
            if text.contains('\0') {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: value contains invalid characters", s.id),
                });
            }
        }

        // -- Number range --
        if s.setting_type == SettingType::Number {
            if let Some(n) = s.effective_value.as_number() {
                if let Some(min) = s.metadata.min {
                    if n < min {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!(
                                "{}: value {} is below minimum {}",
                                s.id, n, min
                            ),
                        });
                    }
                }
                if let Some(max) = s.metadata.max {
                    if n > max {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!(
                                "{}: value {} exceeds maximum {}",
                                s.id, n, max
                            ),
                        });
                    }
                }
            }
        }

        // -- Choice validation --
        if !s.metadata.choices.is_empty() {
            if let Some(text) = text_value {
                if !s.metadata.choices.iter().any(|c| c == text) {
                    issues.push(ConfigIssue {
                        id: s.id.clone(),
                        severity: "error".into(),
                        message: format!(
                            "{}: '{}' is not a valid choice ({})",
                            s.id,
                            text,
                            s.metadata.choices.join(", ")
                        ),
                    });
                }
            }
        }

        // -- File value validation (path + JSON content) --
        if let SettingValue::File { path: file_path, content: file_content } = &s.effective_value {
            // Path validation
            if !file_path.starts_with('/') {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: file path must be absolute", s.id),
                });
            }
            if file_path.contains("..") {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: file path must not contain '..'", s.id),
                });
            }
            if !file_path.starts_with("/root/") && !file_path.starts_with("/root/.") && !file_path.starts_with("/etc/") {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "warning".into(),
                    message: format!("{}: unusual file path (expected under /root/ or /etc/)", s.id),
                });
            }
            // JSON content validation for .json paths
            if file_path.ends_with(".json") && !file_content.is_empty() {
                match serde_json::from_str::<serde_json::Value>(file_content) {
                    Ok(val) => {
                        if !val.is_object() && !val.is_array() {
                            issues.push(ConfigIssue {
                                id: s.id.clone(),
                                severity: "warning".into(),
                                message: format!(
                                    "{}: JSON parsed but is not an object",
                                    s.id
                                ),
                            });
                        }
                    }
                    Err(e) => {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!("{}: invalid JSON -- {}", s.id, e),
                        });
                    }
                }
            }
        }

        // -- API key whitespace check --
        if s.setting_type == SettingType::ApiKey {
            if let Some(text) = text_value {
                if !text.is_empty() {
                    if text.contains(' ') || text.contains('\n') || text.contains('\r') || text.contains('\t') {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "warning".into(),
                            message: format!(
                                "{}: key contains whitespace -- check for copy-paste errors",
                                s.id
                            ),
                        });
                    }
                }
            }
        }

        // -- Enabled provider with empty API key --
        if s.setting_type == SettingType::ApiKey {
            if let Some(text) = text_value {
                if text.trim().is_empty() {
                    // Check if the parent toggle is on
                    if let Some(ref parent_id) = s.enabled_by {
                        if toggle_values.get(parent_id).copied().unwrap_or(false) {
                            issues.push(ConfigIssue {
                                id: s.id.clone(),
                                severity: "warning".into(),
                                message: format!(
                                    "{}: provider is enabled but API key is empty",
                                    s.id
                                ),
                            });
                        }
                    }
                }
            }
        }

        // -- URL validation --
        if s.setting_type == SettingType::Url {
            if let Some(text) = text_value {
                if !text.is_empty()
                    && !text.starts_with("http://")
                    && !text.starts_with("https://")
                {
                    issues.push(ConfigIssue {
                        id: s.id.clone(),
                        severity: "warning".into(),
                        message: format!("{}: not a valid URL", s.id),
                    });
                }
            }
        }
    }

    issues
}

/// Run lint on current merged settings.
pub fn load_merged_lint() -> Vec<ConfigIssue> {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    config_lint(&resolved)
}

// ---------------------------------------------------------------------------
// Settings tree
// ---------------------------------------------------------------------------

/// A settings tree node: either a group of children or a leaf setting.
///
/// Serialized with `tag = "kind"` so JSON includes `{"kind": "group", ...}` or
/// `{"kind": "leaf", ...}`.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum SettingsNode {
    #[serde(rename = "group")]
    Group {
        key: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_by: Option<String>,
        collapsed: bool,
        children: Vec<SettingsNode>,
    },
    #[serde(rename = "leaf")]
    Leaf(ResolvedSetting),
}

/// Build a settings tree mirroring the TOML hierarchy with resolved values at leaves.
///
/// Walks the TOML structure like `collect_settings` but produces nested
/// `SettingsNode::Group` / `SettingsNode::Leaf` instead of flattening.
fn build_tree_from_table(
    path: &str,
    table: &toml::value::Table,
    parent_enabled_by: &Option<String>,
    parent_collapsed: bool,
    resolved_map: &HashMap<String, ResolvedSetting>,
) -> Vec<SettingsNode> {
    // Check if this is a leaf (has "type" key)
    if table.contains_key("type") {
        if let Some(resolved) = resolved_map.get(path) {
            return vec![SettingsNode::Leaf(resolved.clone())];
        }
        return vec![];
    }

    // Group node
    let group_name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let group_description = table
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let group_enabled_by = table
        .get("enabled_by")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| parent_enabled_by.clone());
    let group_collapsed = table
        .get("collapsed")
        .and_then(|v| v.as_bool())
        .unwrap_or(parent_collapsed);

    let mut children = Vec::new();
    for (key, val) in table {
        if matches!(
            key.as_str(),
            "name" | "description" | "enabled_by" | "collapsed"
        ) {
            continue;
        }
        if let Some(child_table) = val.as_table() {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let child_nodes = build_tree_from_table(
                &child_path,
                child_table,
                &group_enabled_by,
                group_collapsed,
                resolved_map,
            );
            children.extend(child_nodes);
        }
    }

    // If we have a group name (this is a named group), wrap children.
    // Top-level call (path is empty) skips wrapping.
    if let Some(name) = group_name {
        if !path.is_empty() {
            return vec![SettingsNode::Group {
                key: path.to_string(),
                name,
                description: group_description,
                enabled_by: if parent_enabled_by.is_some() {
                    // Sub-group inherits parent enabled_by but the group node
                    // itself should show its own enabled_by.
                    group_enabled_by
                } else {
                    table
                        .get("enabled_by")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                },
                collapsed: group_collapsed,
                children,
            }];
        }
    }

    children
}

/// Build the full settings tree from defaults.toml + resolved values.
///
/// Returns top-level groups (AI Providers, Package Registries, etc.).
/// Dynamic `guest.env.*` settings are appended to the Guest Environment group.
pub fn build_settings_tree(resolved: &[ResolvedSetting]) -> Vec<SettingsNode> {
    let root: toml::Value =
        toml::from_str(DEFAULTS_TOML).expect("built-in defaults.toml is invalid");
    let settings = root
        .get("settings")
        .and_then(|v| v.as_table())
        .expect("defaults.toml missing [settings]");

    // Build a lookup from ID to resolved setting.
    let resolved_map: HashMap<String, ResolvedSetting> = resolved
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut tree = Vec::new();
    for (key, val) in settings {
        if let Some(child_table) = val.as_table() {
            let nodes = build_tree_from_table(
                key,
                child_table,
                &None,
                false,
                &resolved_map,
            );
            tree.extend(nodes);
        }
    }

    // Append dynamic guest.env.* settings to the Guest Environment group.
    let dynamic_envs: Vec<&ResolvedSetting> = resolved
        .iter()
        .filter(|s| s.id.starts_with("guest.env.") && !resolved_map.contains_key(&s.id)
            || (s.id.starts_with("guest.env.") && s.category == "Guest Environment" && setting_definitions().iter().all(|d| d.id != s.id)))
        .collect();

    if !dynamic_envs.is_empty() {
        // Find the Guest Environment group and append
        fn append_dynamic(nodes: &mut Vec<SettingsNode>, envs: &[&ResolvedSetting]) {
            for node in nodes.iter_mut() {
                if let SettingsNode::Group { name, children, .. } = node {
                    if name == "Guest Environment" {
                        for env in envs {
                            children.push(SettingsNode::Leaf((*env).clone()));
                        }
                        return;
                    }
                    append_dynamic(children, envs);
                }
            }
        }
        append_dynamic(&mut tree, &dynamic_envs);
    }

    tree
}

/// Load settings tree from standard locations.
pub fn load_settings_tree() -> Vec<SettingsNode> {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    build_settings_tree(&resolved)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_file() -> SettingsFile {
        SettingsFile::default()
    }

    fn now_str() -> String {
        "2026-02-25T00:00:00Z".to_string()
    }

    fn file_with(entries: Vec<(&str, SettingValue)>) -> SettingsFile {
        let mut settings = HashMap::new();
        for (id, value) in entries {
            settings.insert(id.to_string(), SettingEntry {
                value,
                modified: now_str(),
            });
        }
        SettingsFile { settings }
    }

    // -----------------------------------------------------------------------
    // A: Corp override (7)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_override_bool() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_text() {
        let user = file_with(vec![("network.default_action", SettingValue::Text("allow".into()))]);
        let corp = file_with(vec![("network.default_action", SettingValue::Text("deny".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("deny".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_number() {
        let user = file_with(vec![("vm.max_body_capture", SettingValue::Number(8192))]);
        let corp = file_with(vec![("vm.max_body_capture", SettingValue::Number(1024))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "vm.max_body_capture").unwrap();
        assert_eq!(s.effective_value, SettingValue::Number(1024));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_api_key() {
        let user = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_guest_env() {
        let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
        let corp = file_with(vec![("guest.env.EDITOR", SettingValue::Text("nano".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "guest.env.EDITOR").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("nano".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_mixed_categories() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.log_bodies", SettingValue::Bool(true)),
            ("appearance.dark_mode", SettingValue::Bool(false)),
        ]);
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("vm.log_bodies", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);

        let ai = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(ai.effective_value, SettingValue::Bool(false));
        assert_eq!(ai.source, PolicySource::Corp);

        let log = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(log.effective_value, SettingValue::Bool(false));
        assert_eq!(log.source, PolicySource::Corp);

        // appearance.dark_mode not in corp -> user value
        let dark = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
        assert_eq!(dark.effective_value, SettingValue::Bool(false));
        assert_eq!(dark.source, PolicySource::User);
    }

    #[test]
    fn corp_overrides_all_registry_toggles() {
        let corp = file_with(vec![
            ("registry.github.allow", SettingValue::Bool(false)),
            ("registry.npm.allow", SettingValue::Bool(false)),
            ("registry.pypi.allow", SettingValue::Bool(false)),
            ("registry.crates.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&empty_file(), &corp);
        for s in &resolved {
            if s.id.starts_with("registry.") && s.id.ends_with(".allow") {
                assert_eq!(s.effective_value, SettingValue::Bool(false), "failed for {}", s.id);
                assert_eq!(s.source, PolicySource::Corp);
            }
        }
    }

    // -----------------------------------------------------------------------
    // B: User cannot expand (3)
    // -----------------------------------------------------------------------

    #[test]
    fn user_cannot_enable_blocked_provider() {
        // Corp blocks anthropic, user tries to enable
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_change_corp_default_action() {
        let user = file_with(vec![("network.default_action", SettingValue::Text("allow".into()))]);
        let corp = file_with(vec![("network.default_action", SettingValue::Text("deny".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("deny".into()));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_override_corp_api_key() {
        let user = file_with(vec![("ai.openai.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.openai.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
        assert!(s.corp_locked);
    }

    // -----------------------------------------------------------------------
    // C: User isolation (4)
    // -----------------------------------------------------------------------

    #[test]
    fn can_write_corp_is_always_false() {
        assert!(!can_write_corp_settings());
    }

    #[test]
    fn write_user_settings_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_user.toml");
        let file = file_with(vec![("vm.log_bodies", SettingValue::Bool(true))]);
        write_settings_file(&path, &file).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_user_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.toml");
        let file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.max_body_capture", SettingValue::Number(8192)),
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        write_settings_file(&path, &file).unwrap();
        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(file.settings.len(), loaded.settings.len());
        for (key, entry) in &file.settings {
            let loaded_entry = loaded.settings.get(key).unwrap();
            assert_eq!(entry.value, loaded_entry.value, "mismatch for {key}");
        }
    }

    #[test]
    fn write_user_settings_preserves_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preserve.toml");
        let mut file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.log_bodies", SettingValue::Bool(false)),
        ]);
        write_settings_file(&path, &file).unwrap();

        // Update one setting
        file.settings.get_mut("vm.log_bodies").unwrap().value = SettingValue::Bool(true);
        write_settings_file(&path, &file).unwrap();

        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(
            loaded.settings.get("ai.anthropic.allow").unwrap().value,
            SettingValue::Bool(true),
        );
        assert_eq!(
            loaded.settings.get("vm.log_bodies").unwrap().value,
            SettingValue::Bool(true),
        );
    }

    // -----------------------------------------------------------------------
    // D: Defaults (5)
    // -----------------------------------------------------------------------

    #[test]
    fn default_settings_file_is_empty() {
        let file = default_settings_file();
        assert!(file.settings.is_empty());
    }

    #[test]
    fn default_resolve_has_all_definitions() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let defs = setting_definitions();
        for def in &defs {
            assert!(
                resolved.iter().any(|s| s.id == def.id),
                "missing definition: {}",
                def.id,
            );
        }
    }

    #[test]
    fn default_ai_providers_blocked() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &["ai.anthropic.allow", "ai.openai.allow"] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(false), "expected {id} to be false");
        }
    }

    #[test]
    fn default_google_ai_allowed() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.google.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(true), "expected ai.google.allow to be true");
    }

    #[test]
    fn default_registries_allowed() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &[
            "registry.github.allow",
            "registry.npm.allow",
            "registry.pypi.allow",
            "registry.crates.allow",
        ] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(true), "expected {id} to be true");
        }
    }

    #[test]
    fn default_network_session_appearance() {
        let resolved = resolve_settings(&empty_file(), &empty_file());

        let da = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(da.effective_value, SettingValue::Text("deny".into()));

        let lb = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(lb.effective_value, SettingValue::Bool(false));

        let mbc = resolved.iter().find(|s| s.id == "vm.max_body_capture").unwrap();
        assert_eq!(mbc.effective_value, SettingValue::Number(4096));

        let rd = resolved.iter().find(|s| s.id == "vm.retention_days").unwrap();
        assert_eq!(rd.effective_value, SettingValue::Number(30));

        let dm = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
        assert_eq!(dm.effective_value, SettingValue::Bool(true));

        let fs = resolved.iter().find(|s| s.id == "appearance.font_size").unwrap();
        assert_eq!(fs.effective_value, SettingValue::Number(14));
    }

    // -----------------------------------------------------------------------
    // E: Definitions (4)
    // -----------------------------------------------------------------------

    #[test]
    fn definitions_have_unique_ids() {
        let defs = setting_definitions();
        let mut ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "duplicate setting IDs found");
    }

    #[test]
    fn definitions_have_nonempty_descriptions() {
        for def in setting_definitions() {
            assert!(!def.description.is_empty(), "empty description for {}", def.id);
            assert!(!def.name.is_empty(), "empty name for {}", def.id);
        }
    }

    #[test]
    fn registry_toggles_have_domain_metadata() {
        let defs = setting_definitions();
        for def in &defs {
            if def.id.starts_with("registry.") && def.id.ends_with(".allow") {
                assert!(
                    !def.metadata.domains.is_empty(),
                    "toggle {} has no domain metadata",
                    def.id,
                );
            }
        }
    }

    #[test]
    fn ai_providers_have_domains_settings() {
        let defs = setting_definitions();
        for prefix in &["ai.anthropic", "ai.openai", "ai.google"] {
            let domains_id = format!("{prefix}.domains");
            let def = defs.iter().find(|d| d.id == domains_id);
            assert!(def.is_some(), "missing {domains_id} setting");
            let def = def.unwrap();
            assert_eq!(def.setting_type, SettingType::Text);
            assert!(def.enabled_by.is_some());
        }
    }

    #[test]
    fn choice_settings_have_choices_metadata() {
        let defs = setting_definitions();
        let da = defs.iter().find(|d| d.id == "network.default_action").unwrap();
        assert!(!da.metadata.choices.is_empty());
        assert!(da.metadata.choices.contains(&"allow".to_string()));
        assert!(da.metadata.choices.contains(&"deny".to_string()));
    }

    // -----------------------------------------------------------------------
    // F: Source tracking (6)
    // -----------------------------------------------------------------------

    #[test]
    fn source_default() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Default);
        assert!(s.modified.is_none());
    }

    #[test]
    fn source_user() {
        let user = file_with(vec![("vm.log_bodies", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::User);
        assert!(s.modified.is_some());
    }

    #[test]
    fn source_corp() {
        let corp = file_with(vec![("vm.log_bodies", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let s = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Corp);
        assert!(s.modified.is_some());
    }

    #[test]
    fn source_corp_beats_user() {
        let user = file_with(vec![("vm.log_bodies", SettingValue::Bool(true))]);
        let corp = file_with(vec![("vm.log_bodies", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Corp);
        assert_eq!(s.effective_value, SettingValue::Bool(false));
    }

    #[test]
    fn source_dynamic_guest_env() {
        let user = file_with(vec![("guest.env.FOO", SettingValue::Text("bar".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "guest.env.FOO").unwrap();
        assert_eq!(s.source, PolicySource::User);
        assert_eq!(s.category, "Guest Environment");
    }

    #[test]
    fn is_setting_corp_locked_test() {
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        assert!(is_setting_corp_locked("ai.anthropic.allow", &corp));
        assert!(!is_setting_corp_locked("ai.openai.allow", &corp));
    }

    // -----------------------------------------------------------------------
    // G: enabled_by (4)
    // -----------------------------------------------------------------------

    #[test]
    fn enabled_by_parent_on_child_enabled() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert!(child.enabled);
        assert_eq!(child.enabled_by, Some("ai.anthropic.allow".to_string()));
    }

    #[test]
    fn enabled_by_parent_off_child_disabled() {
        // Default: ai.anthropic.allow is false
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert!(!child.enabled);
    }

    #[test]
    fn enabled_by_none_always_enabled() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "vm.log_bodies").unwrap();
        assert!(s.enabled);
        assert!(s.enabled_by.is_none());
    }

    #[test]
    fn enabled_by_chain_not_supported() {
        // Only one level of enabled_by is supported.
        // A child with enabled_by pointing to a non-existent parent is disabled.
        let mut user = empty_file();
        // Simulate a setting with enabled_by pointing to a non-bool setting
        // This should result in enabled=false since the parent can't be resolved to bool
        let resolved = resolve_settings(&user, &empty_file());

        // All api_key settings have enabled_by pointing to .allow toggles
        // When the toggle is off (default for AI), api_key is disabled
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(!key.enabled);

        // Turn on the toggle -> key is enabled
        user = file_with(vec![("ai.openai.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(key.enabled);
    }

    // -----------------------------------------------------------------------
    // H: Translation (5)
    // -----------------------------------------------------------------------

    #[test]
    fn settings_to_domain_policy_defaults() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        // Registries enabled by default -> domains allowed
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("pypi.org");
        assert_eq!(action, Action::Allow);

        // Anthropic/OpenAI disabled by default -> domains blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);

        // Google AI enabled by default -> domains allowed
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);

        // Unknown domains denied
        let (action, _) = dp.evaluate("example.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_off_registry() {
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_on_provider() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn settings_to_guest_config_from_dynamic() {
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
            ("guest.env.TERM", SettingValue::Text("xterm".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        assert_eq!(env.get("TERM").unwrap(), "xterm");
    }

    #[test]
    fn settings_to_http_policy_from_metadata_rules() {
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let hp = settings_to_http_policy(&resolved);

        // github.com is allowed at domain level
        let d = hp.evaluate_domain("github.com");
        assert_eq!(d.action, Action::Allow);

        // GET should be allowed (from metadata rules)
        let d = hp.evaluate_request("github.com", "GET", "/repos/foo");
        assert_eq!(d.action, Action::Allow);
    }

    // -----------------------------------------------------------------------
    // I: Roundtrip + edge cases (4)
    // -----------------------------------------------------------------------

    #[test]
    fn settings_file_toml_roundtrip() {
        let file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.max_body_capture", SettingValue::Number(8192)),
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: r#"{"key":"value"}"#.into(),
            }),
        ]);
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: SettingsFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(file.settings.len(), parsed.settings.len());
        for (key, entry) in &file.settings {
            assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
        }
    }

    #[test]
    fn settings_file_disk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk_roundtrip.toml");
        let file = file_with(vec![
            ("registry.github.allow", SettingValue::Bool(true)),
            ("appearance.font_size", SettingValue::Number(16)),
        ]);
        write_settings_file(&path, &file).unwrap();
        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(file, loaded);
    }

    #[test]
    fn empty_files_use_defaults() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for s in &resolved {
            assert_eq!(s.source, PolicySource::Default, "non-default source for {}", s.id);
        }
    }

    #[test]
    fn invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "{{{{not valid").unwrap();
        let result = load_settings_file(&path);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // TOML parsing from raw strings (M)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_real_user_toml_format() {
        // This is the exact format a real user.toml has on disk.
        let toml_str = r#"
[settings]
"ai.google.api_key" = { value = "AIzaSyTest1234", modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.allow" = { value = true, modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-test-key", modified = "2026-02-25T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse real user.toml format");
        assert_eq!(file.settings.len(), 3);
        assert_eq!(
            file.settings["ai.google.api_key"].value,
            SettingValue::Text("AIzaSyTest1234".into()),
        );
        assert_eq!(
            file.settings["ai.anthropic.allow"].value,
            SettingValue::Bool(true),
        );
        assert_eq!(
            file.settings["ai.anthropic.api_key"].value,
            SettingValue::Text("sk-ant-test-key".into()),
        );
    }

    #[test]
    fn parse_toml_mixed_value_types() {
        let toml_str = r#"
[settings]
"vm.log_bodies" = { value = true, modified = "2026-01-01T00:00:00Z" }
"vm.max_body_capture" = { value = 8192, modified = "2026-01-01T00:00:00Z" }
"network.default_action" = { value = "deny", modified = "2026-01-01T00:00:00Z" }
"appearance.font_size" = { value = 16, modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse mixed types");
        assert_eq!(file.settings["vm.log_bodies"].value, SettingValue::Bool(true));
        assert_eq!(file.settings["vm.max_body_capture"].value, SettingValue::Number(8192));
        assert_eq!(file.settings["network.default_action"].value, SettingValue::Text("deny".into()));
        assert_eq!(file.settings["appearance.font_size"].value, SettingValue::Number(16));
    }

    #[test]
    fn parse_toml_empty_settings_table() {
        let toml_str = "[settings]\n";
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse empty table");
        assert!(file.settings.is_empty());
    }

    #[test]
    fn parse_toml_completely_empty() {
        let file: SettingsFile = toml::from_str("").expect("should parse empty string");
        assert!(file.settings.is_empty());
    }

    #[test]
    fn parse_toml_missing_modified_fails() {
        // SettingEntry requires both value and modified
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "missing 'modified' field should fail");
    }

    #[test]
    fn parse_toml_missing_value_fails() {
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "missing 'value' field should fail");
    }

    #[test]
    fn parse_toml_extra_fields_ignored() {
        // TOML with extra unknown fields in the entry should still parse
        // (serde default behavior: ignore unknown fields)
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z", extra = "ignored" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        // By default serde does NOT deny unknown fields, so this should succeed.
        // If it fails, SettingEntry is using deny_unknown_fields.
        assert!(result.is_ok(), "extra fields should be ignored: {:?}", result.err());
    }

    #[test]
    fn parse_toml_wrong_value_type_fails() {
        // value is an array -- not a valid SettingValue variant
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = [1, 2, 3], modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "array value should fail deserialization");
    }

    #[test]
    fn parse_toml_unquoted_dotted_keys() {
        // In TOML, unquoted dotted keys create nested tables, not flat keys.
        // This is a common mistake: ai.anthropic.allow = { ... } creates
        // [ai] -> [anthropic] -> allow = { ... }, NOT a flat key "ai.anthropic.allow".
        let toml_str = r#"
[settings]
ai.anthropic.allow = { value = true, modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        // This should fail because the nested table structure does not match
        // HashMap<String, SettingEntry>.
        assert!(result.is_err(), "unquoted dotted keys should fail (creates nested tables)");
    }

    #[test]
    fn parse_toml_guest_env_keys() {
        let toml_str = r#"
[settings]
"guest.env.EDITOR" = { value = "vim", modified = "2026-01-01T00:00:00Z" }
"guest.env.TERM" = { value = "xterm-256color", modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse guest env");
        assert_eq!(file.settings.len(), 2);
        assert_eq!(
            file.settings["guest.env.EDITOR"].value,
            SettingValue::Text("vim".into()),
        );
    }

    #[test]
    fn parse_toml_api_key_with_special_chars() {
        // API keys often have dashes, underscores, and mixed case
        let toml_str = r#"
[settings]
"ai.anthropic.api_key" = { value = "sk-ant-api03-ABCD_1234-efgh-5678", modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse API key with special chars");
        assert_eq!(
            file.settings["ai.anthropic.api_key"].value,
            SettingValue::Text("sk-ant-api03-ABCD_1234-efgh-5678".into()),
        );
    }

    #[test]
    fn parse_toml_resolves_with_api_key_type() {
        // Parse from raw TOML, then resolve -- api_key settings must have
        // setting_type == ApiKey, not Text.
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.setting_type, SettingType::ApiKey, "api_key settings must have ApiKey type");
        assert_eq!(s.effective_value, SettingValue::Text("sk-test".into()));
    }

    #[test]
    fn parse_toml_serialized_format_roundtrips() {
        // Verify that toml::to_string_pretty output parses back correctly
        let file = file_with(vec![
            ("ai.google.api_key", SettingValue::Text("AIzaTest".into())),
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.max_body_capture", SettingValue::Number(4096)),
        ]);
        let serialized = toml::to_string_pretty(&file).unwrap();
        let parsed: SettingsFile = toml::from_str(&serialized)
            .unwrap_or_else(|e| panic!("failed to re-parse serialized TOML:\n{serialized}\nerror: {e}"));
        assert_eq!(file.settings.len(), parsed.settings.len());
        for (key, entry) in &file.settings {
            assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
        }
    }

    #[test]
    fn json_metadata_fields_present_when_empty() {
        // SettingMetadata uses skip_serializing_if = "Vec::is_empty" etc.
        // If empty fields are omitted from JSON, the JS frontend will crash
        // because it accesses metadata.choices.length (undefined.length -> TypeError).
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let json = serde_json::to_string(&resolved).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

        // Find a setting with empty metadata (e.g., api_key settings)
        let api_key = parsed.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
            .unwrap();
        let meta = &api_key["metadata"];

        // These fields MUST be present in JSON (even when empty) or the
        // frontend will crash with undefined.length errors.
        assert!(
            meta.get("choices").is_some(),
            "metadata.choices must be present in JSON (got: {meta})"
        );
        assert!(
            meta.get("domains").is_some(),
            "metadata.domains must be present in JSON (got: {meta})"
        );
    }

    #[test]
    fn resolved_settings_json_serialization() {
        // Tauri sends settings as JSON to the frontend. Verify the full
        // pipeline: parse TOML -> resolve -> serialize to JSON -> has setting_type.
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let json = serde_json::to_string(&resolved).expect("should serialize to JSON");

        // Verify key fields are present in the JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();

        // Find the api_key setting
        let api_key = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
            .expect("should have ai.anthropic.api_key in JSON");
        assert_eq!(api_key["setting_type"], "apikey", "setting_type must be 'apikey' in JSON");
        assert_eq!(api_key["effective_value"], "sk-test");
        assert_eq!(api_key["enabled"], true);

        // Find a bool setting
        let allow = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.allow")
            .expect("should have ai.anthropic.allow in JSON");
        assert_eq!(allow["setting_type"], "bool");
        assert_eq!(allow["effective_value"], true);

        // Verify all settings have a setting_type field
        for item in arr {
            assert!(
                item.get("setting_type").is_some(),
                "setting {} missing setting_type in JSON",
                item["id"],
            );
        }
    }

    #[test]
    fn load_settings_file_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let file = load_settings_file(&path).unwrap();
        assert!(file.settings.is_empty());
    }

    #[test]
    fn load_settings_file_garbage_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.toml");
        std::fs::write(&path, "not = [valid { toml }").unwrap();
        assert!(load_settings_file(&path).is_err());
    }

    #[test]
    fn load_settings_file_wrong_schema_returns_error() {
        // Valid TOML but wrong structure (settings is a string, not a table)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrong_schema.toml");
        std::fs::write(&path, "settings = \"not a table\"").unwrap();
        assert!(load_settings_file(&path).is_err());
    }

    // -----------------------------------------------------------------------
    // VM settings
    // -----------------------------------------------------------------------

    #[test]
    fn vm_settings_default_cpu_count() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.cpu_count, Some(4));
    }

    #[test]
    fn vm_settings_default_scratch_size() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(16));
    }

    #[test]
    fn vm_settings_default_ram() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.ram_gb, Some(4));
    }

    #[test]
    fn vm_settings_from_user() {
        let user = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(32))]);
        let resolved = resolve_settings(&user, &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(32));
    }

    #[test]
    fn vm_settings_ram_from_user() {
        let user = file_with(vec![("vm.ram_gb", SettingValue::Number(8))]);
        let resolved = resolve_settings(&user, &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.ram_gb, Some(8));
    }

    #[test]
    fn vm_settings_corp_overrides_user() {
        let user = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(32))]);
        let corp = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(4))]);
        let resolved = resolve_settings(&user, &corp);
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(4));
    }

    #[test]
    fn vm_settings_ram_corp_overrides_user() {
        let user = file_with(vec![("vm.ram_gb", SettingValue::Number(8))]);
        let corp = file_with(vec![("vm.ram_gb", SettingValue::Number(2))]);
        let resolved = resolve_settings(&user, &corp);
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.ram_gb, Some(2));
    }

    #[test]
    fn vm_settings_cpu_from_user() {
        let user = file_with(vec![("vm.cpu_count", SettingValue::Number(2))]);
        let resolved = resolve_settings(&user, &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.cpu_count, Some(2));
    }

    #[test]
    fn vm_settings_cpu_corp_overrides_user() {
        let user = file_with(vec![("vm.cpu_count", SettingValue::Number(8))]);
        let corp = file_with(vec![("vm.cpu_count", SettingValue::Number(2))]);
        let resolved = resolve_settings(&user, &corp);
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.cpu_count, Some(2));
    }

    // -----------------------------------------------------------------------
    // J: Domain settings (4)
    // -----------------------------------------------------------------------

    #[test]
    fn domains_setting_drives_allow_list() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("*.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_drives_block_list() {
        // ai.anthropic.allow defaults to false, so domains go to block list
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn domains_setting_parsed_correctly() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("api.anthropic.com , console.anthropic.com , *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("console.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("new.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_empty_skipped() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Empty domains text means nothing added to allow list
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "empty domains should not allow anything");
    }

    // -----------------------------------------------------------------------
    // K: Corp block enforcement (3)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_blocked_domains_always_in_block_list() {
        // Corp locks ai.anthropic.allow = false
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User tries to empty the domains
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains (*.anthropic.com) should still be blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domains must stay blocked");
    }

    #[test]
    fn corp_blocked_domain_not_allowed_via_other_service() {
        // Corp blocks anthropic
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User adds api.anthropic.com to google domains and enables google
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.domains", SettingValue::Text("*.googleapis.com,api.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // api.anthropic.com should be blocked even though it's in google domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domain must not be allowed via other service");
        // google domains should still work
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn user_disabled_service_domains_in_block_list() {
        // User (not corp) disables a service
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);
    }

    // -----------------------------------------------------------------------
    // K2: Stress tests -- block > allow > default invariants
    // -----------------------------------------------------------------------

    #[test]
    fn stress_disabled_provider_always_blocked_regardless_of_default() {
        // Provider off + default_action=allow => domains must still be blocked.
        let user = file_with(vec![
            ("network.default_action", SettingValue::Text("allow".into())),
            // anthropic defaults to off
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "disabled provider must be blocked even with default=allow");
    }

    #[test]
    fn stress_enabled_provider_always_allowed_regardless_of_default() {
        // Provider on + default_action=deny => domains must still be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow, "enabled provider must be allowed even with default=deny");
    }

    #[test]
    fn stress_corp_block_beats_user_allow() {
        // Corp blocks anthropic, user enables it -- block must win.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow");
    }

    #[test]
    fn stress_corp_block_beats_user_allow_with_default_allow() {
        // Corp blocks, user enables, default=allow -- still blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow + default allow");
    }

    #[test]
    fn stress_corp_block_via_other_provider_wildcard() {
        // Corp blocks *.anthropic.com via anthropic toggle.
        // User adds *.anthropic.com to openai domains and enables openai.
        // Corp-blocked wildcard must still deny.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.openai.domains", SettingValue::Text("*.openai.com, *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // anthropic subdomain must be blocked despite being in openai domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked wildcard must not be allowed via other provider");
        // openai subdomain should be allowed (not corp-blocked)
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_emptying_domains() {
        // Corp blocks anthropic. User empties the domains field to try
        // removing the domains from the block list.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains should still be blocked (union of default + effective)
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must survive user emptying domains");
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_changing_domains() {
        // Corp blocks anthropic. User changes domains to something else.
        // Both old defaults AND new effective domains must be blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("custom.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default wildcard still blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "default domains must remain blocked");
        // User's custom domain also blocked (corp said no anthropic)
        let (action, _) = dp.evaluate("custom.anthropic.com");
        assert_eq!(action, Action::Deny, "user-added domains must also be blocked when corp says no");
    }

    #[test]
    fn stress_user_disable_blocks_even_with_default_allow() {
        // User disables a provider. Even with default_action=allow,
        // that provider's domains must be explicitly blocked.
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny, "user-disabled provider must be blocked even with default=allow");
    }

    #[test]
    fn stress_registry_disable_blocks_all_domains() {
        // Disabling a registry blocks ALL its domains, not just some.
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("api.github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("raw.githubusercontent.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn stress_all_providers_disabled_all_blocked() {
        // Disable every provider and registry. All their domains must be blocked.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
            ("registry.github.allow", SettingValue::Bool(false)),
            ("registry.pypi.allow", SettingValue::Bool(false)),
            ("registry.npm.allow", SettingValue::Bool(false)),
            ("registry.crates.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Every known domain should be denied
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org", "registry.npmjs.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when all services disabled");
        }
    }

    #[test]
    fn stress_all_providers_enabled_all_allowed() {
        // Enable every provider. All their domains must be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("registry.github.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Allow, "{domain} must be allowed when all services enabled");
        }
    }

    #[test]
    fn stress_unknown_domain_follows_default_deny() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // default_action defaults to "deny"
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Deny, "unknown domain must follow default deny");
    }

    #[test]
    fn stress_unknown_domain_follows_default_allow() {
        let user = file_with(vec![
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Allow, "unknown domain must follow default allow");
    }

    #[test]
    fn stress_corp_block_all_providers_user_enables_all() {
        // Corp blocks every AI provider. User enables them all.
        // Corp must win for all.
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when corp blocks all providers");
        }
    }

    #[test]
    fn stress_mixed_corp_and_user_decisions() {
        // Corp blocks anthropic only. User enables openai, disables google.
        // anthropic: corp-blocked (deny)
        // openai: user-enabled (allow)
        // google: user-disabled (deny)
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked anthropic must be denied");

        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow, "user-enabled openai must be allowed");

        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Deny, "user-disabled google must be denied");
    }

    // -----------------------------------------------------------------------
    // L: API key injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_injected_when_toggle_on() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_injected_even_when_toggle_off() {
        // API keys are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![
            // ai.anthropic.allow defaults to false
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_not_injected_when_empty() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn google_api_key_sets_gemini_env_var() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.api_key", SettingValue::Text("AIza-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-test");
        // Only GEMINI_API_KEY is set (not GOOGLE_API_KEY) to avoid
        // gemini CLI warning: "Both GOOGLE_API_KEY and GEMINI_API_KEY are set"
        assert!(env.get("GOOGLE_API_KEY").is_none());
    }

    #[test]
    fn openai_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            // ai.openai.allow defaults to false
            ("ai.openai.api_key", SettingValue::Text("sk-oai-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai-test");
    }

    #[test]
    fn google_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-off".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-off");
    }

    #[test]
    fn all_three_providers_injected() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
        // 3 API keys + 7 built-in env vars (TERM, HOME, PATH, LANG, 3x CA)
        // + 3 CAPSEM_*_ALLOWED provider flags
        assert_eq!(env.len(), 13);
    }

    #[test]
    fn all_three_providers_injected_all_toggles_off() {
        // All toggles off but keys set -- all should still be injected.
        let user = file_with(vec![
            // anthropic defaults to off
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            // google: explicitly disable
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn mixed_toggles_all_keys_injected() {
        // One provider on, two off -- all keys should be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn provider_allowed_env_vars_injected() {
        // CAPSEM_*_ALLOWED env vars reflect the provider allow toggles.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "1");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn provider_allowed_defaults_to_zero() {
        // Default allow values: anthropic=false, openai=false, google=true.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn empty_keys_skipped_regardless_of_toggle() {
        // Toggle on but key empty -- should NOT be injected.
        // Toggle off and key empty -- should NOT be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
            ("ai.openai.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // Only dynamic env vars from defaults might exist, but no API keys.
        let has_ant = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        let has_oai = gc.env.as_ref().is_some_and(|e| e.contains_key("OPENAI_API_KEY"));
        assert!(!has_ant, "empty anthropic key should not be injected");
        assert!(!has_oai, "empty openai key should not be injected");
    }

    // -----------------------------------------------------------------------
    // M: Gemini CLI boot files
    // -----------------------------------------------------------------------

    #[test]
    fn gemini_boot_files_injected_when_google_enabled() {
        // Google AI is enabled by default, so gemini files should be injected
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_injected_even_when_google_disabled() {
        // Boot files are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![("ai.google.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_settings_json_user_override() {
        let custom = r#"{"homeDirectoryWarningDismissed":true,"mcpServers":{"myserver":{}}}"#;
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"));
    }

    #[test]
    fn gemini_boot_files_have_correct_paths() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_user_override_with_toggle_off() {
        // Custom file content should be injected even when google is disabled.
        let custom = r#"{"mcpServers":{"custom":{}}}"#;
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"), "custom content should be present");
    }

    #[test]
    fn gemini_boot_files_empty_value_skipped() {
        // If a file setting is explicitly set to empty content, it should not be injected.
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File { path: "/root/.gemini/settings.json".into(), content: "".into() }),
            ("ai.google.gemini.projects_json", SettingValue::File { path: "/root/.gemini/projects.json".into(), content: "".into() }),
            ("ai.google.gemini.trusted_folders_json", SettingValue::File { path: "/root/.gemini/trustedFolders.json".into(), content: "".into() }),
            ("ai.google.gemini.installation_id", SettingValue::File { path: "/root/.gemini/installation_id".into(), content: "".into() }),
            ("ai.anthropic.claude.settings_json", SettingValue::File { path: "/root/.claude/settings.json".into(), content: "".into() }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let file_paths: Vec<&str> = gc.files.as_ref().map_or(vec![], |f| f.iter().map(|x| x.path.as_str()).collect());
        assert!(!file_paths.contains(&"/root/.gemini/settings.json"));
        assert!(!file_paths.contains(&"/root/.claude/settings.json"));
    }

    #[test]
    fn gemini_boot_files_have_correct_mode() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        for f in &files {
            assert_eq!(f.mode, 0o600, "boot file {} should have mode 0600 (owner-only)", f.path);
        }
    }

    #[test]
    fn api_keys_and_boot_files_both_injected_toggle_off() {
        // End-to-end: toggle off, but key + files should all be present.
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-key".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // API key should be injected
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-key");
        // Boot files (from defaults) should also be injected
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    // -----------------------------------------------------------------------
    // N: File setting type
    // -----------------------------------------------------------------------

    #[test]
    fn file_type_exists_in_setting_type_enum() {
        // The File variant should serialize to "file".
        let st = SettingType::File;
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(json, r#""file""#);
    }

    #[test]
    fn gemini_json_settings_use_file_type() {
        // All .json Gemini settings should be SettingType::File, not Text.
        let defs = setting_definitions();
        for id in &[
            "ai.google.gemini.settings_json",
            "ai.google.gemini.projects_json",
            "ai.google.gemini.trusted_folders_json",
        ] {
            let def = defs.iter().find(|d| d.id == *id).unwrap();
            assert_eq!(
                def.setting_type,
                SettingType::File,
                "{id} should be File type"
            );
        }
    }

    #[test]
    fn gemini_installation_id_is_file_type() {
        // installation_id is now a File type (path + content).
        let defs = setting_definitions();
        let def = defs.iter().find(|d| d.id == "ai.google.gemini.installation_id").unwrap();
        assert_eq!(def.setting_type, SettingType::File);
        let (path, content) = def.default_value.as_file().expect("should be File value");
        assert_eq!(path, "/root/.gemini/installation_id");
        assert!(content.starts_with("capsem-sandbox-"));
    }

    #[test]
    fn file_settings_have_path_in_default_value() {
        // Every File-type setting must have a File default with a valid path.
        let defs = setting_definitions();
        for def in &defs {
            if def.setting_type == SettingType::File {
                let (path, _) = def.default_value.as_file().unwrap_or_else(|| {
                    panic!("File setting {} must have File default value", def.id)
                });
                assert!(path.starts_with('/'), "path must be absolute: {path} (setting {})", def.id);
            }
        }
    }

    #[test]
    fn guest_config_collects_file_type_settings() {
        // settings_to_guest_config should pick up File values directly.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        // All file settings come from SettingValue::File
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    // -----------------------------------------------------------------------
    // O: Setting value validation
    // -----------------------------------------------------------------------

    #[test]
    fn validate_file_setting_rejects_invalid_json() {
        let err = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: "{not valid json".into(),
            },
        );
        assert!(err.is_err(), "invalid JSON should be rejected");
        assert!(err.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn validate_file_setting_accepts_valid_json() {
        let result = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: r#"{"key":"value"}"#.into(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_file_setting_accepts_empty_content() {
        // Empty content is fine -- means "use default" or "don't inject".
        let result = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: "".into(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_json_file_accepts_anything() {
        // installation_id path doesn't end in .json -- no JSON validation.
        let result = validate_setting_value(
            "ai.google.gemini.installation_id",
            &SettingValue::File {
                path: "/root/.gemini/installation_id".into(),
                content: "not json at all".into(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_file_settings_pass_through() {
        // Bool, Number, etc. settings always pass validation.
        let result = validate_setting_value(
            "ai.anthropic.allow",
            &SettingValue::Bool(true),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn file_type_resolved_setting_has_file_value() {
        // The resolved setting for a File type should have a File value with path.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.google.gemini.settings_json").unwrap();
        assert_eq!(s.setting_type, SettingType::File);
        let (path, _content) = s.effective_value.as_file().expect("should be a File value");
        assert_eq!(path, "/root/.gemini/settings.json");
    }

    // -----------------------------------------------------------------------
    // P: Metadata-driven env var injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_settings_have_env_vars_metadata() {
        // API key settings must declare their env var name in metadata.env_vars
        // instead of relying on a hardcoded API_KEY_MAP.
        let defs = setting_definitions();
        let cases = [
            ("ai.anthropic.api_key", "ANTHROPIC_API_KEY"),
            ("ai.openai.api_key", "OPENAI_API_KEY"),
            ("ai.google.api_key", "GEMINI_API_KEY"),
        ];
        for (id, expected_var) in &cases {
            let def = defs.iter().find(|d| d.id == *id)
                .unwrap_or_else(|| panic!("missing setting {id}"));
            assert!(
                def.metadata.env_vars.contains(&expected_var.to_string()),
                "{id} should have env_vars containing {expected_var}, got {:?}",
                def.metadata.env_vars,
            );
        }
    }

    #[test]
    fn builtin_env_settings_exist() {
        // Built-in guest env vars (TERM, HOME, PATH, LANG) must be registered
        // settings, not hardcoded in build_boot_config.
        let defs = setting_definitions();
        let required = ["TERM", "HOME", "PATH", "LANG"];
        for var in &required {
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
            assert!(found, "no setting definition injects env var {var}");
        }
    }

    #[test]
    fn ca_bundle_setting_injects_three_env_vars() {
        // A single CA bundle setting should inject REQUESTS_CA_BUNDLE,
        // NODE_EXTRA_CA_CERTS, and SSL_CERT_FILE.
        let defs = setting_definitions();
        let ca_vars = ["REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE"];
        for var in &ca_vars {
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
            assert!(found, "no setting definition injects env var {var}");
        }
    }

    #[test]
    fn guest_config_env_from_metadata_env_vars() {
        // settings_to_guest_config should inject env vars based on
        // metadata.env_vars, not hardcoded API_KEY_MAP.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("sk-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test");
    }

    #[test]
    fn builtin_env_defaults_in_guest_config() {
        // With no user/corp overrides, the built-in env vars should have
        // their default values from the setting definitions.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
        assert_eq!(env.get("HOME").unwrap(), "/root");
        assert!(env.get("PATH").unwrap().contains("/usr/bin"));
        assert_eq!(env.get("LANG").unwrap(), "C");
    }

    #[test]
    fn ca_bundle_injected_as_three_env_vars() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        let ca_path = "/etc/ssl/certs/ca-certificates.crt";
        assert_eq!(env.get("REQUESTS_CA_BUNDLE").unwrap(), ca_path);
        assert_eq!(env.get("NODE_EXTRA_CA_CERTS").unwrap(), ca_path);
        assert_eq!(env.get("SSL_CERT_FILE").unwrap(), ca_path);
    }

    #[test]
    fn corp_can_override_builtin_env() {
        // Corp should be able to lock down built-in env settings.
        let defs = setting_definitions();
        let term_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"TERM".to_string())).unwrap();
        let corp = file_with(vec![(&term_def.id, SettingValue::Text("dumb".into()))]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "dumb");
    }

    #[test]
    fn user_can_override_builtin_env() {
        let defs = setting_definitions();
        let path_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"PATH".to_string())).unwrap();
        let user = file_with(vec![(&path_def.id, SettingValue::Text("/custom/bin".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("PATH").unwrap(), "/custom/bin");
    }

    #[test]
    fn empty_env_var_setting_not_injected() {
        // A setting with env_vars metadata but empty value should not be injected.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn dynamic_guest_env_still_works() {
        // Dynamic guest.env.* settings should still be injected alongside
        // metadata-driven env vars.
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        // Built-in env vars should also be present.
        assert!(env.contains_key("TERM"));
    }

    #[test]
    fn each_boot_message_fits_in_frame() {
        // Each individual boot message (SetEnv, FileWrite) must fit in
        // MAX_FRAME_SIZE. The old single-BootConfig frame limit is gone.
        use capsem_proto::{encode_host_msg, MAX_FRAME_SIZE};

        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);

        // Each env var as a SetEnv message
        for (key, value) in gc.env.unwrap_or_default() {
            let msg = capsem_proto::HostToGuest::SetEnv { key: key.clone(), value: value.clone() };
            let frame = encode_host_msg(&msg).unwrap();
            assert!(
                frame.len() - 4 <= MAX_FRAME_SIZE as usize,
                "SetEnv({key}) too large: {} bytes",
                frame.len() - 4,
            );
        }

        // Each file as a FileWrite message
        for f in gc.files.unwrap_or_default() {
            let msg = capsem_proto::HostToGuest::FileWrite {
                path: f.path.clone(),
                data: f.content.into_bytes(),
                mode: f.mode,
            };
            let frame = encode_host_msg(&msg).unwrap();
            assert!(
                frame.len() - 4 <= MAX_FRAME_SIZE as usize,
                "FileWrite({}) too large: {} bytes",
                f.path,
                frame.len() - 4,
            );
        }
    }

    #[test]
    fn all_env_vars_metadata_refers_to_text_settings() {
        // Every setting with env_vars metadata must have a text-like type
        // (Text, ApiKey, Password, Url, Email).
        let defs = setting_definitions();
        for def in &defs {
            if !def.metadata.env_vars.is_empty() {
                assert!(
                    matches!(def.setting_type, SettingType::Text | SettingType::ApiKey | SettingType::Password | SettingType::Url | SettingType::Email),
                    "setting {} has env_vars but type {:?} (should be text-like)",
                    def.id, def.setting_type,
                );
            }
        }
    }

    // -------------------------------------------------------------------
    // Boot handshake validation in settings layer
    // -------------------------------------------------------------------

    #[test]
    fn settings_rejects_blocked_env_var() {
        // guest.env.LD_PRELOAD in user.toml should be silently dropped.
        let user = file_with(vec![
            ("guest.env.LD_PRELOAD", SettingValue::Text("/evil/lib.so".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_PRELOAD"));
        assert!(!has_key, "LD_PRELOAD should be dropped by validation");
    }

    #[test]
    fn settings_rejects_ld_library_path() {
        let user = file_with(vec![
            ("guest.env.LD_LIBRARY_PATH", SettingValue::Text("/evil".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_LIBRARY_PATH"));
        assert!(!has_key, "LD_LIBRARY_PATH should be dropped by validation");
    }

    #[test]
    fn settings_accepts_normal_dynamic_env() {
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
    }

    // -----------------------------------------------------------------------
    // Search category
    // -----------------------------------------------------------------------

    #[test]
    fn search_google_allowed_by_default() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "search.google.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(true));
        assert_eq!(s.category, "Google Search");
    }

    #[test]
    fn search_perplexity_firecrawl_blocked_by_default() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &["search.perplexity.allow", "search.firecrawl.allow"] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(false), "expected {id} to be false");
        }
    }

    #[test]
    fn search_google_domains_in_policy() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("www.google.com");
        assert_eq!(action, Action::Allow, "google.com should be allowed by default");
    }

    // -----------------------------------------------------------------------
    // Custom allow/block
    // -----------------------------------------------------------------------

    #[test]
    fn custom_allow_allows_domains() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // elie.net is in the default custom_allow
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Allow, "elie.net should be allowed via custom_allow");
    }

    #[test]
    fn custom_allow_wildcard_allows_subdomains() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("www.elie.net");
        assert_eq!(action, Action::Allow, "*.elie.net should allow subdomains");
    }

    #[test]
    fn custom_block_blocks_domains() {
        let user = file_with(vec![
            ("network.custom_block", SettingValue::Text("evil.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("evil.com");
        assert_eq!(action, Action::Deny, "custom_block should block domains");
    }

    #[test]
    fn custom_block_beats_custom_allow_on_overlap() {
        let user = file_with(vec![
            ("network.custom_allow", SettingValue::Text("overlap.com".into())),
            ("network.custom_block", SettingValue::Text("overlap.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("overlap.com");
        assert_eq!(action, Action::Deny, "block must beat allow for overlapping domains");
    }

    #[test]
    fn custom_allow_empty_entries_tolerated() {
        let user = file_with(vec![
            ("network.custom_allow", SettingValue::Text(",, , foo.com , ,".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("foo.com");
        assert_eq!(action, Action::Allow, "empty entries should be ignored");
    }

    #[test]
    fn custom_block_empty_is_noop() {
        let user = file_with(vec![
            ("network.custom_block", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Default custom_allow domains (elie.net) still allowed
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Allow, "empty custom_block should not block anything");
    }

    #[test]
    fn custom_allow_corp_override() {
        // Corp sets custom_allow to empty -> user's default elie.net is gone
        let corp = file_with(vec![
            ("network.custom_allow", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Deny, "corp should be able to override custom_allow");
    }

    #[test]
    fn custom_allow_in_network_policy() {
        // Verify custom domains also appear in the NetworkPolicy path
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let allowed = dp.allowed_patterns();
        assert!(
            allowed.iter().any(|d| d == "elie.net"),
            "elie.net should be in allowed patterns: {allowed:?}"
        );
    }

    // -----------------------------------------------------------------------
    // MCP server injection into settings.json
    // -----------------------------------------------------------------------

    #[test]
    fn inject_capsem_mcp_server_into_empty_json() {
        let result = inject_capsem_mcp_server(r#"{}"#);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_preserves_existing_servers() {
        let input = r#"{"mcpServers":{"github":{"command":"npx","args":["-y","@github/mcp"]}}}"#;
        let result = inject_capsem_mcp_server(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["mcpServers"]["github"]["command"], "npx");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_preserves_other_keys() {
        let input = r#"{"permissions":{"defaultMode":"bypassPermissions"}}"#;
        let result = inject_capsem_mcp_server(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["permissions"]["defaultMode"], "bypassPermissions");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_invalid_json_passthrough() {
        let input = "not json at all";
        let result = inject_capsem_mcp_server(input);
        assert_eq!(result, input);
    }

    #[test]
    fn claude_default_settings_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let claude = files.iter().find(|f| f.path == "/root/.claude/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&claude.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server",
            "capsem MCP server should be injected into Claude settings.json"
        );
        // Original permissions should still be there
        assert_eq!(parsed["permissions"]["defaultMode"], "bypassPermissions");
    }

    #[test]
    fn gemini_default_settings_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&gemini.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server",
            "capsem MCP server should be injected into Gemini settings.json"
        );
    }

    #[test]
    fn user_mcp_servers_preserved_alongside_capsem() {
        let custom = r#"{"mcpServers":{"myserver":{"command":"my-tool"}}}"#;
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&gemini.content).unwrap();
        assert_eq!(parsed["mcpServers"]["myserver"]["command"], "my-tool");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn capsem_mcp_not_in_non_settings_json_files() {
        // Other boot files (projects.json, etc.) should NOT get mcpServers injected
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let projects = files.iter().find(|f| f.path == "/root/.gemini/projects.json").unwrap();
        assert!(!projects.content.contains("capsem"), "projects.json should not have capsem injected");
    }

    // -----------------------------------------------------------------------
    // TOML registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn toml_registry_parses() {
        // The embedded defaults.toml must parse without panicking.
        let defs = setting_definitions();
        assert!(!defs.is_empty(), "defaults.toml must produce at least one setting");
    }

    #[test]
    fn toml_registry_setting_count() {
        // Guard against accidental deletions. Update this if settings are
        // intentionally added or removed.
        let defs = setting_definitions();
        assert!(
            defs.len() >= 20,
            "expected at least 20 settings from defaults.toml, got {}",
            defs.len(),
        );
    }

    #[test]
    fn toml_registry_ids_from_path() {
        // IDs are dot-separated paths derived from the TOML table nesting.
        let defs = setting_definitions();
        for def in &defs {
            assert!(
                def.id.contains('.'),
                "setting id '{}' should be a dotted path",
                def.id,
            );
        }
    }

    #[test]
    fn toml_registry_category_inherited() {
        // Category is inherited from the nearest ancestor group with a `name`.
        let defs = setting_definitions();
        let anthropic_allow = defs.iter().find(|d| d.id == "ai.anthropic.allow").unwrap();
        assert!(
            !anthropic_allow.category.is_empty(),
            "ai.anthropic.allow should have a category inherited from its group",
        );
    }

    #[test]
    fn toml_registry_enabled_by_inherited() {
        // enabled_by is inherited from the group and applied to children
        // but NOT to the toggle setting itself.
        let defs = setting_definitions();
        let allow = defs.iter().find(|d| d.id == "ai.anthropic.allow").unwrap();
        assert!(
            allow.enabled_by.is_none(),
            "the toggle itself should not have enabled_by",
        );
        let api_key = defs.iter().find(|d| d.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(
            api_key.enabled_by.as_deref(),
            Some("ai.anthropic.allow"),
            "api_key should inherit enabled_by from its group",
        );
    }

    #[test]
    fn toml_registry_meta_fields() {
        // Metadata fields (domains, choices, rules, env_vars)
        // are correctly parsed from the `meta` sub-table.
        let defs = setting_definitions();

        // Registry toggles should have domains in metadata
        let github = defs.iter().find(|d| d.id == "registry.github.allow").unwrap();
        assert!(!github.metadata.domains.is_empty(), "github toggle should have domain metadata");

        // network.default_action should have choices
        let da = defs.iter().find(|d| d.id == "network.default_action").unwrap();
        assert!(!da.metadata.choices.is_empty(), "default_action should have choices");

        // API key settings should have env_vars
        let key = defs.iter().find(|d| d.id == "ai.anthropic.api_key").unwrap();
        assert!(
            !key.metadata.env_vars.is_empty(),
            "api_key settings should have env_vars metadata",
        );
    }

    // -----------------------------------------------------------------------
    // Config lint tests
    // -----------------------------------------------------------------------

    fn make_resolved(id: &str, stype: SettingType, value: SettingValue, meta: SettingMetadata, enabled_by: Option<&str>) -> ResolvedSetting {
        ResolvedSetting {
            id: id.to_string(),
            category: "Test".to_string(),
            name: id.to_string(),
            description: "test".to_string(),
            setting_type: stype,
            default_value: value.clone(),
            effective_value: value,
            source: PolicySource::Default,
            modified: None,
            corp_locked: false,
            enabled_by: enabled_by.map(String::from),
            enabled: true,
            metadata: meta,
            collapsed: false,
        }
    }

    // -- JSON validation (File values) --

    fn file_val(path: &str, content: &str) -> SettingValue {
        SettingValue::File { path: path.into(), content: content.into() }
    }

    #[test]
    fn config_lint_valid_json_passes() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"key":"val"}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_malformed_json_gives_clear_error() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", "{bad json}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("invalid JSON")));
    }

    #[test]
    fn config_lint_json_not_object_warns() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", "42"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("not an object")));
    }

    #[test]
    fn config_lint_empty_json_file_ok() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", ""), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_with_trailing_comma_gives_error() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"a":1,}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error"));
    }

    #[test]
    fn config_lint_json_with_unicode_passes() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"name":"cafe\u0301"}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_deeply_nested_passes() {
        let json = r#"{"a":{"b":{"c":{"d":{"e":"deep"}}}}}"#;
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", json), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_huge_payload_passes() {
        let big_val = "x".repeat(1_000_000);
        let json = format!(r#"{{"data":"{}"}}"#, big_val);
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", &json), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_file_path_must_be_absolute() {
        let s = make_resolved("test.file", SettingType::File, file_val("relative/path.json", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("absolute")));
    }

    #[test]
    fn config_lint_file_path_no_traversal() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/../etc/passwd", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("..")));
    }

    #[test]
    fn config_lint_file_unusual_path_warns() {
        let s = make_resolved("test.file", SettingType::File, file_val("/tmp/test.json", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("unusual")));
    }

    // -- Number validation --

    #[test]
    fn config_lint_number_in_range_ok() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(4), meta, None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_number_below_min_error() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(0), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("below minimum"));
    }

    #[test]
    fn config_lint_number_above_max_error() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.disk", SettingType::Number, SettingValue::Number(256), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("exceeds maximum"));
    }

    #[test]
    fn config_lint_number_at_boundary_ok() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s1 = make_resolved("vm.min", SettingType::Number, SettingValue::Number(1), meta.clone(), None);
        let s2 = make_resolved("vm.max", SettingType::Number, SettingValue::Number(128), meta, None);
        let issues = config_lint(&[s1, s2]);
        assert!(issues.is_empty());
    }

    // -- Choice validation --

    #[test]
    fn config_lint_valid_choice_ok() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("deny".into()), meta, None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_invalid_choice_error() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("block".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("not a valid choice"));
    }

    #[test]
    fn config_lint_empty_choice_when_choices_defined_error() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
    }

    #[test]
    fn config_lint_case_sensitive_choice() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("Allow".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1, "'Allow' != 'allow' -- case sensitive");
    }

    // -- API key validation --

    #[test]
    fn config_lint_apikey_with_whitespace_warns() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant key".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("whitespace")));
    }

    #[test]
    fn config_lint_apikey_with_newline_warns() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant\n".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("whitespace")));
    }

    #[test]
    fn config_lint_apikey_empty_when_enabled_warns() {
        let toggle = make_resolved("ai.provider.allow", SettingType::Bool, SettingValue::Bool(true), SettingMetadata::default(), None);
        let key = make_resolved("ai.provider.key", SettingType::ApiKey, SettingValue::Text("".into()), SettingMetadata::default(), Some("ai.provider.allow"));
        let issues = config_lint(&[toggle, key]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("empty")));
    }

    #[test]
    fn config_lint_apikey_empty_when_disabled_ok() {
        let toggle = make_resolved("ai.provider.allow", SettingType::Bool, SettingValue::Bool(false), SettingMetadata::default(), None);
        let key = make_resolved("ai.provider.key", SettingType::ApiKey, SettingValue::Text("".into()), SettingMetadata::default(), Some("ai.provider.allow"));
        let issues = config_lint(&[toggle, key]);
        assert!(issues.is_empty(), "disabled provider with empty key is fine");
    }

    #[test]
    fn config_lint_apikey_normal_value_ok() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant-api03-valid".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    // -- Text validation --

    #[test]
    fn config_lint_text_with_nul_byte_error() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("hello\0world".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("invalid characters"));
    }

    #[test]
    fn config_lint_text_normal_ok() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("hello".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_text_unicode_ok() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("cafe\u{0301}".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_text_very_long_ok() {
        let long_val = "x".repeat(10_000);
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text(long_val), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    // -- Serialization roundtrip --

    #[test]
    fn config_lint_all_issues_serialize_deserialize() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        let json = serde_json::to_string(&issues).unwrap();
        let roundtrip: Vec<ConfigIssue> = serde_json::from_str(&json).unwrap();
        assert_eq!(issues, roundtrip);
    }

    #[test]
    fn config_lint_issue_messages_are_nonempty() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        for issue in &issues {
            assert!(!issue.message.is_empty());
            assert!(!issue.id.is_empty());
        }
    }

    #[test]
    fn config_lint_issue_ids_are_valid_setting_ids() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("vm.cpu_count", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        for issue in &issues {
            assert_eq!(issue.id, "vm.cpu_count");
        }
    }

    // -- Integration --

    #[test]
    fn config_lint_default_config_has_no_errors() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let issues = config_lint(&resolved);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == "error").collect();
        assert!(errors.is_empty(), "default config should have no errors: {errors:?}");
    }

    #[test]
    fn config_lint_returns_multiple_issues() {
        let meta_num = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s1 = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta_num, None);
        let s2 = make_resolved("v.f", SettingType::File, file_val("/root/test.json", "{bad}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s1, s2]);
        assert!(issues.len() >= 2, "expected multiple issues: {issues:?}");
    }

    // -----------------------------------------------------------------------
    // Settings tree tests
    // -----------------------------------------------------------------------

    #[test]
    fn settings_tree_has_top_level_groups() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let tree = build_settings_tree(&resolved);
        assert!(!tree.is_empty(), "tree should have top-level nodes");
        // All top-level nodes should be groups
        for node in &tree {
            match node {
                SettingsNode::Group { name, .. } => {
                    assert!(!name.is_empty());
                }
                SettingsNode::Leaf(_) => {
                    panic!("top-level nodes should be groups, not leaves");
                }
            }
        }
    }

    #[test]
    fn settings_tree_contains_all_definitions() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let tree = build_settings_tree(&resolved);
        let defs = setting_definitions();

        fn collect_leaf_ids(nodes: &[SettingsNode]) -> Vec<String> {
            let mut ids = Vec::new();
            for node in nodes {
                match node {
                    SettingsNode::Leaf(s) => ids.push(s.id.clone()),
                    SettingsNode::Group { children, .. } => {
                        ids.extend(collect_leaf_ids(children));
                    }
                }
            }
            ids
        }

        let leaf_ids = collect_leaf_ids(&tree);
        for def in &defs {
            assert!(
                leaf_ids.contains(&def.id),
                "tree missing definition: {}",
                def.id,
            );
        }
    }

    #[test]
    fn settings_tree_groups_have_expected_names() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let tree = build_settings_tree(&resolved);

        fn collect_group_names(nodes: &[SettingsNode]) -> Vec<String> {
            let mut names = Vec::new();
            for node in nodes {
                if let SettingsNode::Group { name, children, .. } = node {
                    names.push(name.clone());
                    names.extend(collect_group_names(children));
                }
            }
            names
        }

        let names = collect_group_names(&tree);
        for expected in &["AI Providers", "Package Registries", "Guest Environment", "Network", "VM", "Appearance"] {
            assert!(
                names.contains(&expected.to_string()),
                "tree missing group: {expected}",
            );
        }
    }

    #[test]
    fn settings_tree_serializes_to_json() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let tree = build_settings_tree(&resolved);
        let json = serde_json::to_string(&tree).unwrap();
        // Verify it round-trips
        let _: Vec<SettingsNode> = serde_json::from_str(&json).unwrap();
        assert!(json.contains("\"kind\":\"group\""));
        assert!(json.contains("\"kind\":\"leaf\""));
    }

    #[test]
    fn settings_tree_dynamic_env_appended_to_guest() {
        let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let tree = build_settings_tree(&resolved);

        fn find_leaf_in_group(nodes: &[SettingsNode], group_name: &str, leaf_id: &str) -> bool {
            for node in nodes {
                if let SettingsNode::Group { name, children, .. } = node {
                    if name == group_name {
                        return children.iter().any(|c| match c {
                            SettingsNode::Leaf(s) => s.id == leaf_id,
                            SettingsNode::Group { children, .. } => {
                                children.iter().any(|cc| match cc {
                                    SettingsNode::Leaf(s) => s.id == leaf_id,
                                    _ => false,
                                })
                            }
                        });
                    }
                    if find_leaf_in_group(children, group_name, leaf_id) {
                        return true;
                    }
                }
            }
            false
        }

        assert!(
            find_leaf_in_group(&tree, "Guest Environment", "guest.env.EDITOR"),
            "dynamic guest.env.EDITOR should appear in Guest Environment group",
        );
    }

    #[test]
    fn settings_tree_enabled_by_on_groups() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let tree = build_settings_tree(&resolved);

        fn find_group(nodes: &[SettingsNode], key: &str) -> Option<SettingsNode> {
            for node in nodes {
                if let SettingsNode::Group { key: k, children, .. } = node {
                    if k == key {
                        return Some(node.clone());
                    }
                    if let Some(found) = find_group(children, key) {
                        return Some(found);
                    }
                }
            }
            None
        }

        // ai.anthropic group should have enabled_by = "ai.anthropic.allow"
        let anthropic = find_group(&tree, "ai.anthropic");
        assert!(anthropic.is_some(), "should find ai.anthropic group");
        if let Some(SettingsNode::Group { enabled_by, .. }) = anthropic {
            assert_eq!(enabled_by, Some("ai.anthropic.allow".to_string()));
        }
    }
}
