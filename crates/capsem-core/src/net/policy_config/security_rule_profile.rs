use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::condition::{evaluate_condition_with, validate_condition_with, CompiledCondition};
use super::types::{default_true, PolicySubject};

pub const CORP_PRIORITY_MIN: i32 = -1000;
pub const CORP_PRIORITY_MAX: i32 = -10;
pub const USER_PRIORITY_MIN: i32 = 10;
pub const USER_PRIORITY_MAX: i32 = 1000;
pub const DEFAULT_RULE_PRIORITY: i32 = USER_PRIORITY_MAX + 1;

pub const SECURITY_EVENT_CEL_ROOTS: &[&str] =
    &["http", "dns", "mcp", "model", "file", "process", "security"];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRuleProfile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default: BTreeMap<String, SecurityRule>,
    #[serde(default, skip_serializing_if = "SecurityRuleGroup::is_empty")]
    pub corp: SecurityRuleGroup,
    #[serde(default, skip_serializing_if = "SecurityRuleGroup::is_empty")]
    pub profiles: SecurityRuleGroup,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, SecurityRuleProvider>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRuleGroup {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, SecurityRule>,
}

impl SecurityRuleGroup {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRuleProvider {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listen_ports: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_remote_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<ProviderDiscovery>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, SecurityRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDiscovery {
    pub observed_at: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityRule {
    pub name: String,
    pub action: SecurityRuleAction,
    #[serde(rename = "match")]
    pub condition: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detection_level: Option<DetectionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<SecurityRulePriority>,
    #[serde(default)]
    pub corp_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<SecurityRuleManagedTarget>,
    #[serde(default, flatten)]
    pub plugin_config: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecurityRuleManagedTarget {
    McpServer {
        server: String,
        operation: SecurityRuleManagedOperation,
    },
    McpTool {
        server: String,
        tool: String,
        operation: SecurityRuleManagedOperation,
    },
    Plugin {
        plugin: String,
        operation: SecurityRuleManagedOperation,
    },
    Skill {
        skill: String,
        operation: SecurityRuleManagedOperation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityRuleManagedOperation {
    Permission,
}

impl SecurityRuleManagedTarget {
    pub fn identity_key(&self) -> String {
        match self {
            Self::McpServer { server, operation } => {
                format!("mcp_server:{server}:{}", operation.as_str())
            }
            Self::McpTool {
                server,
                tool,
                operation,
            } => format!("mcp_tool:{server}:{tool}:{}", operation.as_str()),
            Self::Plugin { plugin, operation } => {
                format!("plugin:{plugin}:{}", operation.as_str())
            }
            Self::Skill { skill, operation } => format!("skill:{skill}:{}", operation.as_str()),
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::McpServer { .. } | Self::McpTool { .. } => "mcp",
            Self::Plugin { .. } => "plugin",
            Self::Skill { .. } => "skill",
        }
    }

    pub fn target_kind(&self) -> &'static str {
        match self {
            Self::McpServer { .. } => "mcp_server",
            Self::McpTool { .. } => "mcp_tool",
            Self::Plugin { .. } => "plugin",
            Self::Skill { .. } => "skill",
        }
    }

    pub fn target_key(&self) -> String {
        match self {
            Self::McpServer { server, .. } => server.clone(),
            Self::McpTool { server, tool, .. } => format!("{server}/{tool}"),
            Self::Plugin { plugin, .. } => plugin.clone(),
            Self::Skill { skill, .. } => skill.clone(),
        }
    }

    fn validate(&self, rule_id: &str) -> Result<(), String> {
        match self {
            Self::McpServer { server, .. } => validate_profile_target("mcp server", server),
            Self::McpTool { server, tool, .. } => {
                validate_profile_target("mcp server", server)?;
                validate_profile_target("mcp tool", tool)
            }
            Self::Plugin { plugin, .. } => validate_identifier("plugin id", plugin),
            Self::Skill { skill, .. } => validate_profile_target("skill id", skill),
        }
        .map_err(|error| format!("{rule_id}.managed: {error}"))
    }
}

impl SecurityRuleManagedOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityRuleAction {
    Allow,
    Ask,
    Block,
    Preprocess,
    #[serde(alias = "redact", alias = "mutate", alias = "neutralize")]
    Rewrite,
    Postprocess,
}

impl SecurityRuleAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
            Self::Preprocess => "preprocess",
            Self::Rewrite => "rewrite",
            Self::Postprocess => "postprocess",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecurityRulePriority {
    Explicit(i32),
    Named(SecurityRulePriorityName),
}

impl SecurityRulePriority {
    pub const fn resolve(self) -> i32 {
        match self {
            Self::Explicit(priority) => priority,
            Self::Named(SecurityRulePriorityName::Default) => DEFAULT_RULE_PRIORITY,
        }
    }

    pub const fn is_named_default(self) -> bool {
        matches!(self, Self::Named(SecurityRulePriorityName::Default))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityRulePriorityName {
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityPluginMode {
    Disable,
    Allow,
    Ask,
    Block,
    #[serde(alias = "redact", alias = "mutate", alias = "neutralize")]
    Rewrite,
}

impl SecurityPluginMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
            Self::Rewrite => "rewrite",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityPluginConfig {
    pub mode: SecurityPluginMode,
    #[serde(default = "default_plugin_detection_level")]
    pub detection_level: DetectionLevel,
}

impl SecurityPluginConfig {
    pub const fn active_detection_level(self) -> Option<DetectionLevel> {
        match self.mode {
            SecurityPluginMode::Disable => None,
            SecurityPluginMode::Allow
            | SecurityPluginMode::Ask
            | SecurityPluginMode::Block
            | SecurityPluginMode::Rewrite => Some(self.detection_level),
        }
    }
}

const fn default_plugin_detection_level() -> DetectionLevel {
    DetectionLevel::Informational
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionLevel {
    #[serde(alias = "info")]
    Informational,
    Low,
    Medium,
    High,
    Critical,
}

impl DetectionLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Informational => "informational",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityRuleSource {
    BuiltinDefault,
    User,
    Corp,
}

impl SecurityRuleSource {
    pub const fn default_priority(self, corp_locked: bool) -> i32 {
        if corp_locked || matches!(self, Self::Corp) {
            CORP_PRIORITY_MAX
        } else if matches!(self, Self::BuiltinDefault) {
            DEFAULT_RULE_PRIORITY
        } else {
            USER_PRIORITY_MIN
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompiledSecurityRule {
    pub rule_id: String,
    pub provider: String,
    pub namespace: String,
    pub rule_key: String,
    pub default_rule: bool,
    pub enabled: bool,
    pub name: String,
    pub action: SecurityRuleAction,
    pub condition: String,
    compiled_condition: CompiledCondition,
    pub detection_level: Option<DetectionLevel>,
    pub priority: i32,
    pub corp_locked: bool,
    pub reason: Option<String>,
    pub managed: Option<SecurityRuleManagedTarget>,
}

#[derive(Debug, Clone)]
pub struct SecurityRuleSet {
    rules: Vec<CompiledSecurityRule>,
}

#[derive(Debug, Clone)]
pub struct SecurityRuleEvaluation<'a> {
    matched_rules: Vec<&'a CompiledSecurityRule>,
}

impl SecurityRuleProfile {
    pub fn parse_toml(input: &str) -> Result<Self, String> {
        let profile: Self =
            toml::from_str(input).map_err(|error| format!("security rule TOML: {error}"))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn parse_sigma_yaml(input: &str) -> Result<Self, String> {
        let mut profile = Self::default();
        let mut parsed_any = false;
        for document in serde_yaml::Deserializer::from_str(input) {
            let sigma_rule = SigmaRule::deserialize(document)
                .map_err(|error| format!("security rule Sigma YAML: {error}"))?;
            let (rule_key, rule) = sigma_rule.into_security_rule()?;
            if profile
                .profiles
                .rules
                .insert(rule_key.clone(), rule)
                .is_some()
            {
                return Err(format!("duplicate Sigma-derived rule '{rule_key}'"));
            }
            parsed_any = true;
        }
        if !parsed_any {
            return Err("security rule Sigma YAML: no rules found".to_string());
        }
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_default_rules(&self.default)?;
        validate_rule_group("corp", &self.corp)?;
        validate_rule_group("profiles", &self.profiles)?;
        for plugin_id in self.plugins.keys() {
            validate_identifier("plugin id", plugin_id)?;
        }
        for (provider_id, provider) in &self.ai {
            validate_identifier("provider id", provider_id)?;
            if let Some(name) = provider.name.as_deref() {
                validate_non_empty("provider name", name)?;
            }
            if let Some(protocol) = provider.protocol.as_deref() {
                validate_identifier("provider protocol", protocol)?;
            }
            if let Some(url) = provider.url.as_deref() {
                validate_non_empty("provider url", url)?;
            }
            for alias in &provider.aliases {
                validate_non_empty("provider alias", alias)?;
            }
            for listen_port in &provider.listen_ports {
                if *listen_port == 0 {
                    return Err(format!("ai.{provider_id}.listen_ports cannot include 0"));
                }
            }
            for target in &provider.allowed_remote_targets {
                validate_non_empty("provider allowed_remote_target", target)?;
            }
            if let Some(discovery) = &provider.discovery {
                discovery.validate(&format!("ai.{provider_id}.discovery"))?;
            }
            if provider.rules.is_empty() && provider.discovery.is_none() {
                return Err(format!(
                    "ai.{provider_id} must define at least one rule or discovery record"
                ));
            }
            for (rule_key, rule) in &provider.rules {
                validate_identifier("rule id", rule_key)?;
                rule.validate(&format!("ai.{provider_id}.rules.{rule_key}"))?;
            }
        }
        validate_managed_targets_unique(self)?;
        Ok(())
    }

    pub fn compile(&self, source: SecurityRuleSource) -> Result<Vec<CompiledSecurityRule>, String> {
        self.validate()?;
        let mut compiled = Vec::new();
        self.compile_default_rules(source, &mut compiled)?;
        self.compile_group(
            "corp",
            "corp",
            &self.corp,
            SecurityRuleSource::Corp,
            &mut compiled,
        )?;
        self.compile_group(
            "profiles",
            "profiles",
            &self.profiles,
            source,
            &mut compiled,
        )?;
        for (provider_id, provider) in &self.ai {
            for (rule_key, rule) in &provider.rules {
                let priority = rule.effective_priority(source)?;
                let compiled_condition = rule.compile_match()?;
                compiled.push(CompiledSecurityRule {
                    rule_id: format!("profiles.rules.ai_{provider_id}_{rule_key}"),
                    provider: provider_id.clone(),
                    namespace: "profiles".to_string(),
                    rule_key: rule_key.clone(),
                    default_rule: false,
                    enabled: rule.enabled,
                    name: rule.name.clone(),
                    action: rule.action,
                    condition: rule.condition.clone(),
                    compiled_condition,
                    detection_level: rule.detection_level,
                    priority,
                    corp_locked: rule.corp_locked || matches!(source, SecurityRuleSource::Corp),
                    reason: rule.reason.clone(),
                    managed: rule.managed.clone(),
                });
            }
        }
        compiled.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        Ok(compiled)
    }

    fn compile_default_rules(
        &self,
        source: SecurityRuleSource,
        compiled: &mut Vec<CompiledSecurityRule>,
    ) -> Result<(), String> {
        for (rule_key, rule) in &self.default {
            let priority = rule.effective_priority(source)?;
            let compiled_condition = rule.compile_match()?;
            let compiled_rule_key = format!("default_{rule_key}");
            compiled.push(CompiledSecurityRule {
                rule_id: format!("profiles.rules.{compiled_rule_key}"),
                provider: "profiles".to_string(),
                namespace: "profiles".to_string(),
                rule_key: compiled_rule_key,
                default_rule: true,
                enabled: rule.enabled,
                name: rule.name.clone(),
                action: rule.action,
                condition: rule.condition.clone(),
                compiled_condition,
                detection_level: rule.detection_level,
                priority,
                corp_locked: rule.corp_locked || matches!(source, SecurityRuleSource::Corp),
                reason: rule.reason.clone(),
                managed: rule.managed.clone(),
            });
        }
        Ok(())
    }

    fn compile_group(
        &self,
        namespace: &str,
        provider: &str,
        group: &SecurityRuleGroup,
        source: SecurityRuleSource,
        compiled: &mut Vec<CompiledSecurityRule>,
    ) -> Result<(), String> {
        for (rule_key, rule) in &group.rules {
            let priority = rule.effective_priority(source)?;
            let compiled_condition = rule.compile_match()?;
            compiled.push(CompiledSecurityRule {
                rule_id: format!("{namespace}.rules.{rule_key}"),
                provider: provider.to_string(),
                namespace: namespace.to_string(),
                rule_key: rule_key.clone(),
                default_rule: false,
                enabled: rule.enabled,
                name: rule.name.clone(),
                action: rule.action,
                condition: rule.condition.clone(),
                compiled_condition,
                detection_level: rule.detection_level,
                priority,
                corp_locked: rule.corp_locked || matches!(source, SecurityRuleSource::Corp),
                reason: rule.reason.clone(),
                managed: rule.managed.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigmaRule {
    title: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "status")]
    _status: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "author")]
    _author: Option<String>,
    #[serde(default, rename = "date")]
    _date: Option<String>,
    logsource: SigmaLogsource,
    detection: BTreeMap<String, serde_yaml::Value>,
    #[serde(default, rename = "falsepositives")]
    _falsepositives: Vec<String>,
    level: DetectionLevel,
    #[serde(default)]
    capsem: SigmaCapsem,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigmaLogsource {
    product: String,
    service: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigmaCapsem {
    #[serde(default)]
    action: Option<SecurityRuleAction>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    priority: Option<SecurityRulePriority>,
    #[serde(default)]
    corp_locked: bool,
}

impl SigmaRule {
    fn into_security_rule(self) -> Result<(String, SecurityRule), String> {
        if self.logsource.product != "capsem" || self.logsource.service != "security_event" {
            return Err(format!(
                "Sigma rule '{}' must use logsource product=capsem service=security_event",
                self.title
            ));
        }
        let condition = self
            .detection
            .get("condition")
            .and_then(serde_yaml::Value::as_str)
            .ok_or_else(|| format!("Sigma rule '{}' missing detection.condition", self.title))?;
        let selections = self.selection_clauses()?;
        let condition = sigma_condition_to_security_event_match(condition, &selections)?;
        let rule_key = derive_sigma_rule_key(&self.title)?;
        let rule = SecurityRule {
            name: rule_key.clone(),
            action: self.capsem.action.unwrap_or(SecurityRuleAction::Allow),
            condition,
            enabled: true,
            detection_level: Some(self.level),
            priority: self.capsem.priority,
            corp_locked: self.capsem.corp_locked,
            reason: self
                .capsem
                .reason
                .or(self.description)
                .or_else(|| self.id.map(|id| format!("Sigma rule {id}"))),
            managed: None,
            plugin_config: BTreeMap::new(),
        };
        rule.validate(&format!("profiles.rules.{rule_key}"))?;
        Ok((rule_key, rule))
    }

    fn selection_clauses(&self) -> Result<BTreeMap<String, SigmaSelectionClause>, String> {
        let mut selections = BTreeMap::new();
        for (name, value) in &self.detection {
            if name == "condition" {
                continue;
            }
            validate_identifier("Sigma selection id", name)?;
            let mapping = value
                .as_mapping()
                .ok_or_else(|| format!("Sigma selection '{name}' must be a mapping"))?;
            let mut positive = Vec::new();
            let mut negative = Vec::new();
            for (field, expected) in mapping {
                let field = field
                    .as_str()
                    .ok_or_else(|| format!("Sigma selection '{name}' has a non-string field"))?;
                validate_security_event_field(field)?;
                let clause = sigma_field_clause(field, expected)?;
                positive.push(clause.positive);
                negative.push(clause.negative);
            }
            if positive.is_empty() {
                return Err(format!("Sigma selection '{name}' must not be empty"));
            }
            selections.insert(
                name.clone(),
                SigmaSelectionClause {
                    positive: positive.join(" && "),
                    negative: negative.join(" || "),
                },
            );
        }
        Ok(selections)
    }
}

#[derive(Debug, Clone)]
struct SigmaSelectionClause {
    positive: String,
    negative: String,
}

fn sigma_condition_to_security_event_match(
    condition: &str,
    selections: &BTreeMap<String, SigmaSelectionClause>,
) -> Result<String, String> {
    let tokens = tokenize_sigma_condition(condition)?;
    let mut output = Vec::new();
    let mut negate_next = false;
    for token in tokens {
        match token.as_str() {
            "and" => output.push("&&".to_string()),
            "or" => output.push("||".to_string()),
            "not" => {
                if negate_next {
                    return Err("Sigma condition has repeated 'not'".to_string());
                }
                negate_next = true;
            }
            "(" | ")" => {
                return Err("Sigma condition grouping is not supported yet".to_string());
            }
            name => {
                let clause = selections.get(name).ok_or_else(|| {
                    format!("Sigma condition references unknown selection '{name}'")
                })?;
                if negate_next {
                    output.push(clause.negative.clone());
                    negate_next = false;
                } else {
                    output.push(clause.positive.clone());
                }
            }
        }
    }
    if negate_next {
        return Err("Sigma condition ends with 'not'".to_string());
    }
    Ok(output.join(" "))
}

fn tokenize_sigma_condition(condition: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in condition.chars() {
        match ch {
            '(' | ')' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch if ch == '_' || ch.is_ascii_alphanumeric() => current.push(ch),
            _ => {
                return Err(format!(
                    "unsupported Sigma condition token near '{ch}' in '{condition}'"
                ));
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        Err("Sigma condition must not be empty".to_string())
    } else {
        Ok(tokens)
    }
}

fn sigma_field_clause(
    field: &str,
    expected: &serde_yaml::Value,
) -> Result<SigmaSelectionClause, String> {
    if let Some(values) = expected.as_sequence() {
        if values.is_empty() {
            return Err(format!("Sigma field '{field}' sequence must not be empty"));
        }
        let mut positive = Vec::new();
        let mut negative = Vec::new();
        for value in values {
            positive.push(sigma_scalar_compare(field, "==", value)?);
            negative.push(sigma_scalar_compare(field, "!=", value)?);
        }
        return Ok(SigmaSelectionClause {
            positive: positive.join(" || "),
            negative: negative.join(" && "),
        });
    }
    Ok(SigmaSelectionClause {
        positive: sigma_scalar_compare(field, "==", expected)?,
        negative: sigma_scalar_compare(field, "!=", expected)?,
    })
}

fn sigma_scalar_compare(
    field: &str,
    operator: &str,
    expected: &serde_yaml::Value,
) -> Result<String, String> {
    let expected = sigma_scalar_to_string(expected)
        .ok_or_else(|| format!("Sigma field '{field}' value must be a scalar or sequence"))?;
    Ok(format!(
        "{field} {operator} {}",
        cel_string_literal(&expected)
    ))
}

fn sigma_scalar_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        serde_yaml::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn cel_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("string literal serialization cannot fail")
}

fn derive_sigma_rule_key(title: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut last_was_sep = true;
    for ch in title.chars() {
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
    if output.len() > 64 {
        output.truncate(64);
        while output.ends_with('_') {
            output.pop();
        }
    }
    validate_identifier("Sigma-derived rule id", &output)?;
    Ok(output)
}

impl SecurityRuleSet {
    pub fn new(mut rules: Vec<CompiledSecurityRule>) -> Self {
        rules.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        Self { rules }
    }

    pub fn compile_profile(
        profile: &SecurityRuleProfile,
        source: SecurityRuleSource,
    ) -> Result<Self, String> {
        profile.compile(source).map(Self::new)
    }

    pub fn rules(&self) -> &[CompiledSecurityRule] {
        &self.rules
    }

    pub fn evaluate<S>(&self, subject: &S) -> Result<SecurityRuleEvaluation<'_>, String>
    where
        S: PolicySubject + ?Sized,
    {
        let mut matched_rules = Vec::new();
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if rule.matches_security_event(subject)? {
                matched_rules.push(rule);
            }
        }
        Ok(SecurityRuleEvaluation { matched_rules })
    }
}

impl<'a> SecurityRuleEvaluation<'a> {
    pub fn matched_rules(&self) -> &[&'a CompiledSecurityRule] {
        &self.matched_rules
    }

    pub fn detections(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| rule.detection_level.is_some())
            .collect()
    }

    pub fn rules_for_action(&self, action: SecurityRuleAction) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| rule.action == action)
            .collect()
    }

    pub fn preprocess_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| {
                matches!(
                    rule.action,
                    SecurityRuleAction::Preprocess | SecurityRuleAction::Rewrite
                )
            })
            .collect()
    }

    pub fn postprocess_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.rules_for_action(SecurityRuleAction::Postprocess)
    }

    pub fn enforcement_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| {
                matches!(
                    rule.action,
                    SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block
                )
            })
            .collect()
    }
}

impl ProviderDiscovery {
    pub fn validate(&self, path: &str) -> Result<(), String> {
        validate_non_empty(&format!("{path}.observed_at"), &self.observed_at)?;
        validate_non_empty(&format!("{path}.source"), &self.source)?;
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(format!("{path}.confidence must be between 0 and 1"));
        }
        if let Some(event_type) = self.event_type.as_deref() {
            crate::security_engine::RuntimeSecurityEventType::try_from(event_type)
                .map_err(|error| format!("{path}.event_type: {error}"))?;
        }
        if let Some(credential_ref) = self.credential_ref.as_deref() {
            if !capsem_logger::is_credential_reference(credential_ref) {
                return Err(format!(
                    "{path}.credential_ref must be a credential:blake3 reference"
                ));
            }
        }
        Ok(())
    }
}

impl SecurityRule {
    pub fn validate(&self, rule_id: &str) -> Result<(), String> {
        validate_rule_name("rule name", &self.name)?;
        validate_non_empty("rule match", &self.condition)?;
        if self.plugin_config.contains_key("on") {
            return Err(format!("{rule_id} must not use 'on'"));
        }
        if self.plugin_config.contains_key("if") {
            return Err(format!("{rule_id} must not use 'if'; use 'match'"));
        }
        if self.plugin_config.contains_key("decision") {
            return Err(format!("{rule_id} must not use 'decision'; use 'action'"));
        }
        if self.plugin_config.contains_key("actions") {
            return Err(format!(
                "{rule_id} must not use 'actions'; use one 'action'"
            ));
        }
        if self.plugin_config.contains_key("level") {
            return Err(format!(
                "{rule_id} must not use 'level'; use 'detection_level'"
            ));
        }
        if self.plugin_config.contains_key("plugin") {
            return Err(format!(
                "{rule_id} must not use 'plugin'; plugins own their filtering"
            ));
        }
        if let Some(managed) = &self.managed {
            managed.validate(rule_id)?;
        }
        if !self.plugin_config.is_empty() {
            let fields = self
                .plugin_config
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("{rule_id} has unknown rule fields: {fields}"));
        }
        self.validate_match()?;
        Ok(())
    }

    pub fn effective_priority(&self, source: SecurityRuleSource) -> Result<i32, String> {
        let priority = self
            .priority
            .map(SecurityRulePriority::resolve)
            .unwrap_or_else(|| source.default_priority(self.corp_locked));
        validate_priority_for_source(
            &self.name,
            source,
            self.corp_locked,
            self.priority,
            priority,
        )?;
        Ok(priority)
    }

    pub fn validate_match(&self) -> Result<(), String> {
        validate_security_event_match(&self.condition)
    }

    pub fn compile_match(&self) -> Result<CompiledCondition, String> {
        compile_security_event_match(&self.condition)
    }

    pub fn matches_security_event<S>(&self, subject: &S) -> Result<bool, String>
    where
        S: PolicySubject + ?Sized,
    {
        evaluate_security_event_match(&self.condition, subject)
    }
}

impl CompiledSecurityRule {
    pub fn matches_security_event<S>(&self, subject: &S) -> Result<bool, String>
    where
        S: PolicySubject + ?Sized,
    {
        self.compiled_condition.evaluate(subject)
    }
}

fn validate_priority_for_source(
    rule_name: &str,
    source: SecurityRuleSource,
    corp_locked: bool,
    raw_priority: Option<SecurityRulePriority>,
    priority: i32,
) -> Result<(), String> {
    if raw_priority.is_some_and(SecurityRulePriority::is_named_default) {
        if corp_locked || matches!(source, SecurityRuleSource::Corp) {
            return Err(format!(
                "rule '{rule_name}' corp priority cannot use named default priority"
            ));
        }
        return Ok(());
    }
    if matches!(source, SecurityRuleSource::BuiltinDefault)
        && raw_priority.is_none()
        && priority == DEFAULT_RULE_PRIORITY
    {
        return Ok(());
    }

    if !(CORP_PRIORITY_MIN..=USER_PRIORITY_MAX).contains(&priority) {
        return Err(format!(
            "rule '{rule_name}' priority {priority} must be between -1000 and 1000"
        ));
    }
    if corp_locked || matches!(source, SecurityRuleSource::Corp) {
        if priority <= CORP_PRIORITY_MAX {
            return Ok(());
        }
        return Err(format!(
            "rule '{rule_name}' corp priority {priority} must be <= -10"
        ));
    }

    match source {
        SecurityRuleSource::BuiltinDefault => {
            if priority == DEFAULT_RULE_PRIORITY {
                Ok(())
            } else {
                Err(format!(
                    "rule '{rule_name}' default priority {priority} must be default"
                ))
            }
        }
        SecurityRuleSource::User => {
            if priority < 0 {
                Err(format!(
                    "rule '{rule_name}' user/plugin priority {priority} cannot use negative priority"
                ))
            } else if priority >= USER_PRIORITY_MIN {
                Ok(())
            } else {
                Err(format!(
                    "rule '{rule_name}' user/plugin priority {priority} must be >= 10"
                ))
            }
        }
        SecurityRuleSource::Corp => unreachable!("corp source handled above"),
    }
}

fn validate_rule_group(namespace: &str, group: &SecurityRuleGroup) -> Result<(), String> {
    for (rule_key, rule) in &group.rules {
        validate_identifier("rule id", rule_key)?;
        rule.validate(&format!("{namespace}.rules.{rule_key}"))?;
    }
    Ok(())
}

fn validate_default_rules(default: &BTreeMap<String, SecurityRule>) -> Result<(), String> {
    for (rule_key, rule) in default {
        validate_identifier("default rule id", rule_key)?;
        rule.validate(&format!("default.{rule_key}"))?;
    }
    Ok(())
}

fn validate_managed_targets_unique(profile: &SecurityRuleProfile) -> Result<(), String> {
    let mut seen = BTreeMap::new();
    for (rule_key, rule) in &profile.default {
        track_managed_target(&mut seen, format!("default.{rule_key}"), rule)?;
    }
    for (rule_key, rule) in &profile.corp.rules {
        track_managed_target(&mut seen, format!("corp.rules.{rule_key}"), rule)?;
    }
    for (rule_key, rule) in &profile.profiles.rules {
        track_managed_target(&mut seen, format!("profiles.rules.{rule_key}"), rule)?;
    }
    for (provider_id, provider) in &profile.ai {
        for (rule_key, rule) in &provider.rules {
            track_managed_target(
                &mut seen,
                format!("ai.{provider_id}.rules.{rule_key}"),
                rule,
            )?;
        }
    }
    Ok(())
}

fn track_managed_target(
    seen: &mut BTreeMap<String, String>,
    rule_id: String,
    rule: &SecurityRule,
) -> Result<(), String> {
    let Some(managed) = &rule.managed else {
        return Ok(());
    };
    let identity = managed.identity_key();
    if let Some(previous) = seen.insert(identity.clone(), rule_id.clone()) {
        return Err(format!(
            "managed security rule target {identity} is defined by both {previous} and {rule_id}"
        ));
    }
    Ok(())
}

pub fn validate_security_event_match(condition: &str) -> Result<(), String> {
    validate_condition_with(condition, validate_security_event_field)
}

pub fn compile_security_event_match(condition: &str) -> Result<CompiledCondition, String> {
    CompiledCondition::parse_with(condition, validate_security_event_field)
}

pub fn evaluate_security_event_match<S>(condition: &str, subject: &S) -> Result<bool, String>
where
    S: PolicySubject + ?Sized,
{
    evaluate_condition_with(condition, subject, validate_security_event_field)
}

fn validate_security_event_field(field: &str) -> Result<(), String> {
    let Some(root) = field.split('.').next() else {
        return Err("security-event CEL field must not be empty".to_string());
    };
    if SECURITY_EVENT_CEL_ROOTS.contains(&root) {
        Ok(())
    } else {
        Err(format!(
            "field '{field}' is not a first-party security-event root"
        ))
    }
}

pub(crate) fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    validate_non_empty(kind, value)?;
    if value.len() > 64 {
        return Err(format!("{kind} must be at most 64 characters"));
    }
    if value
        .chars()
        .all(|ch| ch == '_' || ch == '-' || ch.is_ascii_lowercase() || ch.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(format!(
            "{kind} must use only lowercase a-z, 0-9, '_' or '-': {value}"
        ))
    }
}

fn validate_rule_name(kind: &str, value: &str) -> Result<(), String> {
    validate_identifier(kind, value)
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

#[cfg(test)]
mod tests;
