use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::provider_profile::AiProviderProfile;
use super::security_rule_profile::{SecurityPluginConfig, SecurityRuleGroup, SecurityRuleProfile};
use super::types::RuleFileReferences;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfigFile {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_svg: Option<String>,
    pub revision: String,
    pub refresh_policy: String,
    #[serde(default)]
    pub availability: ProfileAvailability,
    #[serde(default)]
    pub assets: ProfileAssetConfig,
    #[serde(default)]
    pub vm: ProfileVmDefaults,
    #[serde(default, skip_serializing_if = "RuleFileReferences::is_empty")]
    pub rule_files: RuleFileReferences,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default: BTreeMap<String, super::security_rule_profile::SecurityRule>,
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
    pub format: String,
    pub refresh_policy: String,
    pub filesystem: String,
    pub compression: String,
    pub compression_level: u8,
    pub arch: BTreeMap<String, ProfileArchAssets>,
}

impl Default for ProfileAssetConfig {
    fn default() -> Self {
        ProfileConfigFile::builtin_code().assets
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileArchAssets {
    pub kernel: ProfileAssetDescriptor,
    pub initrd: ProfileAssetDescriptor,
    pub rootfs: ProfileAssetDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileAssetDescriptor {
    pub name: String,
    pub url: String,
    pub hash: String,
    pub signature: String,
    pub size: u64,
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_level: Option<u8>,
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
    pub fn builtin_code() -> Self {
        toml::from_str(include_str!("../../../../../config/profiles/code.toml"))
            .expect("built-in code profile TOML must parse")
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_profile_id(&self.id)?;
        validate_non_empty("profile.name", &self.name)?;
        validate_non_empty("profile.description", &self.description)?;
        validate_non_empty("profile.revision", &self.revision)?;
        validate_non_empty("profile.refresh_policy", &self.refresh_policy)?;
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
            default: self.default.clone(),
            profiles: self.profiles.clone(),
            ai: self.ai.clone(),
            plugins: self.plugins.clone(),
            ..SecurityRuleProfile::default()
        };
        rule_profile.validate()?;
        Ok(())
    }
}

impl ProfileAssetConfig {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty("profile.assets.format", &self.format)?;
        if self.format != "profile-assets.v1" {
            return Err("profile.assets.format must be profile-assets.v1".to_string());
        }
        validate_non_empty("profile.assets.refresh_policy", &self.refresh_policy)?;
        validate_non_empty("profile.assets.filesystem", &self.filesystem)?;
        validate_non_empty("profile.assets.compression", &self.compression)?;
        if self.arch.is_empty() {
            return Err("profile.assets.arch must define at least one architecture".to_string());
        }
        for (arch, assets) in &self.arch {
            validate_arch_key(arch)?;
            assets.validate(arch)?;
        }
        Ok(())
    }

    pub fn current_arch_assets(&self) -> Option<&ProfileArchAssets> {
        self.arch.get(current_profile_arch())
    }
}

impl ProfileArchAssets {
    fn validate(&self, arch: &str) -> Result<(), String> {
        self.kernel
            .validate(&format!("profile.assets.arch.{arch}.kernel"))?;
        self.initrd
            .validate(&format!("profile.assets.arch.{arch}.initrd"))?;
        self.rootfs
            .validate(&format!("profile.assets.arch.{arch}.rootfs"))?;
        Ok(())
    }
}

impl ProfileAssetDescriptor {
    fn validate(&self, field: &str) -> Result<(), String> {
        validate_non_empty(&format!("{field}.name"), &self.name)?;
        validate_non_empty(&format!("{field}.url"), &self.url)?;
        if !(self.url.starts_with("https://") || self.url.starts_with("file://")) {
            return Err(format!("{field}.url must use https:// or file://"));
        }
        if self.url.contains("..") || self.url.contains('\\') {
            return Err(format!("{field}.url must not contain path traversal"));
        }
        validate_blake3_hash(&format!("{field}.hash"), &self.hash)?;
        validate_non_empty(&format!("{field}.signature"), &self.signature)?;
        if self.size == 0 {
            return Err(format!("{field}.size must be greater than 0"));
        }
        validate_non_empty(&format!("{field}.content_type"), &self.content_type)?;
        if let Some(filesystem) = &self.filesystem {
            validate_non_empty(&format!("{field}.filesystem"), filesystem)?;
        }
        if let Some(compression) = &self.compression {
            validate_non_empty(&format!("{field}.compression"), compression)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileCatalog {
    profiles: BTreeMap<String, ProfileConfigFile>,
    source: ProfileCatalogSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileCatalogSource {
    BuiltIn,
    Directory(PathBuf),
}

impl ProfileCatalog {
    pub fn builtin() -> Self {
        let profile = ProfileConfigFile::builtin_code();
        let profiles = BTreeMap::from([(profile.id.clone(), profile)]);
        Self {
            profiles,
            source: ProfileCatalogSource::BuiltIn,
        }
    }

    pub fn load_from_dir(path: &Path) -> Result<Self, String> {
        let entries = fs::read_dir(path)
            .map_err(|error| format!("read profile directory {}: {error}", path.display()))?;
        let mut profiles = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("read profile directory entry: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("read profile file type: {error}"))?;
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("read profile {}: {error}", path.display()))?;
            let profile: ProfileConfigFile = toml::from_str(&content)
                .map_err(|error| format!("parse profile {}: {error}", path.display()))?;
            profile
                .validate()
                .map_err(|error| format!("validate profile {}: {error}", path.display()))?;
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("profile file {} has no valid stem", path.display()))?;
            if profile.id != stem {
                return Err(format!(
                    "profile file {} id mismatch: file stem is {stem}, profile id is {}",
                    path.display(),
                    profile.id
                ));
            }
            if profiles.insert(profile.id.clone(), profile).is_some() {
                return Err(format!("duplicate profile id {stem}"));
            }
        }
        if profiles.is_empty() {
            return Err(format!(
                "profile directory {} contains no profile TOML files",
                path.display()
            ));
        }
        Ok(Self {
            profiles,
            source: ProfileCatalogSource::Directory(path.to_path_buf()),
        })
    }

    pub fn load_default() -> Result<Self, String> {
        if let Ok(path) = std::env::var("CAPSEM_PROFILES_DIR") {
            if !path.is_empty() {
                return Self::load_from_dir(Path::new(&path));
            }
        }
        let installed = crate::paths::capsem_home().join("profiles");
        if installed.is_dir() {
            return match Self::load_from_dir(&installed) {
                Ok(catalog) => Ok(catalog),
                Err(error) if error.contains("contains no profile TOML files") => {
                    Ok(Self::builtin())
                }
                Err(error) => Err(error),
            };
        }
        Ok(Self::builtin())
    }

    pub fn source(&self) -> &ProfileCatalogSource {
        &self.source
    }

    pub fn profiles(&self) -> impl Iterator<Item = &ProfileConfigFile> {
        self.profiles.values()
    }

    pub fn get(&self, profile_id: &str) -> Option<&ProfileConfigFile> {
        self.profiles.get(profile_id)
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

const fn default_cpu_count() -> u32 {
    4
}

const fn default_ram_gb() -> u32 {
    4
}

const fn default_scratch_disk_size_gb() -> u32 {
    16
}

pub fn current_profile_arch() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        std::env::consts::ARCH
    }
}

fn validate_arch_key(arch: &str) -> Result<(), String> {
    validate_non_empty("profile.assets.arch", arch)?;
    if !arch
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err("profile.assets.arch keys must use lowercase ascii, digits, '-' or '_'".into());
    }
    Ok(())
}

fn validate_blake3_hash(field: &str, value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("blake3:") else {
        return Err(format!("{field} must use blake3:<64 lowercase hex>"));
    };
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("{field} must use blake3:<64 lowercase hex>"));
    }
    if hex.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(format!("{field} must use lowercase hex"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
