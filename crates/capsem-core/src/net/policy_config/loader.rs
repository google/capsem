use std::path::Path;

use super::{SettingValue, SettingsFile};

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
