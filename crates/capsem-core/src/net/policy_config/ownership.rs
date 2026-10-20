//! Which sections of a settings file the user may write.
//!
//! Every registry setting id is the user's: settings.toml may carry any of
//! them, and `/settings/edit` writes them. Corp may set any id too, and an id
//! corp sets is corp-locked: it wins the merge and `/settings/edit` refuses to
//! change it. Ownership is therefore per section, never per setting id.

use super::types::SettingsFile;

/// settings.toml carries the user's settings and the user's policy: every
/// registry setting, security rules (inline or through `rule_files`), AI
/// providers, plugin modes and MCP servers. Corporate integration and network
/// mechanics stay corp-only.
pub fn validate_settings_toml_contract(file: &SettingsFile) -> Result<(), String> {
    if file.refresh_policy.is_some() {
        return Err("settings.toml cannot define corp refresh metadata".to_string());
    }
    if !file.corp.is_empty() {
        return Err("settings.toml cannot define corp.rules".to_string());
    }
    if !file.corp_rule_files.is_empty() {
        return Err("settings.toml cannot define corp rule-file endpoints".to_string());
    }
    if !file.network.is_empty() {
        return Err("settings.toml cannot define network mechanics".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
