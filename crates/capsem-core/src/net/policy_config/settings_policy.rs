//! Edits to the user's policy in settings.toml, and the readbacks they need.
//!
//! Each edit opens the file as written, changes one thing, validates the whole
//! next file -- referenced rule files merged, the settings contract held, the
//! rules compiled -- and only then writes it whole. It answers with what
//! changed, for the policy-mutation ledger.
//!
//! The corp config wins over every value here. An edit that corp already
//! decides is refused rather than written to no effect.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::loader::{load_settings_document, write_local_settings_document};
use super::provider_profile::ProviderRuleProfile;
use super::security_rule_profile::{
    SecurityPluginConfig, SecurityRule, SecurityRuleAction, SecurityRuleManagedOperation, SecurityRuleManagedTarget,
    SecurityRulePriority, SecurityRulePriorityName, SecurityRuleProfile, SecurityRuleSource,
};
use super::types::SettingsFile;
use crate::mcp::policy::McpConfig;
use capsem_config::validate_policy_target;

const SETTINGS_FILENAME: &str = "settings.toml";
const BUILTIN_LOCAL_SERVER: &str = "local";

/// One applied edit of settings.toml, as the policy-mutation ledger records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyMutationSummary {
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

impl PolicyMutationSummary {
    pub fn into_logger_event(
        self,
        timestamp_unix_ms: i64,
        mutation_id: impl Into<String>,
        status: capsem_logger::PolicyMutationStatus,
        error: Option<String>,
        trace_id: Option<String>,
    ) -> capsem_logger::PolicyMutationEvent {
        capsem_logger::PolicyMutationEvent {
            timestamp_unix_ms,
            mutation_id: mutation_id.into(),
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

/// The effective MCP permission for a tool or the default, and who set it:
/// `corp`, `settings`, or the built-in `default`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolPermissionStatus {
    pub action: SecurityRuleAction,
    pub source: String,
    pub rule_id: Option<String>,
}

/// The user's settings.toml, opened for one edit.
pub struct SettingsPolicyEdit {
    path: PathBuf,
    document: SettingsFile,
    old_hash: String,
    old_size: u64,
}

struct EditTarget<'a> {
    category: &'a str,
    target_kind: &'a str,
    target_key: String,
    operation: &'a str,
    rule_id: Option<String>,
}

impl SettingsPolicyEdit {
    pub fn open(path: &Path) -> Result<Self, String> {
        let (old_hash, old_size) = content_identity(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            document: load_settings_document(path)?.unwrap_or_default(),
            old_hash,
            old_size,
        })
    }

    /// The plugin configuration settings.toml sets, if it sets one.
    pub fn plugin_config(&self, plugin_id: &str) -> Option<SecurityPluginConfig> {
        self.document.plugins.get(plugin_id).copied()
    }

    /// Set one plugin's mode and detection level.
    pub fn set_plugin_config(
        mut self,
        corp: &SettingsFile,
        plugin_id: &str,
        config: SecurityPluginConfig,
        actor: &str,
    ) -> Result<PolicyMutationSummary, String> {
        validate_policy_target("plugin id", plugin_id)?;
        if corp.plugins.contains_key(plugin_id) {
            return Err(format!("plugin {plugin_id} is set by the corp config"));
        }
        self.document.plugins.insert(plugin_id.to_string(), config);
        self.commit(
            actor,
            EditTarget {
                category: "plugin",
                target_kind: "plugin",
                target_key: plugin_id.to_string(),
                operation: "edit",
                rule_id: None,
            },
        )
    }

    /// Set the action of `default.mcp`, the rule every MCP call falls to when
    /// no tool rule matches.
    pub fn set_mcp_default_permission(
        mut self,
        corp: &SettingsFile,
        action: SecurityRuleAction,
        actor: &str,
    ) -> Result<PolicyMutationSummary, String> {
        let action = mcp_permission_action(action)?;
        if corp.default.contains_key("mcp") {
            return Err("default.mcp is set by the corp config".to_string());
        }
        let mut rule = match self.document.default.get("mcp") {
            Some(rule) => rule.clone(),
            None => builtin_default_mcp_rule()?,
        };
        rule.action = action;
        self.document.default.insert("mcp".to_string(), rule);
        self.commit(
            actor,
            EditTarget {
                category: "mcp",
                target_kind: "mcp_default",
                target_key: "default.mcp".to_string(),
                operation: "permission",
                rule_id: Some("default.mcp".to_string()),
            },
        )
    }

    /// Allow, ask or block one tool of a configured MCP server, as a managed
    /// `profiles.rules.*` rule.
    pub fn set_mcp_tool_permission(
        mut self,
        corp: &SettingsFile,
        server: &str,
        tool: &str,
        action: SecurityRuleAction,
        actor: &str,
    ) -> Result<PolicyMutationSummary, String> {
        let action = mcp_permission_action(action)?;
        validate_policy_target("mcp server", server)?;
        validate_policy_target("mcp tool", tool)?;
        let configured = McpConfig::merged(self.document.mcp.as_ref(), corp.mcp.as_ref());
        ensure_mcp_server_configured(configured.as_ref(), server)?;

        let managed = mcp_tool_target(server, tool);
        let rule_key = match managed_rule_keys(&self.document.profiles.rules, &managed)?.first() {
            Some(key) => key.clone(),
            None => managed_mcp_rule_key(server, tool),
        };
        self.document.profiles.rules.insert(
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
                priority: Some(SecurityRulePriority::Named(SecurityRulePriorityName::Default)),
                corp_locked: false,
                reason: Some(format!("Settings-managed MCP tool permission for {server}/{tool}.")),
                managed: Some(managed.clone()),
                plugin_config: BTreeMap::new(),
            },
        );
        self.commit(
            actor,
            EditTarget {
                category: managed.category(),
                target_kind: managed.target_kind(),
                target_key: managed.target_key(),
                operation: SecurityRuleManagedOperation::Permission.as_str(),
                rule_id: Some(format!("profiles.rules.{rule_key}")),
            },
        )
    }

    fn commit(self, actor: &str, target: EditTarget<'_>) -> Result<PolicyMutationSummary, String> {
        SecurityRuleProfile {
            default: self.document.default.clone(),
            profiles: self.document.profiles.clone(),
            ..SecurityRuleProfile::default()
        }
        .compile(SecurityRuleSource::User)
        .map_err(|error| format!("compile settings rules after edit: {error}"))?;
        write_local_settings_document(&self.path, &self.document)?;
        let (new_hash, new_size) = content_identity(&self.path)?;
        Ok(PolicyMutationSummary {
            actor: actor.to_string(),
            category: target.category.to_string(),
            filename: SETTINGS_FILENAME.to_string(),
            affected_path: SETTINGS_FILENAME.to_string(),
            target_kind: target.target_kind.to_string(),
            target_key: target.target_key,
            operation: target.operation.to_string(),
            rule_id: target.rule_id,
            old_hash: self.old_hash,
            old_size: self.old_size,
            new_hash,
            new_size,
        })
    }
}

/// The effective `default.mcp` action: corp's, else the user's, else built in.
pub fn mcp_default_permission(settings: &SettingsFile, corp: &SettingsFile) -> Result<McpToolPermissionStatus, String> {
    let (rule, source) = if let Some(rule) = corp.default.get("mcp") {
        (rule.clone(), "corp")
    } else if let Some(rule) = settings.default.get("mcp") {
        (rule.clone(), "settings")
    } else {
        (builtin_default_mcp_rule()?, "default")
    };
    Ok(McpToolPermissionStatus {
        action: mcp_permission_action(rule.action)?,
        source: source.to_string(),
        rule_id: Some("default.mcp".to_string()),
    })
}

/// The effective permission of one tool: its managed rule, corp's first, else
/// the effective `default.mcp`.
pub fn mcp_tool_permission(
    settings: &SettingsFile,
    corp: &SettingsFile,
    server: &str,
    tool: &str,
) -> Result<McpToolPermissionStatus, String> {
    validate_policy_target("mcp server", server)?;
    validate_policy_target("mcp tool", tool)?;
    let managed = mcp_tool_target(server, tool);
    for (rules, source) in [(&corp.profiles.rules, "corp"), (&settings.profiles.rules, "settings")] {
        if let Some(key) = managed_rule_keys(rules, &managed)?.first() {
            return Ok(McpToolPermissionStatus {
                action: mcp_permission_action(rules[key].action)?,
                source: source.to_string(),
                rule_id: Some(format!("profiles.rules.{key}")),
            });
        }
    }
    mcp_default_permission(settings, corp)
}

/// Whether `server` is an MCP server this configuration runs: the built-in
/// `local` server unless turned off, or a declared server.
pub fn mcp_server_configured(config: Option<&McpConfig>, server: &str) -> bool {
    if server == BUILTIN_LOCAL_SERVER {
        return config
            .and_then(|config| config.server_enabled.get(BUILTIN_LOCAL_SERVER))
            .copied()
            .unwrap_or(true);
    }
    config.is_some_and(|config| config.servers.iter().any(|entry| entry.name == server))
}

fn ensure_mcp_server_configured(config: Option<&McpConfig>, server: &str) -> Result<(), String> {
    if mcp_server_configured(config, server) {
        Ok(())
    } else {
        Err(format!("MCP server {server} is not configured"))
    }
}

fn mcp_tool_target(server: &str, tool: &str) -> SecurityRuleManagedTarget {
    SecurityRuleManagedTarget::McpTool {
        server: server.to_string(),
        tool: tool.to_string(),
        operation: SecurityRuleManagedOperation::Permission,
    }
}

fn managed_rule_keys(
    rules: &BTreeMap<String, SecurityRule>,
    managed: &SecurityRuleManagedTarget,
) -> Result<Vec<String>, String> {
    let keys: Vec<String> = rules
        .iter()
        .filter(|(_, rule)| rule.managed.as_ref() == Some(managed))
        .map(|(key, _)| key.clone())
        .collect();
    if keys.len() > 1 {
        return Err(format!("duplicate managed target {}", managed.identity_key()));
    }
    Ok(keys)
}

fn builtin_default_mcp_rule() -> Result<SecurityRule, String> {
    ProviderRuleProfile::builtin_security_defaults()
        .default
        .get("mcp")
        .cloned()
        .ok_or_else(|| "built-in default.mcp rule is missing".to_string())
}

fn mcp_permission_action(action: SecurityRuleAction) -> Result<SecurityRuleAction, String> {
    match action {
        SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block => Ok(action),
        other => Err(format!(
            "MCP permission action must be allow, ask, or block, got {}",
            other.as_str()
        )),
    }
}

/// `blake3:<hex>` and length of a file's bytes; an absent file is empty.
fn content_identity(path: &Path) -> Result<(String, u64), String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    Ok((format!("blake3:{}", blake3::hash(&bytes).to_hex()), bytes.len() as u64))
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
