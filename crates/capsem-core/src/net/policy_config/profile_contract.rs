use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::provider_profile::{AiProviderProfile, ProviderRuleProfile};
use super::security_rule_profile::{SecurityPluginConfig, SecurityRuleGroup, SecurityRuleProfile};
use super::types::{RuleFileReferences, ToolConfigSourceRecord};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfigFile {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_svg: Option<String>,
    #[serde(default)]
    pub availability: ProfileAvailability,
    #[serde(default)]
    pub assets: ProfileAssetConfig,
    #[serde(default)]
    pub vm: ProfileVmDefaults,
    #[serde(default, skip_serializing_if = "RuleFileReferences::is_empty")]
    pub rule_files: RuleFileReferences,
    #[serde(
        default,
        skip_serializing_if = "super::security_rule_profile::SecurityRuleGroup::is_empty"
    )]
    pub profiles: SecurityRuleGroup,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, AiProviderProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::policy::McpUserConfig>,
    #[serde(default)]
    pub skills: ProfileSkills,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_config_sources: BTreeMap<String, ToolConfigSourceRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileAvailability {
    #[serde(default = "default_true")]
    pub web: bool,
    #[serde(default = "default_true")]
    pub shell: bool,
    #[serde(default)]
    pub mobile: bool,
}

impl Default for ProfileAvailability {
    fn default() -> Self {
        Self {
            web: true,
            shell: true,
            mobile: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileAssetConfig {
    #[serde(default = "default_asset_channel")]
    pub channel: String,
    #[serde(default = "default_kernel_asset")]
    pub kernel: String,
    #[serde(default = "default_initrd_asset")]
    pub initrd: String,
    #[serde(default = "default_rootfs_asset")]
    pub rootfs: String,
}

impl Default for ProfileAssetConfig {
    fn default() -> Self {
        Self {
            channel: default_asset_channel(),
            kernel: default_kernel_asset(),
            initrd: default_initrd_asset(),
            rootfs: default_rootfs_asset(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileVmDefaults {
    #[serde(default = "default_cpu_count")]
    pub cpu_count: u32,
    #[serde(default = "default_ram_gb")]
    pub ram_gb: u32,
    #[serde(default = "default_scratch_disk_size_gb")]
    pub scratch_disk_size_gb: u32,
}

impl Default for ProfileVmDefaults {
    fn default() -> Self {
        Self {
            cpu_count: default_cpu_count(),
            ram_gb: default_ram_gb(),
            scratch_disk_size_gb: default_scratch_disk_size_gb(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSkills {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

impl ProfileConfigFile {
    pub fn builtin_default() -> Self {
        let defaults = ProviderRuleProfile::builtin_security_defaults();
        Self {
            id: "default".to_string(),
            name: "Default".to_string(),
            description: "Built-in Capsem developer profile.".to_string(),
            icon_svg: None,
            availability: ProfileAvailability::default(),
            assets: ProfileAssetConfig::default(),
            vm: ProfileVmDefaults::default(),
            rule_files: RuleFileReferences::default(),
            profiles: defaults.profiles,
            ai: defaults.ai,
            plugins: defaults.plugins,
            mcp: None,
            skills: ProfileSkills::default(),
            tool_config_sources: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_profile_id(&self.id)?;
        validate_non_empty("profile.name", &self.name)?;
        validate_non_empty("profile.description", &self.description)?;
        if let Some(icon_svg) = self.icon_svg.as_deref() {
            let trimmed = icon_svg.trim_start();
            if !trimmed.starts_with("<svg") {
                return Err("profile.icon_svg must start with <svg".to_string());
            }
        }
        self.assets.validate()?;
        self.vm.validate()?;
        self.skills.validate()?;
        let rule_profile = SecurityRuleProfile {
            profiles: self.profiles.clone(),
            ai: self.ai.clone(),
            plugins: self.plugins.clone(),
            ..SecurityRuleProfile::default()
        };
        rule_profile.validate()?;
        for (record_id, record) in &self.tool_config_sources {
            record.validate(record_id)?;
        }
        Ok(())
    }
}

impl ProfileAssetConfig {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty("profile.assets.channel", &self.channel)?;
        validate_non_empty("profile.assets.kernel", &self.kernel)?;
        validate_non_empty("profile.assets.initrd", &self.initrd)?;
        validate_non_empty("profile.assets.rootfs", &self.rootfs)
    }
}

impl ProfileVmDefaults {
    fn validate(&self) -> Result<(), String> {
        if self.cpu_count == 0 {
            return Err("profile.vm.cpu_count must be greater than 0".to_string());
        }
        if self.ram_gb == 0 {
            return Err("profile.vm.ram_gb must be greater than 0".to_string());
        }
        if self.scratch_disk_size_gb == 0 {
            return Err("profile.vm.scratch_disk_size_gb must be greater than 0".to_string());
        }
        Ok(())
    }
}

impl ProfileSkills {
    fn validate(&self) -> Result<(), String> {
        for path in &self.paths {
            validate_non_empty("profile.skills.paths", path)?;
        }
        Ok(())
    }
}

pub fn validate_profile_id(id: &str) -> Result<(), String> {
    validate_non_empty("profile.id", id)?;
    if id.len() > 64 {
        return Err("profile.id must be at most 64 characters".to_string());
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    {
        return Err("profile.id must use lowercase ascii, digits, '-' or '_'".to_string());
    }
    Ok(())
}

fn validate_non_empty(kind: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{kind} must not be empty"))
    } else {
        Ok(())
    }
}

const fn default_true() -> bool {
    true
}

fn default_asset_channel() -> String {
    "stable".to_string()
}

fn default_kernel_asset() -> String {
    "vmlinuz".to_string()
}

fn default_initrd_asset() -> String {
    "initrd.img".to_string()
}

fn default_rootfs_asset() -> String {
    "rootfs.erofs".to_string()
}

const fn default_cpu_count() -> u32 {
    4
}

const fn default_ram_gb() -> u32 {
    4
}

const fn default_scratch_disk_size_gb() -> u32 {
    16
}

#[cfg(test)]
mod tests;
