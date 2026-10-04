use super::types::SettingsFile;

/// Which file may set a registry setting id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOwner {
    /// UI and application preferences, written by `/settings/edit`.
    Settings,
    /// Behavior settings (VM resources, network mechanics, credentials): only
    /// a corporate config may set them; everyone else runs their defaults.
    Corp,
}

impl ConfigOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Corp => "corp",
        }
    }
}

pub fn setting_id_owner(id: &str) -> ConfigOwner {
    if id.starts_with("app.") || id.starts_with("appearance.") {
        ConfigOwner::Settings
    } else {
        ConfigOwner::Corp
    }
}

/// settings.toml carries the user's preferences and the user's policy:
/// security rules (inline or through `rule_files`), AI providers, plugin
/// modes and MCP servers. Corporate integration and network mechanics stay
/// corp-only.
pub fn validate_settings_toml_contract(file: &SettingsFile) -> Result<(), String> {
    reject_corp_only_sections(file)?;
    reject_settings_keys_not_owned_by(file, ConfigOwner::Settings, "settings.toml")
}

pub fn validate_corp_toml_contract(file: &SettingsFile) -> Result<(), String> {
    reject_settings_keys_not_owned_by(file, ConfigOwner::Corp, "corp.toml")
}

fn reject_corp_only_sections(file: &SettingsFile) -> Result<(), String> {
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

fn reject_settings_keys_not_owned_by(file: &SettingsFile, expected: ConfigOwner, label: &str) -> Result<(), String> {
    for id in file.settings.keys() {
        let owner = setting_id_owner(id);
        if owner != expected {
            return Err(format!(
                "{label} cannot define setting '{id}': owned by {}",
                owner.as_str()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
