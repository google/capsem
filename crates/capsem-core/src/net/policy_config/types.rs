/// Generic typed settings system with corp override.
///
/// Each setting has an id, name, description, type, category, default value,
/// and optional `enabled_by` pointer to a parent toggle. Settings are stored
/// in TOML files at:
///   - User: ~/.capsem/user.toml
///   - Corporate: /etc/capsem/corp.toml
///
/// Merge semantics: corp settings override user settings per-key.
/// User can only write user.toml. Corp file is read-only (MDM-distributed).
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::condition::{evaluate_policy_condition, validate_policy_condition};

const DEFAULT_POLICY_RULE_PRIORITY: i32 = 1000;

// ---------------------------------------------------------------------------
// Setting ID constants (must match defaults.toml paths)
// ---------------------------------------------------------------------------

pub const SETTING_ANTHROPIC_ALLOW: &str = "ai.anthropic.allow";
pub const SETTING_ANTHROPIC_API_KEY: &str = "ai.anthropic.api_key";
pub const SETTING_OPENAI_ALLOW: &str = "ai.openai.allow";
pub const SETTING_OPENAI_API_KEY: &str = "ai.openai.api_key";
pub const SETTING_GOOGLE_ALLOW: &str = "ai.google.allow";
pub const SETTING_GOOGLE_API_KEY: &str = "ai.google.api_key";
pub const SETTING_GITHUB_ALLOW: &str = "repository.providers.github.allow";
pub const SETTING_GITHUB_TOKEN: &str = "repository.providers.github.token";
pub const SETTING_GITLAB_ALLOW: &str = "repository.providers.gitlab.allow";
pub const SETTING_GITLAB_TOKEN: &str = "repository.providers.gitlab.token";
pub const SETTING_SSH_PUBLIC_KEY: &str = "vm.environment.ssh.public_key";

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// The data type of a setting (drives UI rendering).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    Text,
    Number,
    Url,
    Email,
    #[serde(rename = "apikey")]
    ApiKey,
    Bool,
    /// File to write to a guest path. Value is `{ path, content }`.
    /// JSON files (.json extension) are validated on save.
    File,
    /// Key-value string map (e.g. env vars, HTTP headers).
    KvMap,
    /// List of strings (e.g. domain patterns, tags).
    StringList,
    /// List of integers.
    IntList,
    /// List of floats.
    FloatList,
    /// An MCP tool discovered from a server.
    McpTool,
}

/// Explicit UI widget override. When set on a setting's metadata,
/// the frontend renders this widget instead of inferring from SettingType.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    Toggle,
    TextInput,
    NumberInput,
    PasswordInput,
    Select,
    FileEditor,
    DomainChips,
    StringChips,
    Slider,
    KvEditor,
}

/// Frontend side effect triggered when a setting value changes.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    ToggleTheme,
}

/// Action identifier for grammar-driven action nodes (buttons/widgets).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    CheckUpdate,
    PresetSelect,
    RerunWizard,
}

/// MCP server transport protocol.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
}

/// Where an MCP tool runs.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolOrigin {
    Builtin,
    Remote,
    InVm,
}

/// A setting value (untagged for clean TOML serialization).
///
/// Variant order matters: `#[serde(untagged)]` tries variants top-to-bottom.
/// `File` (a table with `path` + `content`) must come before `Text` (a plain
/// string) so TOML tables like `{ path = "...", content = "..." }` deserialize
/// as `File` rather than failing on `Text`.
/// List variants must come before `Text` so arrays deserialize correctly.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(i64),
    Float(f64),
    File { path: String, content: String },
    KvMap(HashMap<String, String>),
    StringList(Vec<String>),
    IntList(Vec<i64>),
    FloatList(Vec<f64>),
    Text(String),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            SettingValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            SettingValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_file(&self) -> Option<(&str, &str)> {
        match self {
            SettingValue::File { path, content } => Some((path, content)),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            SettingValue::Float(f) => Some(*f),
            SettingValue::Number(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_string_list(&self) -> Option<&[String]> {
        match self {
            SettingValue::StringList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_int_list(&self) -> Option<&[i64]> {
        match self {
            SettingValue::IntList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_float_list(&self) -> Option<&[f64]> {
        match self {
            SettingValue::FloatList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_kv_map(&self) -> Option<&HashMap<String, String>> {
        match self {
            SettingValue::KvMap(m) => Some(m),
            _ => None,
        }
    }
}

/// Per-rule HTTP method permissions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct HttpMethodPermissions {
    /// Optional per-rule domain subset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// Path pattern (e.g., "/repos/*").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub get: bool,
    #[serde(default)]
    pub post: bool,
    #[serde(default)]
    pub put: bool,
    #[serde(default)]
    pub delete: bool,
    /// All methods not listed above.
    #[serde(default)]
    pub other: bool,
}

/// Structured metadata for a setting.
///
/// Note: `skip_serializing_if` is intentionally NOT used on collection fields.
/// The frontend accesses fields like `metadata.choices.length` directly, so
/// omitting empty fields from JSON would cause `undefined.length` TypeErrors.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingMetadata {
    /// Domain patterns for network settings.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Valid values for text choice settings.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Minimum for number settings.
    #[serde(default)]
    pub min: Option<i64>,
    /// Maximum for number settings.
    #[serde(default)]
    pub max: Option<i64>,
    /// HTTP rules (keyed by rule name).
    #[serde(default)]
    pub rules: HashMap<String, HttpMethodPermissions>,
    /// Env var name(s) to inject in the guest when this setting is non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    /// Whether this setting or section starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
    /// Display format hint (DEPRECATED: use `widget` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Documentation URL (applies to any setting type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Expected token/key prefix hint for the UI (e.g. "ghp_", "sk-ant-").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// File type hint for syntax highlighting (e.g. "json", "bash", "conf").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filetype: Option<String>,
    /// Explicit UI widget override. When set, the frontend renders this widget
    /// instead of inferring from setting_type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<Widget>,
    /// Frontend side effect triggered when the value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect: Option<SideEffect>,
    /// Step increment for number settings (e.g. 1 for integers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
    /// Setting is hidden from the UI but still active for policy building.
    #[serde(default)]
    pub hidden: bool,
    /// Non-removable by user (e.g. built-in MCP servers).
    #[serde(default)]
    pub builtin: bool,
    /// Render as masked input (replaces the old `password` SettingType).
    #[serde(default)]
    pub mask: bool,
    /// Regex pattern for value validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    /// MCP tool origin (builtin, remote, in_vm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<McpToolOrigin>,
}

/// Schema definition for a setting (loaded from defaults.toml at compile time).
pub struct SettingDef {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    /// Parent toggle ID (child is greyed out when parent is off).
    pub enabled_by: Option<String>,
    pub metadata: SettingMetadata,
}

/// A single stored setting entry in TOML.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SettingEntry {
    pub value: SettingValue,
    pub modified: String,
}

// ---------------------------------------------------------------------------
// Policy V2 named rule config
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyCallback {
    #[serde(rename = "mcp.request")]
    McpRequest,
    #[serde(rename = "mcp.response")]
    McpResponse,
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.response")]
    HttpResponse,
    #[serde(rename = "dns.query")]
    DnsQuery,
    #[serde(rename = "dns.response")]
    DnsResponse,
    #[serde(rename = "model.request")]
    ModelRequest,
    #[serde(rename = "model.response")]
    ModelResponse,
    #[serde(rename = "model.tool_call")]
    ModelToolCall,
    #[serde(rename = "model.tool_response")]
    ModelToolResponse,
    #[serde(rename = "hook.decision")]
    HookDecision,
}

impl PolicyCallback {
    pub fn policy_type(self) -> PolicyRuleType {
        match self {
            PolicyCallback::McpRequest | PolicyCallback::McpResponse => PolicyRuleType::Mcp,
            PolicyCallback::HttpRequest | PolicyCallback::HttpResponse => PolicyRuleType::Http,
            PolicyCallback::DnsQuery | PolicyCallback::DnsResponse => PolicyRuleType::Dns,
            PolicyCallback::ModelRequest
            | PolicyCallback::ModelResponse
            | PolicyCallback::ModelToolCall
            | PolicyCallback::ModelToolResponse => PolicyRuleType::Model,
            PolicyCallback::HookDecision => PolicyRuleType::Hook,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyDecisionKind {
    Allow,
    Ask,
    Block,
    Rewrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySubjectValue<'a> {
    String(Cow<'a, str>),
    Bool(bool),
    Present,
}

impl<'a> PolicySubjectValue<'a> {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_ref()),
            Self::Bool(true) => Some("true"),
            Self::Bool(false) => Some("false"),
            Self::Present => None,
        }
    }
}

pub trait PolicySubject {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>>;
}

impl PolicySubject for serde_json::Value {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        let mut current = self;
        for segment in field.split('.') {
            current = current.get(segment)?;
        }
        match current {
            serde_json::Value::String(value) => {
                Some(PolicySubjectValue::String(Cow::Borrowed(value.as_str())))
            }
            serde_json::Value::Bool(value) => Some(PolicySubjectValue::Bool(*value)),
            serde_json::Value::Number(value) => {
                Some(PolicySubjectValue::String(Cow::Owned(value.to_string())))
            }
            serde_json::Value::Null
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => Some(PolicySubjectValue::Present),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyRuleType {
    Mcp,
    Http,
    Dns,
    Model,
    Hook,
}

impl PolicyRuleType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "mcp" => Some(Self::Mcp),
            "http" => Some(Self::Http),
            "dns" => Some(Self::Dns),
            "model" => Some(Self::Model),
            "hook" => Some(Self::Hook),
            _ => None,
        }
    }
}

/// One named `policy.<type>.<rule_name>` rule from user.toml/corp.toml.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PolicyRuleConfig {
    #[serde(rename = "on")]
    pub on: PolicyCallback,
    #[serde(rename = "if")]
    pub condition: String,
    pub decision: PolicyDecisionKind,
    #[serde(default = "default_policy_rule_priority")]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_request_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_response_headers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchedPolicyRule<'a> {
    pub name: &'a str,
    pub rule: &'a PolicyRuleConfig,
}

fn default_policy_rule_priority() -> i32 {
    DEFAULT_POLICY_RULE_PRIORITY
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicyRuleConfig {
    #[serde(rename = "on")]
    on: PolicyCallback,
    #[serde(rename = "if")]
    condition: String,
    decision: PolicyDecisionKind,
    #[serde(default = "default_policy_rule_priority")]
    priority: i32,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    rewrite_target: Option<String>,
    #[serde(default)]
    rewrite_value: Option<String>,
    #[serde(default)]
    strip_request_headers: Vec<String>,
    #[serde(default)]
    strip_response_headers: Vec<String>,
}

impl<'de> Deserialize<'de> for PolicyRuleConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPolicyRuleConfig::deserialize(deserializer)?;
        let strip_request_headers =
            normalize_header_names("strip_request_headers", raw.strip_request_headers)
                .map_err(serde::de::Error::custom)?;
        let strip_response_headers =
            normalize_header_names("strip_response_headers", raw.strip_response_headers)
                .map_err(serde::de::Error::custom)?;
        let rule = Self {
            on: raw.on,
            condition: raw.condition,
            decision: raw.decision,
            priority: raw.priority,
            reason: raw.reason,
            rewrite_target: raw.rewrite_target,
            rewrite_value: raw.rewrite_value,
            strip_request_headers,
            strip_response_headers,
        };
        rule.validate().map_err(serde::de::Error::custom)?;
        Ok(rule)
    }
}

impl PolicyRuleConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.condition.trim().is_empty() {
            return Err("policy rule requires a non-empty CEL condition".into());
        }
        validate_policy_condition(self.on, &self.condition)?;

        match self.decision {
            PolicyDecisionKind::Rewrite => {
                let has_target = self
                    .rewrite_target
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty());
                let has_value = self
                    .rewrite_value
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty());
                let has_header_strip = !self.strip_request_headers.is_empty()
                    || !self.strip_response_headers.is_empty();

                if has_target != has_value {
                    return Err("rewrite requires both rewrite_target and rewrite_value".into());
                }
                if !has_target && !has_header_strip {
                    return Err(
                        "rewrite requires rewrite_target/rewrite_value or header strip fields"
                            .into(),
                    );
                }
                if has_target {
                    validate_rewrite_target_and_value(
                        self.rewrite_target.as_deref().unwrap_or_default(),
                        self.rewrite_value.as_deref().unwrap_or_default(),
                    )?;
                }
            }
            PolicyDecisionKind::Allow | PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                if self.rewrite_target.is_some()
                    || self.rewrite_value.is_some()
                    || !self.strip_request_headers.is_empty()
                    || !self.strip_response_headers.is_empty()
                {
                    return Err("only rewrite decisions may carry rewrite fields".into());
                }
            }
        }

        Ok(())
    }
}

fn validate_rewrite_target_and_value(target: &str, value: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("rewrite_target must not be empty".into());
    }

    let captures = rewrite_target_captures(target)?;
    let replacement_references = replacement_capture_references(value)?;
    for reference in replacement_references {
        if !captures.contains(&reference) {
            return Err(format!(
                "rewrite_value references unknown capture '{reference}'"
            ));
        }
    }
    Ok(())
}

fn rewrite_target_captures(target: &str) -> Result<HashSet<String>, String> {
    let Some((_, rhs)) = target.split_once("=~") else {
        return Ok(HashSet::new());
    };
    let regex_text = rhs.trim();
    if regex_text.len() < 2 {
        return Err("rewrite_target regex must be quoted".into());
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        return Err("rewrite_target regex must be quoted".into());
    }
    let Some(end) = regex_text[1..].rfind(quote) else {
        return Err("rewrite_target regex is missing a closing quote".into());
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        return Err("rewrite_target regex has trailing content after closing quote".into());
    }
    let pattern = &regex_text[1..=end];
    let compiled =
        regex::Regex::new(pattern).map_err(|e| format!("invalid rewrite_target regex: {e}"))?;
    Ok(compiled
        .capture_names()
        .flatten()
        .map(ToOwned::to_owned)
        .collect())
}

fn replacement_capture_references(value: &str) -> Result<Vec<String>, String> {
    let reference_re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")
        .map_err(|e| format!("invalid replacement reference regex: {e}"))?;
    Ok(reference_re
        .captures_iter(value)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .collect())
}

fn normalize_header_names(field: &str, headers: Vec<String>) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for header in headers {
        let trimmed = header.trim();
        if trimmed.is_empty() {
            return Err(format!("{field} contains an empty HTTP header name"));
        }
        let name = http::header::HeaderName::from_bytes(trimmed.as_bytes())
            .map_err(|_| format!("{field} contains invalid HTTP header name '{header}'"))?;
        let name = name.as_str().to_string();
        if seen.insert(name.clone()) {
            normalized.push(name);
        }
    }
    Ok(normalized)
}

/// All configured named Policy V2 rules.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct PolicyConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mcp: HashMap<String, PolicyRuleConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub http: HashMap<String, PolicyRuleConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dns: HashMap<String, PolicyRuleConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub model: HashMap<String, PolicyRuleConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub hook: HashMap<String, PolicyRuleConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicyConfig {
    #[serde(default)]
    mcp: HashMap<String, PolicyRuleConfig>,
    #[serde(default)]
    http: HashMap<String, PolicyRuleConfig>,
    #[serde(default)]
    dns: HashMap<String, PolicyRuleConfig>,
    #[serde(default)]
    model: HashMap<String, PolicyRuleConfig>,
    #[serde(default)]
    hook: HashMap<String, PolicyRuleConfig>,
}

impl<'de> Deserialize<'de> for PolicyConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPolicyConfig::deserialize(deserializer)?;
        let config = Self {
            mcp: raw.mcp,
            http: raw.http,
            dns: raw.dns,
            model: raw.model,
            hook: raw.hook,
        };
        config.validate().map_err(serde::de::Error::custom)?;
        Ok(config)
    }
}

impl PolicyConfig {
    fn validate(&self) -> Result<(), String> {
        validate_policy_rule_map(PolicyRuleType::Mcp, &self.mcp)?;
        validate_policy_rule_map(PolicyRuleType::Http, &self.http)?;
        validate_policy_rule_map(PolicyRuleType::Dns, &self.dns)?;
        validate_policy_rule_map(PolicyRuleType::Model, &self.model)?;
        validate_policy_rule_map(PolicyRuleType::Hook, &self.hook)?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.mcp.is_empty()
            && self.http.is_empty()
            && self.dns.is_empty()
            && self.model.is_empty()
            && self.hook.is_empty()
    }

    pub fn rules_for_callback(&self, callback: PolicyCallback) -> Vec<(&str, &PolicyRuleConfig)> {
        let mut rules: Vec<_> = self
            .rules(callback.policy_type())
            .iter()
            .filter(|(_, rule)| rule.on == callback)
            .map(|(name, rule)| (name.as_str(), rule))
            .collect();
        rules.sort_by(|(left_name, left), (right_name, right)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left_name.cmp(right_name))
        });
        rules
    }

    pub fn find_matching_rule<'a, S>(
        &'a self,
        callback: PolicyCallback,
        subject: &S,
    ) -> Result<Option<MatchedPolicyRule<'a>>, String>
    where
        S: PolicySubject + ?Sized,
    {
        for (name, rule) in self.rules_for_callback(callback) {
            if evaluate_policy_condition(callback, &rule.condition, subject)? {
                return Ok(Some(MatchedPolicyRule { name, rule }));
            }
        }
        Ok(None)
    }

    pub fn contains_rule_key(&self, key: &str) -> Result<bool, String> {
        let (rule_type, rule_name) = parse_policy_rule_key(key)?;
        Ok(self.rules(rule_type).contains_key(&rule_name))
    }

    pub fn upsert_rule_key(&mut self, key: &str, rule: PolicyRuleConfig) -> Result<(), String> {
        let (rule_type, rule_name) = parse_policy_rule_key(key)?;
        if rule.on.policy_type() != rule_type {
            return Err(format!(
                "policy rule '{key}' uses callback for a different policy type"
            ));
        }
        self.rules_mut(rule_type).insert(rule_name, rule);
        Ok(())
    }

    pub fn remove_rule_key(&mut self, key: &str) -> Result<(), String> {
        let (rule_type, rule_name) = parse_policy_rule_key(key)?;
        self.rules_mut(rule_type).remove(&rule_name);
        Ok(())
    }

    pub fn merge_first_wins(&mut self, next: PolicyConfig) {
        merge_rule_map_first_wins(&mut self.mcp, next.mcp);
        merge_rule_map_first_wins(&mut self.http, next.http);
        merge_rule_map_first_wins(&mut self.dns, next.dns);
        merge_rule_map_first_wins(&mut self.model, next.model);
        merge_rule_map_first_wins(&mut self.hook, next.hook);
    }

    pub fn merged(user: &PolicyConfig, corp: &PolicyConfig) -> PolicyConfig {
        let mut merged = user.clone();
        merge_rule_map_override(&mut merged.mcp, &corp.mcp);
        merge_rule_map_override(&mut merged.http, &corp.http);
        merge_rule_map_override(&mut merged.dns, &corp.dns);
        merge_rule_map_override(&mut merged.model, &corp.model);
        merge_rule_map_override(&mut merged.hook, &corp.hook);
        merged
    }

    fn rules(&self, rule_type: PolicyRuleType) -> &HashMap<String, PolicyRuleConfig> {
        match rule_type {
            PolicyRuleType::Mcp => &self.mcp,
            PolicyRuleType::Http => &self.http,
            PolicyRuleType::Dns => &self.dns,
            PolicyRuleType::Model => &self.model,
            PolicyRuleType::Hook => &self.hook,
        }
    }

    fn rules_mut(&mut self, rule_type: PolicyRuleType) -> &mut HashMap<String, PolicyRuleConfig> {
        match rule_type {
            PolicyRuleType::Mcp => &mut self.mcp,
            PolicyRuleType::Http => &mut self.http,
            PolicyRuleType::Dns => &mut self.dns,
            PolicyRuleType::Model => &mut self.model,
            PolicyRuleType::Hook => &mut self.hook,
        }
    }
}

fn validate_policy_rule_map(
    rule_type: PolicyRuleType,
    rules: &HashMap<String, PolicyRuleConfig>,
) -> Result<(), String> {
    for (name, rule) in rules {
        if !is_valid_policy_rule_name(name) {
            return Err(format!("invalid policy rule name: {name}"));
        }
        if rule.on.policy_type() != rule_type {
            return Err(format!(
                "policy rule '{name}' uses callback for a different policy type"
            ));
        }
    }
    Ok(())
}

fn merge_rule_map_first_wins(
    base: &mut HashMap<String, PolicyRuleConfig>,
    next: HashMap<String, PolicyRuleConfig>,
) {
    for (name, rule) in next {
        base.entry(name).or_insert(rule);
    }
}

fn merge_rule_map_override(
    base: &mut HashMap<String, PolicyRuleConfig>,
    overrides: &HashMap<String, PolicyRuleConfig>,
) {
    for (name, rule) in overrides {
        base.insert(name.clone(), rule.clone());
    }
}

pub fn parse_policy_rule_key(key: &str) -> Result<(PolicyRuleType, String), String> {
    let mut parts = key.split('.');
    let prefix = parts.next();
    let rule_type = parts.next();
    let rule_name = parts.next();
    if prefix != Some("policy")
        || rule_type.is_none()
        || rule_name.is_none()
        || parts.next().is_some()
    {
        return Err(format!(
            "policy rule key must be policy.<type>.<rule_name>: {key}"
        ));
    }
    let rule_type = PolicyRuleType::parse(rule_type.unwrap_or_default())
        .ok_or_else(|| format!("unknown policy type in key: {key}"))?;
    let rule_name = rule_name.unwrap_or_default();
    if !is_valid_policy_rule_name(rule_name) {
        return Err(format!("invalid policy rule name in key: {key}"));
    }
    Ok((rule_type, rule_name.to_string()))
}

pub fn is_policy_rule_key(key: &str) -> bool {
    key.starts_with("policy.")
}

fn is_valid_policy_rule_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

/// TOML file format for settings files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub settings: HashMap<String, SettingEntry>,
    /// Policy V2 named rules (`[policy.<type>.<rule_name>]`).
    #[serde(default, skip_serializing_if = "PolicyConfig::is_empty")]
    pub policy: PolicyConfig,
    /// MCP server configuration (optional section in user.toml / corp.toml).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::policy::McpUserConfig>,
}

/// Where a setting's effective value came from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicySource {
    #[default]
    Default,
    User,
    Corp,
}

/// A single value change record for audit trail.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub value: serde_json::Value,
    pub source: PolicySource,
}

/// A fully resolved setting (for UI consumption).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolvedSetting {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    pub effective_value: SettingValue,
    pub source: PolicySource,
    pub modified: Option<String>,
    pub corp_locked: bool,
    pub enabled_by: Option<String>,
    /// Computed: is the parent toggle on? (true if no parent).
    pub enabled: bool,
    pub metadata: SettingMetadata,
    /// Whether this setting starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
    /// Value change history (audit trail).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HistoryEntry>,
}

// ---------------------------------------------------------------------------
// MCP server definitions
// ---------------------------------------------------------------------------

pub fn default_true() -> bool {
    true
}

/// A declarative MCP server definition from defaults.toml, user.toml, or corp.toml.
///
/// MCP servers are auto-injected into AI agent config files (Claude, Gemini, Codex)
/// at boot time. Enterprises can add servers via corp.toml.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct McpServerDef {
    /// TOML key (e.g. "capsem", "internal_tools").
    #[serde(default)]
    pub key: String,
    /// Display name.
    pub name: String,
    /// Help text.
    #[serde(default)]
    pub description: Option<String>,
    /// Transport protocol.
    pub transport: McpTransport,
    /// Command to run (required for stdio transport).
    #[serde(default)]
    pub command: Option<String>,
    /// URL to connect to (required for sse transport).
    #[serde(default)]
    pub url: Option<String>,
    /// Command-line arguments (stdio only).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTP headers (sse only).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Non-removable by user (built-in servers).
    #[serde(default)]
    pub builtin: bool,
    /// Explicit enable/disable.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Where this definition came from.
    #[serde(default)]
    pub source: PolicySource,
    /// Whether corp.toml defines this server (user cannot modify).
    #[serde(default)]
    pub corp_locked: bool,
}

// ---------------------------------------------------------------------------
// Unified settings response
// ---------------------------------------------------------------------------

/// Unified response returned by `load_settings` and `save_settings` commands.
/// Bundles everything the frontend needs in a single IPC call.
#[derive(Serialize, Debug, Clone)]
pub struct SettingsResponse {
    pub tree: Vec<crate::net::policy_config::tree::SettingsNode>,
    pub issues: Vec<crate::net::policy_config::lint::ConfigIssue>,
    pub presets: Vec<crate::net::policy_config::presets::SecurityPreset>,
    pub policy: PolicyConfig,
}

// ---------------------------------------------------------------------------
// Guest config and VM settings
// ---------------------------------------------------------------------------

/// VM resource settings (extracted from settings).
#[derive(Debug, Default, Clone)]
pub struct VmSettings {
    pub cpu_count: Option<u32>,
    pub scratch_disk_size_gb: Option<u32>,
    pub ram_gb: Option<u32>,
    pub max_concurrent_vms: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("k".into(), "v".into());
        m
    }

    #[test]
    fn setting_value_as_bool_returns_value_only_for_bool_variant() {
        assert_eq!(SettingValue::Bool(true).as_bool(), Some(true));
        assert_eq!(SettingValue::Bool(false).as_bool(), Some(false));
        assert_eq!(SettingValue::Number(1).as_bool(), None);
        assert_eq!(SettingValue::Text("x".into()).as_bool(), None);
    }

    #[test]
    fn setting_value_as_number_returns_value_only_for_number_variant() {
        assert_eq!(SettingValue::Number(42).as_number(), Some(42));
        assert_eq!(SettingValue::Float(1.0).as_number(), None);
        assert_eq!(SettingValue::Text("42".into()).as_number(), None);
    }

    #[test]
    fn setting_value_as_text_returns_borrowed_str() {
        assert_eq!(SettingValue::Text("hi".into()).as_text(), Some("hi"));
        assert_eq!(SettingValue::Bool(true).as_text(), None);
    }

    #[test]
    fn setting_value_as_file_returns_tuple() {
        let v = SettingValue::File {
            path: "/tmp/x".into(),
            content: "body".into(),
        };
        assert_eq!(v.as_file(), Some(("/tmp/x", "body")));
        assert_eq!(SettingValue::Bool(true).as_file(), None);
    }

    #[test]
    fn setting_value_as_float_accepts_number_and_float() {
        assert_eq!(SettingValue::Float(1.5).as_float(), Some(1.5));
        // Number -> float coercion.
        assert_eq!(SettingValue::Number(3).as_float(), Some(3.0));
        assert_eq!(SettingValue::Text("1.5".into()).as_float(), None);
    }

    #[test]
    fn setting_value_list_accessors_return_slices() {
        let s = SettingValue::StringList(vec!["a".into(), "b".into()]);
        assert_eq!(
            s.as_string_list(),
            Some(&["a".to_string(), "b".to_string()][..])
        );
        assert_eq!(s.as_int_list(), None);
        assert_eq!(s.as_float_list(), None);

        let i = SettingValue::IntList(vec![1, 2]);
        assert_eq!(i.as_int_list(), Some(&[1i64, 2][..]));
        assert_eq!(i.as_string_list(), None);

        let f = SettingValue::FloatList(vec![1.0, 2.5]);
        assert_eq!(f.as_float_list(), Some(&[1.0f64, 2.5][..]));
        assert_eq!(f.as_int_list(), None);
    }

    #[test]
    fn setting_value_as_kv_map_returns_map() {
        let m = make_map();
        let v = SettingValue::KvMap(m.clone());
        assert_eq!(v.as_kv_map(), Some(&m));
        assert_eq!(SettingValue::Bool(true).as_kv_map(), None);
    }

    #[test]
    fn setting_value_deserializes_file_before_text() {
        // File variant must win over Text when input is a table.
        let toml = r#"path = "/etc/x"
content = "hello""#;
        let v: SettingValue = toml::from_str(toml).unwrap();
        match v {
            SettingValue::File { path, content } => {
                assert_eq!(path, "/etc/x");
                assert_eq!(content, "hello");
            }
            other => panic!("expected File variant, got {other:?}"),
        }
    }

    #[test]
    fn setting_value_deserializes_string_list_before_text() {
        let v: SettingValue = toml::from_str("value = [\"a\", \"b\"]")
            .and_then(|t: toml::Value| toml::Value::try_into(t["value"].clone()))
            .unwrap();
        match v {
            SettingValue::StringList(list) => assert_eq!(list, vec!["a", "b"]),
            other => panic!("expected StringList, got {other:?}"),
        }
    }

    #[test]
    fn default_true_helper_returns_true() {
        assert!(default_true());
    }

    #[test]
    fn policy_source_default_is_default_variant() {
        assert_eq!(PolicySource::default(), PolicySource::Default);
    }

    #[test]
    fn http_method_permissions_default_all_off() {
        let p = HttpMethodPermissions::default();
        assert!(!p.get && !p.post && !p.put && !p.delete && !p.other);
        assert!(p.domains.is_empty());
        assert!(p.path.is_none());
    }

    #[test]
    fn settings_file_default_has_empty_settings_and_no_mcp() {
        let f = SettingsFile::default();
        assert!(f.settings.is_empty());
        assert!(f.mcp.is_none());
    }

    #[test]
    fn setting_value_round_trips_through_json() {
        let cases = vec![
            SettingValue::Bool(true),
            SettingValue::Number(7),
            SettingValue::Float(2.5),
            SettingValue::Text("hello".into()),
            SettingValue::StringList(vec!["a".into()]),
            SettingValue::IntList(vec![1, 2, 3]),
            SettingValue::FloatList(vec![1.0, 2.0]),
            SettingValue::KvMap(make_map()),
            SettingValue::File {
                path: "/x".into(),
                content: "y".into(),
            },
        ];
        for v in cases {
            let j = serde_json::to_string(&v).unwrap();
            let back: SettingValue = serde_json::from_str(&j).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn enum_variants_serialize_with_snake_case() {
        assert_eq!(
            serde_json::to_string(&SettingType::ApiKey).unwrap(),
            "\"apikey\""
        );
        assert_eq!(
            serde_json::to_string(&SettingType::KvMap).unwrap(),
            "\"kv_map\""
        );
        assert_eq!(
            serde_json::to_string(&Widget::PasswordInput).unwrap(),
            "\"password_input\""
        );
        assert_eq!(
            serde_json::to_string(&SideEffect::ToggleTheme).unwrap(),
            "\"toggle_theme\""
        );
        assert_eq!(
            serde_json::to_string(&ActionKind::CheckUpdate).unwrap(),
            "\"check_update\""
        );
        assert_eq!(
            serde_json::to_string(&McpTransport::Stdio).unwrap(),
            "\"stdio\""
        );
        assert_eq!(
            serde_json::to_string(&McpToolOrigin::InVm).unwrap(),
            "\"in_vm\""
        );
        assert_eq!(
            serde_json::to_string(&PolicySource::Corp).unwrap(),
            "\"corp\""
        );
    }
}
