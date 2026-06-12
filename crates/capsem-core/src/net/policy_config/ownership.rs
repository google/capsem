use super::types::SettingsFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOwner {
    Settings,
    Profile,
    Corp,
}

impl ConfigOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Profile => "profile",
            Self::Corp => "corp",
        }
    }
}

pub fn setting_id_owner(id: &str) -> ConfigOwner {
    if id.starts_with("app.") || id.starts_with("appearance.") {
        ConfigOwner::Settings
    } else {
        ConfigOwner::Profile
    }
}

pub fn validate_settings_toml_contract(file: &SettingsFile) -> Result<(), String> {
    reject_non_settings_sections(file)?;
    reject_settings_keys_not_owned_by(file, ConfigOwner::Settings, "settings.toml")
}

pub fn validate_profile_toml_contract(file: &SettingsFile) -> Result<(), String> {
    if file.refresh_policy.is_some() {
        return Err("profile.toml cannot define corp refresh metadata".to_string());
    }
    if !file.corp.is_empty() {
        return Err("profile.toml cannot define corp.rules".to_string());
    }
    if !file.corp_rule_files.is_empty() {
        return Err("profile.toml cannot define corp rule-file endpoints".to_string());
    }
    reject_settings_keys_not_owned_by(file, ConfigOwner::Profile, "profile.toml")
}

pub fn validate_corp_toml_contract(file: &SettingsFile) -> Result<(), String> {
    reject_settings_keys_not_owned_by(file, ConfigOwner::Profile, "corp.toml")
}

fn reject_non_settings_sections(file: &SettingsFile) -> Result<(), String> {
    if !file.rule_files.is_empty() {
        return Err("settings.toml cannot define rule_files".to_string());
    }
    if !file.default.is_empty() {
        return Err("settings.toml cannot define default rules".to_string());
    }
    if file.refresh_policy.is_some() {
        return Err("settings.toml cannot define corp refresh metadata".to_string());
    }
    if !file.profiles.is_empty() {
        return Err("settings.toml cannot define profiles.rules".to_string());
    }
    if !file.corp.is_empty() {
        return Err("settings.toml cannot define corp.rules".to_string());
    }
    if !file.corp_rule_files.is_empty() {
        return Err("settings.toml cannot define corp rule-file endpoints".to_string());
    }
    if !file.ai.is_empty() {
        return Err("settings.toml cannot define ai providers".to_string());
    }
    if !file.plugins.is_empty() {
        return Err("settings.toml cannot define plugins".to_string());
    }
    if file.mcp.is_some() {
        return Err("settings.toml cannot define MCP servers".to_string());
    }
    Ok(())
}

fn reject_settings_keys_not_owned_by(
    file: &SettingsFile,
    expected: ConfigOwner,
    label: &str,
) -> Result<(), String> {
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
