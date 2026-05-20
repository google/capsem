use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::condition::{evaluate_policy_condition, validate_policy_condition};

const DEFAULT_POLICY_RULE_PRIORITY: i32 = 1000;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyCallback {
    #[serde(rename = "mcp.request")]
    McpRequest,
    #[serde(rename = "mcp.response")]
    McpResponse,
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.read")]
    HttpRead,
    #[serde(rename = "http.write")]
    HttpWrite,
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
            PolicyCallback::HttpRequest
            | PolicyCallback::HttpRead
            | PolicyCallback::HttpWrite
            | PolicyCallback::HttpResponse => PolicyRuleType::Http,
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

/// One named `policy.<type>.<rule_name>` rule from a Profile policy document.
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

/// All configured named Policy rules.
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
struct PolicyConfigDocument {
    policy: PolicyConfig,
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
    pub fn from_policy_toml_str(input: &str) -> Result<Self, String> {
        toml::from_str::<PolicyConfigDocument>(input)
            .map(|document| document.policy)
            .map_err(|error| error.to_string())
    }

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
