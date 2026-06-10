use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::provider_profile::AiProviderProfile;
use super::security_rule_profile::{
    SecurityPluginConfig, SecurityRuleGroup, SecurityRuleProfile, SecurityRuleSet,
    SecurityRuleSource,
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obom: Option<ProfileObomConfig>,
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
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileObomConfig {
    pub format: String,
    pub arch: BTreeMap<String, ProfileObomDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileObomDescriptor {
    pub name: String,
    pub url: String,
    pub hash: String,
    pub size: u64,
    pub generator: String,
    pub generator_version: String,
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
        toml::from_str(include_str!(
            "../../../../../config/profiles/code/profile.toml"
        ))
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
        if let Some(obom) = &self.obom {
            obom.validate()?;
        }
        self.vm.validate()?;
        self.skills.validate()?;
        if let Some(mcp) = &self.mcp {
            mcp.validate("profile")?;
        }
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

    pub fn inline_security_rule_profile(&self) -> SecurityRuleProfile {
        SecurityRuleProfile {
            default: self.default.clone(),
            profiles: self.profiles.clone(),
            ai: self.ai.clone(),
            plugins: self.plugins.clone(),
            ..SecurityRuleProfile::default()
        }
    }

    pub fn security_rule_profile_from_files(
        &self,
        base_dir: &Path,
    ) -> Result<SecurityRuleProfile, String> {
        let mut profile = self.inline_security_rule_profile();
        if let Some(enforcement) = self.rule_files.enforcement.as_deref() {
            let path = resolve_profile_rule_file_path(base_dir, enforcement);
            let content = fs::read_to_string(&path).map_err(|error| {
                format!("read profile enforcement rules {}: {error}", path.display())
            })?;
            let rules = SecurityRuleProfile::parse_toml(&content).map_err(|error| {
                format!(
                    "parse profile enforcement rules {}: {error}",
                    path.display()
                )
            })?;
            merge_profile_rule_file(&mut profile, rules, &path)?;
        }
        if let Some(sigma) = self.rule_files.sigma.as_deref() {
            let path = resolve_profile_rule_file_path(base_dir, sigma);
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("read profile Sigma rules {}: {error}", path.display()))?;
            let rules = SecurityRuleProfile::parse_sigma_yaml(&content).map_err(|error| {
                format!("parse profile Sigma rules {}: {error}", path.display())
            })?;
            merge_profile_rule_file(&mut profile, rules, &path)?;
        }
        profile.validate()?;
        Ok(profile)
    }

    pub fn compile_security_rule_set_from_files(
        &self,
        base_dir: &Path,
        source: SecurityRuleSource,
    ) -> Result<SecurityRuleSet, String> {
        SecurityRuleSet::compile_profile(&self.security_rule_profile_from_files(base_dir)?, source)
    }
}

pub fn resolve_profile_rule_file_path(base_dir: &Path, rule_file: &str) -> PathBuf {
    let path = PathBuf::from(rule_file);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn merge_profile_rule_file(
    target: &mut SecurityRuleProfile,
    source: SecurityRuleProfile,
    path: &Path,
) -> Result<(), String> {
    let path = path.display();
    if !source.corp.is_empty() {
        return Err(format!(
            "profile rule file {path} must not define corp.rules"
        ));
    }
    merge_rule_map(
        "default",
        &mut target.default,
        source.default,
        &path.to_string(),
    )?;
    merge_security_rule_group(
        "profiles",
        &mut target.profiles,
        source.profiles,
        &path.to_string(),
    )?;
    merge_map("ai", &mut target.ai, source.ai, &path.to_string())?;
    merge_map(
        "plugins",
        &mut target.plugins,
        source.plugins,
        &path.to_string(),
    )?;
    Ok(())
}

fn merge_security_rule_group(
    namespace: &str,
    target: &mut SecurityRuleGroup,
    source: SecurityRuleGroup,
    path: &str,
) -> Result<(), String> {
    merge_rule_map(namespace, &mut target.rules, source.rules, path)
}

fn merge_rule_map(
    namespace: &str,
    target: &mut BTreeMap<String, super::security_rule_profile::SecurityRule>,
    source: BTreeMap<String, super::security_rule_profile::SecurityRule>,
    path: &str,
) -> Result<(), String> {
    merge_map(namespace, target, source, path)
}

fn merge_map<T>(
    namespace: &str,
    target: &mut BTreeMap<String, T>,
    source: BTreeMap<String, T>,
    path: &str,
) -> Result<(), String> {
    for (key, value) in source {
        if target.contains_key(&key) {
            return Err(format!(
                "duplicate profile rule file entry {namespace}.{key} from {path}"
            ));
        }
        target.insert(key, value);
    }
    Ok(())
}

impl ProfileAssetConfig {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty("profile.assets.format", &self.format)?;
        if self.format != "profile-assets.v1" {
            return Err("profile.assets.format must be profile-assets.v1".to_string());
        }
        validate_non_empty("profile.assets.refresh_policy", &self.refresh_policy)?;
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

impl ProfileObomConfig {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty("profile.obom.format", &self.format)?;
        if self.format != "cyclonedx-obom.v1" {
            return Err("profile.obom.format must be cyclonedx-obom.v1".to_string());
        }
        if self.arch.is_empty() {
            return Err("profile.obom.arch must define at least one architecture".to_string());
        }
        for (arch, descriptor) in &self.arch {
            validate_arch_key(arch)?;
            descriptor.validate(&format!("profile.obom.arch.{arch}"))?;
        }
        Ok(())
    }

    pub fn current_arch_obom(&self) -> Option<&ProfileObomDescriptor> {
        self.arch.get(current_profile_arch())
    }
}

impl ProfileObomDescriptor {
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
        if self.size == 0 {
            return Err(format!("{field}.size must be greater than 0"));
        }
        validate_non_empty(&format!("{field}.generator"), &self.generator)?;
        validate_non_empty(
            &format!("{field}.generator_version"),
            &self.generator_version,
        )?;
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
        if self.size == 0 {
            return Err(format!("{field}.size must be greater than 0"));
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
            if !file_type.is_dir() {
                continue;
            }
            let profile_dir = entry.path();
            let path = profile_dir.join("profile.toml");
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("read profile {}: {error}", path.display()))?;
            let profile: ProfileConfigFile = toml::from_str(&content)
                .map_err(|error| format!("parse profile {}: {error}", path.display()))?;
            profile
                .validate()
                .map_err(|error| format!("validate profile {}: {error}", path.display()))?;
            let dir_name = profile_dir
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    format!(
                        "profile directory {} has no valid directory name",
                        profile_dir.display()
                    )
                })?;
            if profile.id != dir_name {
                return Err(format!(
                    "profile file {} id mismatch: directory is {dir_name}, profile id is {}",
                    path.display(),
                    profile.id
                ));
            }
            if profiles.insert(profile.id.clone(), profile).is_some() {
                return Err(format!("duplicate profile id {dir_name}"));
            }
        }
        if profiles.is_empty() {
            return Err(format!(
                "profile directory {} contains no profile directories with profile.toml",
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
            return Self::load_from_dir(&installed);
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
