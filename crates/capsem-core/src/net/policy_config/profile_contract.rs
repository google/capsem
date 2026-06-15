use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::provider_profile::{AiProviderProfile, ModelEndpointRegistry, ProviderRuleProfile};
use super::security_rule_profile::{
    SecurityPluginConfig, SecurityRule, SecurityRuleAction, SecurityRuleGroup,
    SecurityRuleManagedOperation, SecurityRuleManagedTarget, SecurityRulePriority,
    SecurityRulePriorityName, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use super::types::{NetworkConfig, RuleFileReferences, SettingsFile};

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
    pub mcp: Option<crate::mcp::policy::McpProfileConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obom: Option<ProfileObomConfig>,
    #[serde(default, skip_serializing_if = "ProfileFileReferences::is_empty")]
    pub files: ProfileFileReferences,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFileReferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detection: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apt_packages: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_requirements: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm_packages: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tips: Option<ProfileFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_manifest: Option<ProfileFileDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFileDescriptor {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    profile_dir: PathBuf,
    config_root: PathBuf,
    config: ProfileConfigFile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveProfileFile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub revision: String,
    #[serde(default)]
    pub profile_rules: SecurityRuleProfile,
    #[serde(default)]
    pub corp_rules: SecurityRuleProfile,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::policy::McpProfileConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileStatus {
    pub profile_id: String,
    pub ready: bool,
    pub files: Vec<ProfileFileStatus>,
    pub assets: Vec<ProfileAssetStatus>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFileStatus {
    pub kind: String,
    pub path: PathBuf,
    pub expected_hash: String,
    pub expected_size: u64,
    pub actual_hash: Option<String>,
    pub actual_size: Option<u64>,
    pub present: bool,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileAssetStatus {
    pub arch: String,
    pub kind: String,
    pub path: PathBuf,
    pub expected_hash: String,
    pub expected_size: u64,
    pub actual_hash: Option<String>,
    pub actual_size: Option<u64>,
    pub present: bool,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMutationSummary {
    pub profile_id: String,
    pub actor: String,
    pub category: String,
    pub filename: String,
    pub affected_path: String,
    pub target_kind: String,
    pub target_key: String,
    pub operation: String,
    pub rule_id: Option<String>,
    pub old_hash: String,
    pub old_size: u64,
    pub new_hash: String,
    pub new_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolPermissionStatus {
    pub action: SecurityRuleAction,
    pub source: String,
    pub rule_id: Option<String>,
}

impl ProfileMutationSummary {
    pub fn into_logger_event(
        self,
        timestamp_unix_ms: i64,
        mutation_id: impl Into<String>,
        status: capsem_logger::ProfileMutationStatus,
        error: Option<String>,
        trace_id: Option<String>,
    ) -> capsem_logger::ProfileMutationEvent {
        capsem_logger::ProfileMutationEvent {
            timestamp_unix_ms,
            mutation_id: mutation_id.into(),
            profile_id: self.profile_id,
            actor: self.actor,
            category: self.category,
            filename: self.filename,
            affected_path: self.affected_path,
            target_kind: self.target_kind,
            target_key: self.target_key,
            operation: self.operation,
            rule_id: self.rule_id,
            old_hash: self.old_hash,
            old_size: self.old_size,
            new_hash: self.new_hash,
            new_size: self.new_size,
            status,
            error,
            trace_id,
        }
    }
}

impl Profile {
    pub fn load_from_dir(profile_dir: impl AsRef<Path>) -> Result<Self, String> {
        let profile_dir = profile_dir.as_ref().to_path_buf();
        let path = profile_dir.join("profile.toml");
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("read profile {}: {error}", path.display()))?;
        let config: ProfileConfigFile = toml::from_str(&content)
            .map_err(|error| format!("parse profile {}: {error}", path.display()))?;
        let config_root = profile_dir
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                format!(
                    "profile directory {} must be under <config>/profiles/<id>",
                    profile_dir.display()
                )
            })?
            .to_path_buf();
        Self::from_config(config_root, profile_dir, config)
    }

    pub fn from_config(
        config_root: PathBuf,
        profile_dir: PathBuf,
        config: ProfileConfigFile,
    ) -> Result<Self, String> {
        config.validate()?;
        let dir_name = profile_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "profile directory {} has no valid directory name",
                    profile_dir.display()
                )
            })?;
        if config.id != dir_name {
            return Err(format!(
                "profile directory id mismatch: directory is {dir_name}, profile id is {}",
                config.id
            ));
        }
        Ok(Self {
            profile_dir,
            config_root,
            config,
        })
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn config(&self) -> &ProfileConfigFile {
        &self.config
    }

    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    pub fn status(&self, assets_dir: &Path, arch: &str) -> ProfileStatus {
        let files = self.file_statuses();
        let assets = self.asset_statuses(assets_dir, arch);
        let mut errors = Vec::new();
        for file in &files {
            if !file.valid {
                errors.push(format!("profile file {} is not valid", file.path.display()));
            }
        }
        for asset in &assets {
            if !asset.valid {
                errors.push(format!(
                    "profile asset {} is not valid",
                    asset.path.display()
                ));
            }
        }
        ProfileStatus {
            profile_id: self.config.id.clone(),
            ready: errors.is_empty(),
            files,
            assets,
            errors,
        }
    }

    pub fn check(&self, assets_dir: &Path, arch: &str) -> Result<ProfileStatus, String> {
        let status = self.status(assets_dir, arch);
        if status.ready {
            Ok(status)
        } else {
            Err(status.errors.join("; "))
        }
    }

    pub fn download_assets(&self, assets_dir: &Path, arch: &str) -> Result<ProfileStatus, String> {
        let arch_assets =
            self.config.assets.arch.get(arch).ok_or_else(|| {
                format!("profile {} has no assets for arch {arch}", self.config.id)
            })?;
        fs::create_dir_all(assets_dir.join(arch))
            .map_err(|error| format!("create asset dir {}: {error}", assets_dir.display()))?;
        for (kind, descriptor) in arch_assets.iter() {
            let Some(source_path) = descriptor.url.strip_prefix("file://") else {
                return Err(format!(
                    "profile {} asset {arch}/{kind} must use file:// for local profile download",
                    self.config.id
                ));
            };
            let source_path = PathBuf::from(source_path);
            let destination = profile_asset_path(assets_dir, arch, descriptor)?;
            fs::copy(&source_path, &destination).map_err(|error| {
                format!(
                    "copy profile asset {} to {}: {error}",
                    source_path.display(),
                    destination.display()
                )
            })?;
            verify_hash_and_size(
                &destination,
                descriptor.resolved_hash(&format!("profile.assets.arch.{arch}.{kind}"))?,
                descriptor.resolved_size(&format!("profile.assets.arch.{arch}.{kind}"))?,
            )
            .map_err(|error| {
                format!(
                    "verify downloaded profile asset {}: {error}",
                    destination.display()
                )
            })?;
        }
        self.check(assets_dir, arch)
    }

    pub fn set_mcp_tool_permission(
        &mut self,
        server: &str,
        tool: &str,
        action: SecurityRuleAction,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        if !matches!(
            action,
            SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block
        ) {
            return Err("MCP tool permission action must be allow, ask, or block".to_string());
        }
        validate_profile_target("mcp server", server)?;
        validate_profile_target("mcp tool", tool)?;
        self.ensure_mcp_server_known(server)?;

        let enforcement_descriptor = self.config.files.enforcement.clone().ok_or_else(|| {
            "profile.files.enforcement is required before mutating enforcement rules".to_string()
        })?;
        let enforcement_rule_file =
            self.config
                .rule_files
                .enforcement
                .as_deref()
                .ok_or_else(|| {
                    "profile.rule_files.enforcement is required before mutating enforcement rules"
                        .to_string()
                })?;
        if enforcement_descriptor.path != enforcement_rule_file {
            return Err(format!(
                "profile.files.enforcement.path must match rule_files.enforcement: {} != {}",
                enforcement_descriptor.path, enforcement_rule_file
            ));
        }

        let enforcement_path = self.config_root.join(&enforcement_descriptor.path);
        let (old_hash, old_size) = verify_hash_and_size(
            &enforcement_path,
            enforcement_descriptor.resolved_hash("profile.files.enforcement")?,
            enforcement_descriptor.resolved_size("profile.files.enforcement")?,
        )?;
        let content = fs::read_to_string(&enforcement_path).map_err(|error| {
            format!(
                "read enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let mut rules = SecurityRuleProfile::parse_toml(&content).map_err(|error| {
            format!(
                "parse enforcement file {} before mutation: {error}",
                enforcement_path.display()
            )
        })?;

        let managed = SecurityRuleManagedTarget::McpTool {
            server: server.to_string(),
            tool: tool.to_string(),
            operation: SecurityRuleManagedOperation::Permission,
        };
        let existing_keys = rules
            .profiles
            .rules
            .iter()
            .filter(|(_, rule)| rule.managed.as_ref() == Some(&managed))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if existing_keys.len() > 1 {
            return Err(format!(
                "enforcement file {} has duplicate managed target {}",
                enforcement_path.display(),
                managed.identity_key()
            ));
        }
        let rule_key = existing_keys
            .first()
            .cloned()
            .unwrap_or_else(|| managed_mcp_rule_key(server, tool));
        rules.profiles.rules.insert(
            rule_key.clone(),
            SecurityRule {
                name: rule_key.clone(),
                action,
                condition: format!(
                    "mcp.server.name == {} && mcp.tool_call.name == {}",
                    cel_string(server),
                    cel_string(tool)
                ),
                enabled: true,
                detection_level: None,
                priority: Some(SecurityRulePriority::Named(
                    SecurityRulePriorityName::Default,
                )),
                corp_locked: false,
                reason: Some(format!(
                    "Profile-managed MCP tool permission for {server}/{tool}."
                )),
                managed: Some(managed.clone()),
                plugin_config: BTreeMap::new(),
            },
        );
        rules.validate()?;

        let serialized = toml::to_string_pretty(&rules)
            .map_err(|error| format!("serialize enforcement file: {error}"))?;
        fs::write(&enforcement_path, serialized).map_err(|error| {
            format!(
                "write enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let (new_hash, new_size) = file_hash_and_size(&enforcement_path)?;
        self.config.files.enforcement = Some(ProfileFileDescriptor {
            path: enforcement_descriptor.path.clone(),
            hash: Some(format!("blake3:{new_hash}")),
            size: Some(new_size),
        });
        self.save()?;

        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: managed.category().to_string(),
            filename: Path::new(&enforcement_descriptor.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("enforcement.toml")
                .to_string(),
            affected_path: enforcement_descriptor.path,
            target_kind: managed.target_kind().to_string(),
            target_key: managed.target_key(),
            operation: SecurityRuleManagedOperation::Permission
                .as_str()
                .to_string(),
            rule_id: Some(format!("profiles.rules.{rule_key}")),
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn mcp_tool_permission(
        &self,
        server: &str,
        tool: &str,
    ) -> Result<McpToolPermissionStatus, String> {
        validate_profile_target("mcp server", server)?;
        validate_profile_target("mcp tool", tool)?;
        self.ensure_mcp_server_known(server)?;

        let (_, _, _, _, rules) = self.load_verified_enforcement_rules()?;
        let managed = SecurityRuleManagedTarget::McpTool {
            server: server.to_string(),
            tool: tool.to_string(),
            operation: SecurityRuleManagedOperation::Permission,
        };
        let matches = rules
            .profiles
            .rules
            .iter()
            .filter(|(_, rule)| rule.managed.as_ref() == Some(&managed))
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(format!(
                "enforcement file has duplicate managed target {}",
                managed.identity_key()
            ));
        }
        if let Some((rule_id, rule)) = matches.first() {
            return mcp_permission_action(rule.action).map(|action| McpToolPermissionStatus {
                action,
                source: "profile_managed".to_string(),
                rule_id: Some(format!("profiles.rules.{rule_id}")),
            });
        }

        let default = rules.default.get("mcp").ok_or_else(|| {
            "default.mcp rule is required for MCP permission readback".to_string()
        })?;
        mcp_permission_action(default.action).map(|action| McpToolPermissionStatus {
            action,
            source: "default".to_string(),
            rule_id: Some("default.mcp".to_string()),
        })
    }

    pub fn mcp_default_permission(&self) -> Result<McpToolPermissionStatus, String> {
        let (_, _, _, _, rules) = self.load_verified_enforcement_rules()?;
        let default = rules.default.get("mcp").ok_or_else(|| {
            "default.mcp rule is required for MCP permission readback".to_string()
        })?;
        mcp_permission_action(default.action).map(|action| McpToolPermissionStatus {
            action,
            source: "default".to_string(),
            rule_id: Some("default.mcp".to_string()),
        })
    }

    pub fn set_mcp_default_permission(
        &mut self,
        action: SecurityRuleAction,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        let action = mcp_permission_action(action)?;
        let (enforcement_descriptor, enforcement_path, old_hash, old_size, mut rules) =
            self.load_verified_enforcement_rules()?;
        let default = rules.default.get_mut("mcp").ok_or_else(|| {
            "default.mcp rule is required before mutating MCP default permission".to_string()
        })?;
        default.action = action;
        rules.validate()?;

        let serialized = toml::to_string_pretty(&rules)
            .map_err(|error| format!("serialize enforcement file: {error}"))?;
        fs::write(&enforcement_path, serialized).map_err(|error| {
            format!(
                "write enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let (new_hash, new_size) =
            self.update_enforcement_pin(&enforcement_descriptor.path, &enforcement_path)?;
        self.save()?;

        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: "mcp".to_string(),
            filename: Path::new(&enforcement_descriptor.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("enforcement.toml")
                .to_string(),
            affected_path: enforcement_descriptor.path,
            target_kind: "mcp_default".to_string(),
            target_key: "default.mcp".to_string(),
            operation: "permission".to_string(),
            rule_id: Some("default.mcp".to_string()),
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn upsert_profile_rule(
        &mut self,
        rule_id: &str,
        rule: SecurityRule,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("profile rule id", rule_id)?;
        if rule.corp_locked {
            return Err(
                "profile rule mutations cannot write corp_locked rules; corp rules must come from corp config"
                    .to_string(),
            );
        }
        let (enforcement_descriptor, enforcement_path, old_hash, old_size, mut rules) =
            self.load_verified_enforcement_rules()?;
        rules.profiles.rules.insert(rule_id.to_string(), rule);
        rules.compile(SecurityRuleSource::User).map_err(|error| {
            format!("compile profile enforcement rules after mutation: {error}")
        })?;
        let serialized = toml::to_string_pretty(&rules)
            .map_err(|error| format!("serialize enforcement file: {error}"))?;
        fs::write(&enforcement_path, serialized).map_err(|error| {
            format!(
                "write enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let (new_hash, new_size) =
            self.update_enforcement_pin(&enforcement_descriptor.path, &enforcement_path)?;
        self.save()?;
        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: "enforcement".to_string(),
            filename: Path::new(&enforcement_descriptor.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("enforcement.toml")
                .to_string(),
            affected_path: enforcement_descriptor.path,
            target_kind: "security_rule".to_string(),
            target_key: rule_id.to_string(),
            operation: "upsert".to_string(),
            rule_id: Some(format!("profiles.rules.{rule_id}")),
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn delete_profile_rule(
        &mut self,
        rule_id: &str,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("profile rule id", rule_id)?;
        let (enforcement_descriptor, enforcement_path, old_hash, old_size, mut rules) =
            self.load_verified_enforcement_rules()?;
        if rules.profiles.rules.remove(rule_id).is_none() {
            return Err(format!("profile enforcement rule not found: {rule_id}"));
        }
        rules
            .compile(SecurityRuleSource::User)
            .map_err(|error| format!("compile profile enforcement rules after delete: {error}"))?;
        let serialized = toml::to_string_pretty(&rules)
            .map_err(|error| format!("serialize enforcement file: {error}"))?;
        fs::write(&enforcement_path, serialized).map_err(|error| {
            format!(
                "write enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let (new_hash, new_size) =
            self.update_enforcement_pin(&enforcement_descriptor.path, &enforcement_path)?;
        self.save()?;
        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: "enforcement".to_string(),
            filename: Path::new(&enforcement_descriptor.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("enforcement.toml")
                .to_string(),
            affected_path: enforcement_descriptor.path,
            target_kind: "security_rule".to_string(),
            target_key: rule_id.to_string(),
            operation: "delete".to_string(),
            rule_id: Some(format!("profiles.rules.{rule_id}")),
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn upsert_mcp_server(
        &mut self,
        server: crate::mcp::policy::McpManualServer,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("MCP server", &server.name)?;
        validate_non_empty("MCP server URL", &server.url)?;
        let profile_path = self.profile_dir.join("profile.toml");
        let (old_hash, old_size) = file_hash_and_size(&profile_path)?;

        let mut mcp = self.config.mcp.clone().unwrap_or_default();
        mcp.servers.retain(|existing| existing.name != server.name);
        mcp.servers.push(server.clone());
        mcp.validate("profile")?;
        self.config.mcp = Some(mcp);
        self.config.validate()?;
        self.save()?;
        let (new_hash, new_size) = file_hash_and_size(&profile_path)?;

        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: "mcp".to_string(),
            filename: "profile.toml".to_string(),
            affected_path: self.profile_toml_relative_path(),
            target_kind: "mcp_server".to_string(),
            target_key: server.name,
            operation: "upsert".to_string(),
            rule_id: None,
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn delete_mcp_server(
        &mut self,
        server: &str,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("MCP server", server)?;
        let profile_path = self.profile_dir.join("profile.toml");
        let (old_hash, old_size) = file_hash_and_size(&profile_path)?;

        let mut mcp = self.config.mcp.clone().unwrap_or_default();
        let before_len = mcp.servers.len();
        mcp.servers.retain(|existing| existing.name != server);
        let removed_server = mcp.servers.len() != before_len;
        let removed_enabled = mcp.server_enabled.remove(server).is_some();
        if !removed_server && !removed_enabled {
            return Err(format!("profile MCP server not found: {server}"));
        }
        mcp.validate("profile")?;
        self.config.mcp = Some(mcp);
        self.config.validate()?;
        self.save()?;
        let (new_hash, new_size) = file_hash_and_size(&profile_path)?;

        Ok(ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: "mcp".to_string(),
            filename: "profile.toml".to_string(),
            affected_path: self.profile_toml_relative_path(),
            target_kind: "mcp_server".to_string(),
            target_key: server.to_string(),
            operation: "delete".to_string(),
            rule_id: None,
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        })
    }

    pub fn add_skill_path(
        &mut self,
        path: &str,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_skill_path(path)?;
        let skill_id = skill_id_for_path(path)?;
        let profile_path = self.profile_dir.join("profile.toml");
        let (old_hash, old_size) = file_hash_and_size(&profile_path)?;
        if self
            .config
            .skills
            .paths
            .iter()
            .any(|existing| existing == path)
        {
            return Err(format!("profile skill already exists: {skill_id}"));
        }
        if self
            .config
            .skills
            .paths
            .iter()
            .any(|existing| skill_id_for_path(existing).as_deref() == Ok(skill_id.as_str()))
        {
            return Err(format!("profile skill id already exists: {skill_id}"));
        }
        self.config.skills.paths.push(path.to_string());
        self.config.validate()?;
        self.save()?;
        let (new_hash, new_size) = file_hash_and_size(&profile_path)?;
        Ok(self.profile_toml_mutation_summary(
            actor, "skills", "skill", &skill_id, "add", old_hash, old_size, new_hash, new_size,
        ))
    }

    pub fn edit_skill_path(
        &mut self,
        skill_id: &str,
        path: &str,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("skill id", skill_id)?;
        validate_profile_skill_path(path)?;
        let new_skill_id = skill_id_for_path(path)?;
        let profile_path = self.profile_dir.join("profile.toml");
        let (old_hash, old_size) = file_hash_and_size(&profile_path)?;
        let index = self
            .config
            .skills
            .paths
            .iter()
            .position(|existing| skill_id_for_path(existing).as_deref() == Ok(skill_id))
            .ok_or_else(|| format!("profile skill not found: {skill_id}"))?;
        if new_skill_id != skill_id
            && self
                .config
                .skills
                .paths
                .iter()
                .any(|existing| skill_id_for_path(existing).as_deref() == Ok(new_skill_id.as_str()))
        {
            return Err(format!("profile skill id already exists: {new_skill_id}"));
        }
        self.config.skills.paths[index] = path.to_string();
        self.config.validate()?;
        self.save()?;
        let (new_hash, new_size) = file_hash_and_size(&profile_path)?;
        Ok(self.profile_toml_mutation_summary(
            actor,
            "skills",
            "skill",
            &new_skill_id,
            "edit",
            old_hash,
            old_size,
            new_hash,
            new_size,
        ))
    }

    pub fn delete_skill(
        &mut self,
        skill_id: &str,
        actor: &str,
    ) -> Result<ProfileMutationSummary, String> {
        validate_profile_target("skill id", skill_id)?;
        let profile_path = self.profile_dir.join("profile.toml");
        let (old_hash, old_size) = file_hash_and_size(&profile_path)?;
        let index = self
            .config
            .skills
            .paths
            .iter()
            .position(|existing| skill_id_for_path(existing).as_deref() == Ok(skill_id))
            .ok_or_else(|| format!("profile skill not found: {skill_id}"))?;
        self.config.skills.paths.remove(index);
        self.config.validate()?;
        self.save()?;
        let (new_hash, new_size) = file_hash_and_size(&profile_path)?;
        Ok(self.profile_toml_mutation_summary(
            actor, "skills", "skill", skill_id, "delete", old_hash, old_size, new_hash, new_size,
        ))
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self.profile_dir.join("profile.toml");
        let content = toml::to_string_pretty(&self.config)
            .map_err(|error| format!("serialize profile: {error}"))?;
        fs::write(&path, content)
            .map_err(|error| format!("write profile {}: {error}", path.display()))
    }

    fn profile_toml_relative_path(&self) -> String {
        format!("profiles/{}/profile.toml", self.config.id)
    }

    #[allow(clippy::too_many_arguments)]
    fn profile_toml_mutation_summary(
        &self,
        actor: &str,
        category: &str,
        target_kind: &str,
        target_key: &str,
        operation: &str,
        old_hash: String,
        old_size: u64,
        new_hash: String,
        new_size: u64,
    ) -> ProfileMutationSummary {
        ProfileMutationSummary {
            profile_id: self.config.id.clone(),
            actor: actor.to_string(),
            category: category.to_string(),
            filename: "profile.toml".to_string(),
            affected_path: self.profile_toml_relative_path(),
            target_kind: target_kind.to_string(),
            target_key: target_key.to_string(),
            operation: operation.to_string(),
            rule_id: None,
            old_hash: format!("blake3:{old_hash}"),
            old_size,
            new_hash: format!("blake3:{new_hash}"),
            new_size,
        }
    }

    fn load_verified_enforcement_rules(
        &self,
    ) -> Result<
        (
            ProfileFileDescriptor,
            PathBuf,
            String,
            u64,
            SecurityRuleProfile,
        ),
        String,
    > {
        let enforcement_descriptor = self.config.files.enforcement.clone().ok_or_else(|| {
            "profile.files.enforcement is required before mutating enforcement rules".to_string()
        })?;
        let enforcement_rule_file =
            self.config
                .rule_files
                .enforcement
                .as_deref()
                .ok_or_else(|| {
                    "profile.rule_files.enforcement is required before mutating enforcement rules"
                        .to_string()
                })?;
        if enforcement_descriptor.path != enforcement_rule_file {
            return Err(format!(
                "profile.files.enforcement.path must match rule_files.enforcement: {} != {}",
                enforcement_descriptor.path, enforcement_rule_file
            ));
        }
        let enforcement_path = self.config_root.join(&enforcement_descriptor.path);
        let (old_hash, old_size) = verify_hash_and_size(
            &enforcement_path,
            enforcement_descriptor.resolved_hash("profile.files.enforcement")?,
            enforcement_descriptor.resolved_size("profile.files.enforcement")?,
        )?;
        let content = fs::read_to_string(&enforcement_path).map_err(|error| {
            format!(
                "read enforcement file {}: {error}",
                enforcement_path.display()
            )
        })?;
        let rules = SecurityRuleProfile::parse_toml(&content).map_err(|error| {
            format!(
                "parse enforcement file {} before mutation: {error}",
                enforcement_path.display()
            )
        })?;
        Ok((
            enforcement_descriptor,
            enforcement_path,
            old_hash,
            old_size,
            rules,
        ))
    }

    fn update_enforcement_pin(
        &mut self,
        descriptor_path: &str,
        enforcement_path: &Path,
    ) -> Result<(String, u64), String> {
        let (new_hash, new_size) = file_hash_and_size(enforcement_path)?;
        self.config.files.enforcement = Some(ProfileFileDescriptor {
            path: descriptor_path.to_string(),
            hash: Some(format!("blake3:{new_hash}")),
            size: Some(new_size),
        });
        Ok((new_hash, new_size))
    }

    fn file_statuses(&self) -> Vec<ProfileFileStatus> {
        self.config
            .files
            .iter()
            .map(|(kind, descriptor)| {
                let path = self.config_root.join(&descriptor.path);
                let expected_hash = descriptor
                    .hash
                    .clone()
                    .unwrap_or_else(|| "unresolved".into());
                let expected_size = descriptor.size.unwrap_or(0);
                match file_hash_and_size(&path) {
                    Ok((hash, size)) => ProfileFileStatus {
                        kind: kind.to_string(),
                        path,
                        expected_hash: expected_hash.clone(),
                        expected_size,
                        actual_hash: Some(format!("blake3:{hash}")),
                        actual_size: Some(size),
                        present: true,
                        valid: descriptor
                            .hash
                            .as_deref()
                            .is_some_and(|expected| expected == format!("blake3:{hash}"))
                            && descriptor.size == Some(size),
                    },
                    Err(_) => ProfileFileStatus {
                        kind: kind.to_string(),
                        path,
                        expected_hash,
                        expected_size,
                        actual_hash: None,
                        actual_size: None,
                        present: false,
                        valid: false,
                    },
                }
            })
            .collect()
    }

    fn asset_statuses(&self, assets_dir: &Path, arch: &str) -> Vec<ProfileAssetStatus> {
        let Some(assets) = self.config.assets.arch.get(arch) else {
            return Vec::new();
        };
        assets
            .iter()
            .map(|(kind, descriptor)| {
                let path = profile_asset_path(assets_dir, arch, descriptor)
                    .unwrap_or_else(|_| assets_dir.join(arch).join(&descriptor.name));
                let expected_hash = descriptor
                    .hash
                    .clone()
                    .unwrap_or_else(|| "unresolved".into());
                let expected_size = descriptor.size.unwrap_or(0);
                match file_hash_and_size(&path) {
                    Ok((hash, size)) => ProfileAssetStatus {
                        arch: arch.to_string(),
                        kind: kind.to_string(),
                        path,
                        expected_hash: expected_hash.clone(),
                        expected_size,
                        actual_hash: Some(format!("blake3:{hash}")),
                        actual_size: Some(size),
                        present: true,
                        valid: descriptor
                            .hash
                            .as_deref()
                            .is_some_and(|expected| expected == format!("blake3:{hash}"))
                            && descriptor.size == Some(size),
                    },
                    Err(_) => ProfileAssetStatus {
                        arch: arch.to_string(),
                        kind: kind.to_string(),
                        path,
                        expected_hash,
                        expected_size,
                        actual_hash: None,
                        actual_size: None,
                        present: false,
                        valid: false,
                    },
                }
            })
            .collect()
    }

    fn ensure_mcp_server_known(&self, server: &str) -> Result<(), String> {
        if server == "local"
            && self
                .config
                .mcp
                .as_ref()
                .and_then(|mcp| mcp.server_enabled.get("local"))
                .copied()
                .unwrap_or(false)
        {
            return Ok(());
        }
        if self
            .config
            .mcp
            .as_ref()
            .is_some_and(|mcp| mcp.servers.iter().any(|entry| entry.name == server))
        {
            return Ok(());
        }
        let descriptor =
            self.config.files.mcp.as_ref().ok_or_else(|| {
                "profile.files.mcp is required to mutate MCP permissions".to_string()
            })?;
        let path = self.config_root.join(&descriptor.path);
        verify_hash_and_size(
            &path,
            descriptor.resolved_hash("profile.files.mcp")?,
            descriptor.resolved_size("profile.files.mcp")?,
        )?;
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("read MCP config {}: {error}", path.display()))?;
        let config: McpJsonConfig = serde_json::from_str(&content)
            .map_err(|error| format!("parse MCP config {}: {error}", path.display()))?;
        if config.mcp_servers.contains_key(server) {
            Ok(())
        } else {
            Err(format!(
                "MCP server {server} is not declared in profile file {}",
                descriptor.path
            ))
        }
    }
}

impl ActiveProfileFile {
    pub fn from_profile_and_corp(
        profile: &Profile,
        corp: &SettingsFile,
        plugin_overrides: BTreeMap<String, SecurityPluginConfig>,
    ) -> Result<Self, String> {
        corp.validate_metadata_contract()?;
        let config = profile.config();
        let mut profile_rules = config.security_rule_profile_from_files(profile.config_root())?;

        let mut plugins = ProviderRuleProfile::builtin_security_defaults().plugins;
        for (plugin_id, plugin) in &profile_rules.plugins {
            plugins.insert(plugin_id.clone(), *plugin);
        }
        for (plugin_id, plugin) in plugin_overrides {
            plugins.insert(plugin_id, plugin);
        }
        for (plugin_id, plugin) in &corp.plugins {
            plugins.insert(plugin_id.clone(), *plugin);
        }
        profile_rules.plugins.clear();

        let corp_rules = SecurityRuleProfile {
            default: corp.default.clone(),
            profiles: corp.profiles.clone(),
            corp: corp.corp.clone(),
            ai: corp.ai.clone(),
            plugins: BTreeMap::new(),
        };
        corp_rules.validate()?;

        let network_profile = SettingsFile {
            default: profile_rules.default.clone(),
            profiles: profile_rules.profiles.clone(),
            ai: profile_rules.ai.clone(),
            ..SettingsFile::default()
        };
        let network_corp = SettingsFile {
            settings: corp.settings.clone(),
            default: corp.default.clone(),
            profiles: corp.profiles.clone(),
            corp: corp.corp.clone(),
            ai: corp.ai.clone(),
            plugins: corp.plugins.clone(),
            network: corp.network.clone(),
            ..SettingsFile::default()
        };
        let merged_network =
            super::builder::MergedPolicies::from_files(&network_profile, &network_corp).network;
        let mut network =
            NetworkConfig::from_policy_and_dns(&merged_network, corp.network.dns.clone());
        network.upstream_overrides = corp.network.upstream_overrides.clone();

        let active = Self {
            id: config.id.clone(),
            name: config.name.clone(),
            description: config.description.clone(),
            revision: config.revision.clone(),
            profile_rules,
            corp_rules,
            plugins,
            network,
            mcp: config.mcp.clone(),
        };
        active.validate()?;
        Ok(active)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_profile_id(&self.id)?;
        validate_non_empty("active_profile.name", &self.name)?;
        validate_non_empty("active_profile.description", &self.description)?;
        validate_non_empty("active_profile.revision", &self.revision)?;
        self.profile_rules.validate()?;
        self.corp_rules.validate()?;
        for plugin_id in self.plugins.keys() {
            validate_profile_target("plugin id", plugin_id)?;
        }
        self.network.validate()?;
        if let Some(mcp) = &self.mcp {
            mcp.validate("active_profile")?;
        }
        Ok(())
    }

    pub fn merged_policy_inputs(&self) -> (SettingsFile, SettingsFile) {
        let profile = SettingsFile {
            default: self.profile_rules.default.clone(),
            profiles: self.profile_rules.profiles.clone(),
            ai: self.profile_rules.ai.clone(),
            ..SettingsFile::default()
        };
        let corp = SettingsFile {
            default: self.corp_rules.default.clone(),
            profiles: self.corp_rules.profiles.clone(),
            corp: self.corp_rules.corp.clone(),
            ai: self.corp_rules.ai.clone(),
            network: self.network.clone(),
            ..SettingsFile::default()
        };
        (profile, corp)
    }

    pub fn compile_security_rule_set(&self) -> Result<SecurityRuleSet, String> {
        self.validate()?;
        let (profile, corp) = self.merged_policy_inputs();
        Ok(super::builder::MergedPolicies::from_files(&profile, &corp).security_rules)
    }

    pub fn model_endpoint_registry(&self) -> Result<ModelEndpointRegistry, String> {
        self.validate()?;
        let provider_profile = ProviderRuleProfile::merge_defaults_user_and_corp(
            &ProviderRuleProfile {
                ai: self.profile_rules.ai.clone(),
            },
            &ProviderRuleProfile {
                ai: self.corp_rules.ai.clone(),
            },
        )?;
        provider_profile.endpoint_registry()
    }
}

fn mcp_permission_action(action: SecurityRuleAction) -> Result<SecurityRuleAction, String> {
    match action {
        SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block => {
            Ok(action)
        }
        other => Err(format!(
            "MCP tool permission action must be allow, ask, or block, got {}",
            other.as_str()
        )),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpJsonConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, serde_json::Value>,
}

impl ProfileConfigFile {
    pub fn builtin_primary() -> Self {
        builtin_profile_configs()
            .into_iter()
            .next()
            .expect("at least one built-in profile must exist")
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
        self.files.validate()?;
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

fn builtin_profile_configs() -> Vec<ProfileConfigFile> {
    [
        include_str!("../../../../../config/profiles/code/profile.toml"),
        include_str!("../../../../../config/profiles/co-work/profile.toml"),
    ]
    .into_iter()
    .map(|content| toml::from_str(content).expect("built-in profile TOML must parse"))
    .collect()
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
    fn iter(&self) -> impl Iterator<Item = (&'static str, &ProfileAssetDescriptor)> {
        [
            ("kernel", &self.kernel),
            ("initrd", &self.initrd),
            ("rootfs", &self.rootfs),
        ]
        .into_iter()
    }

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

impl ProfileFileReferences {
    pub fn is_empty(&self) -> bool {
        self.enforcement.is_none()
            && self.detection.is_none()
            && self.mcp.is_none()
            && self.apt_packages.is_none()
            && self.python_requirements.is_none()
            && self.npm_packages.is_none()
            && self.build.is_none()
            && self.tips.is_none()
            && self.root_manifest.is_none()
    }

    fn validate(&self) -> Result<(), String> {
        for (field, descriptor) in [
            ("profile.files.enforcement", self.enforcement.as_ref()),
            ("profile.files.detection", self.detection.as_ref()),
            ("profile.files.mcp", self.mcp.as_ref()),
            ("profile.files.apt_packages", self.apt_packages.as_ref()),
            (
                "profile.files.python_requirements",
                self.python_requirements.as_ref(),
            ),
            ("profile.files.npm_packages", self.npm_packages.as_ref()),
            ("profile.files.build", self.build.as_ref()),
            ("profile.files.tips", self.tips.as_ref()),
            ("profile.files.root_manifest", self.root_manifest.as_ref()),
        ] {
            if let Some(descriptor) = descriptor {
                descriptor.validate(field)?;
            }
        }
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &ProfileFileDescriptor)> {
        [
            ("enforcement", self.enforcement.as_ref()),
            ("detection", self.detection.as_ref()),
            ("mcp", self.mcp.as_ref()),
            ("apt_packages", self.apt_packages.as_ref()),
            ("python_requirements", self.python_requirements.as_ref()),
            ("npm_packages", self.npm_packages.as_ref()),
            ("build", self.build.as_ref()),
            ("tips", self.tips.as_ref()),
            ("root_manifest", self.root_manifest.as_ref()),
        ]
        .into_iter()
        .filter_map(|(kind, descriptor)| descriptor.map(|descriptor| (kind, descriptor)))
    }
}

impl ProfileFileDescriptor {
    fn validate(&self, field: &str) -> Result<(), String> {
        validate_non_empty(&format!("{field}.path"), &self.path)?;
        validate_relative_profile_path(&format!("{field}.path"), &self.path)?;
        if let Some(hash) = self.hash.as_ref() {
            validate_blake3_hash(&format!("{field}.hash"), hash)?;
        }
        if let Some(size) = self.size {
            if size == 0 {
                return Err(format!("{field}.size must be greater than 0"));
            }
        }
        Ok(())
    }

    pub fn resolved_hash(&self, field: &str) -> Result<&str, String> {
        self.hash
            .as_deref()
            .ok_or_else(|| format!("{field}.hash is unresolved"))
    }

    pub fn resolved_size(&self, field: &str) -> Result<u64, String> {
        self.size
            .ok_or_else(|| format!("{field}.size is unresolved"))
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
        if let Some(hash) = self.hash.as_ref() {
            validate_blake3_hash(&format!("{field}.hash"), hash)?;
        }
        if let Some(size) = self.size {
            if size == 0 {
                return Err(format!("{field}.size must be greater than 0"));
            }
        }
        Ok(())
    }

    pub fn resolved_hash(&self, field: &str) -> Result<&str, String> {
        self.hash
            .as_deref()
            .ok_or_else(|| format!("{field}.hash is unresolved"))
    }

    pub fn resolved_size(&self, field: &str) -> Result<u64, String> {
        self.size
            .ok_or_else(|| format!("{field}.size is unresolved"))
    }
}

fn validate_relative_profile_path(field: &str, value: &str) -> Result<(), String> {
    if value.starts_with('/') || value.starts_with("file://") {
        return Err(format!("{field} must be a config-root-relative path"));
    }
    if value.contains("..") || value.contains('\\') {
        return Err(format!("{field} must not contain path traversal"));
    }
    if value.trim() != value || value.is_empty() {
        return Err(format!("{field} must not be empty or padded"));
    }
    Ok(())
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
        let profiles = builtin_profile_configs()
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect();
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

fn validate_profile_target(kind: &str, value: &str) -> Result<(), String> {
    validate_non_empty(kind, value)?;
    if value.len() > 128 {
        return Err(format!("{kind} must be at most 128 characters"));
    }
    if value.contains("..") || value.contains('\\') || value.trim() != value {
        return Err(format!("{kind} must not contain traversal or padding"));
    }
    Ok(())
}

fn validate_profile_skill_path(value: &str) -> Result<(), String> {
    validate_non_empty("profile skill path", value)?;
    if value.trim() != value || value.contains("..") || value.contains('\\') {
        return Err("profile skill path must not contain traversal or padding".to_string());
    }
    skill_id_for_path(value).map(|_| ())
}

pub fn skill_id_for_path(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    let id = if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        path.parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
    } else {
        path.file_stem().and_then(|name| name.to_str())
    }
    .ok_or_else(|| "profile skill path must identify a skill".to_string())?;
    validate_profile_target("skill id", id)?;
    Ok(id.to_string())
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

fn profile_asset_path(
    assets_dir: &Path,
    arch: &str,
    descriptor: &ProfileAssetDescriptor,
) -> Result<PathBuf, String> {
    let hash = descriptor
        .hash
        .as_deref()
        .ok_or_else(|| format!("profile asset {} hash is unresolved", descriptor.name))?
        .strip_prefix("blake3:")
        .ok_or_else(|| {
            format!(
                "profile asset {} hash must use blake3: prefix",
                descriptor.name
            )
        })?;
    Ok(assets_dir
        .join(arch)
        .join(crate::asset_manager::hash_filename(&descriptor.name, hash)))
}

fn file_hash_and_size(path: &Path) -> Result<(String, u64), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("stat {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let hash = crate::asset_manager::hash_file(path)
        .map_err(|error| format!("hash {}: {error}", path.display()))?;
    Ok((hash, metadata.len()))
}

fn verify_hash_and_size(
    path: &Path,
    expected_hash: &str,
    expected_size: u64,
) -> Result<(String, u64), String> {
    let (hash, size) = file_hash_and_size(path)?;
    let expected_hash = expected_hash
        .strip_prefix("blake3:")
        .ok_or_else(|| "expected hash must use blake3: prefix".to_string())?;
    if hash != expected_hash {
        return Err(format!(
            "{} hash mismatch: expected blake3:{expected_hash}, got blake3:{hash}",
            path.display()
        ));
    }
    if size != expected_size {
        return Err(format!(
            "{} size mismatch: expected {expected_size}, got {size}",
            path.display()
        ));
    }
    Ok((hash, size))
}

fn cel_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

fn managed_mcp_rule_key(server: &str, tool: &str) -> String {
    let mut key = format!(
        "mcp_{}_{}_permission",
        rule_key_fragment(server),
        rule_key_fragment(tool)
    );
    if key.len() > 64 {
        key.truncate(64);
        while key.ends_with('_') || key.ends_with('-') {
            key.pop();
        }
    }
    key
}

fn rule_key_fragment(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_sep = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            output.push('_');
            last_was_sep = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    if output.is_empty() {
        "target".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests;
