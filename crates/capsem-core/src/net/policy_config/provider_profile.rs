use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::net::ai_traffic::provider::ModelProtocol;

use super::{
    CompiledSecurityRule, ProviderDiscovery, SecurityRuleProfile, SecurityRuleProvider,
    SecurityRuleSet, SecurityRuleSource,
};

const DEFAULT_PROVIDER_RULES_TOML: &str = include_str!("default_provider_rules.toml");
const REQUIRED_BUILTIN_PLUGINS: &[&str] = &["credential_broker"];
const REQUIRED_DEFAULT_RULE_KEYS: &[&str] = &[
    "default_http_requests",
    "default_dns_queries",
    "default_mcp_activity",
    "default_model_calls",
    "default_file_activity",
    "default_process_activity",
    "default_credentials",
    "default_snapshots",
];

pub type AiProviderProfile = SecurityRuleProvider;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderDiscoveryPatch {
    pub provider_id: String,
    pub discovery: ProviderDiscovery,
}

impl ProviderDiscoveryPatch {
    pub fn for_builtin_provider(
        provider_id: impl Into<String>,
        discovery: ProviderDiscovery,
    ) -> Result<Self, String> {
        let provider_id = provider_id.into();
        if !ProviderRuleProfile::builtin_defaults()
            .ai
            .contains_key(&provider_id)
        {
            return Err(format!(
                "provider discovery only supports configured provider '{provider_id}'"
            ));
        }
        discovery.validate(&format!("ai.{provider_id}.discovery"))?;
        Ok(Self {
            provider_id,
            discovery,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEndpoint {
    pub provider_id: String,
    pub display_name: String,
    pub protocol: ModelProtocol,
    pub upstream_url: String,
    pub aliases: Vec<String>,
    pub listen_ports: Vec<u16>,
    pub credential_setting_id: Option<String>,
    pub credential_ref: Option<String>,
    pub allowed_remote_targets: Vec<String>,
    pub files: Vec<String>,
}

impl ModelEndpoint {
    pub fn matches_host(&self, host: &str) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        self.hosts()
            .into_iter()
            .any(|candidate| candidate.as_deref() == Some(host.as_str()))
    }

    pub fn matches_target(&self, host: &str, port: u16) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        self.target_specs().into_iter().any(|target| {
            target
                .host
                .as_deref()
                .is_some_and(|candidate| candidate == host.as_str())
                && target.port.is_none_or(|target_port| target_port == port)
        })
    }

    fn hosts(&self) -> Vec<Option<String>> {
        std::iter::once(upstream_target(&self.upstream_url).and_then(|target| target.host))
            .chain(self.aliases.iter().map(|alias| normalize_host(alias)))
            .chain(
                self.allowed_remote_targets
                    .iter()
                    .map(|target| upstream_target(target).and_then(|target| target.host)),
            )
            .collect()
    }

    fn target_specs(&self) -> Vec<TargetSpec> {
        let upstream = upstream_target(&self.upstream_url).unwrap_or_default();
        let alias_targets = self.aliases.iter().flat_map(|alias| {
            let host = normalize_host(alias);
            if self.listen_ports.is_empty() {
                vec![TargetSpec { host, port: None }]
            } else {
                self.listen_ports
                    .iter()
                    .map(|port| TargetSpec {
                        host: host.clone(),
                        port: Some(*port),
                    })
                    .collect::<Vec<_>>()
            }
        });
        std::iter::once(upstream)
            .chain(
                self.allowed_remote_targets
                    .iter()
                    .filter_map(|target| upstream_target(target)),
            )
            .chain(alias_targets)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelEndpointRegistry {
    endpoints: BTreeMap<String, ModelEndpoint>,
}

impl ModelEndpointRegistry {
    pub fn from_provider_profile(profile: &ProviderRuleProfile) -> Result<Self, String> {
        profile.validate()?;
        let mut endpoints = BTreeMap::new();
        for (provider_id, provider) in &profile.ai {
            let protocol = provider
                .protocol
                .as_deref()
                .ok_or_else(|| format!("ai.{provider_id}.protocol is required"))?;
            let url = provider
                .url
                .as_deref()
                .ok_or_else(|| format!("ai.{provider_id}.url is required"))?;
            endpoints.insert(
                provider_id.clone(),
                ModelEndpoint {
                    provider_id: provider_id.clone(),
                    display_name: provider.name.clone().unwrap_or_else(|| provider_id.clone()),
                    protocol: ModelProtocol::try_from(protocol)?,
                    upstream_url: url.to_string(),
                    aliases: provider.aliases.clone(),
                    listen_ports: provider.listen_ports.clone(),
                    credential_setting_id: provider.credential_setting_id.clone(),
                    credential_ref: provider.credential_ref.clone(),
                    allowed_remote_targets: provider.allowed_remote_targets.clone(),
                    files: provider.files.clone(),
                },
            );
        }
        Ok(Self { endpoints })
    }

    pub fn get(&self, provider_id: &str) -> Option<&ModelEndpoint> {
        self.endpoints.get(provider_id)
    }

    pub fn endpoint_for_host(&self, host: &str) -> Option<&ModelEndpoint> {
        self.endpoints
            .values()
            .find(|endpoint| endpoint.matches_host(host))
    }

    pub fn endpoint_for_target(&self, host: &str, port: u16) -> Option<&ModelEndpoint> {
        self.endpoints
            .values()
            .find(|endpoint| endpoint.matches_target(host, port))
    }

    pub fn protocol_for_host(&self, host: &str) -> Option<ModelProtocol> {
        self.endpoint_for_host(host)
            .map(|endpoint| endpoint.protocol)
    }

    pub fn protocol_for_target(&self, host: &str, port: u16) -> Option<ModelProtocol> {
        self.endpoint_for_target(host, port)
            .map(|endpoint| endpoint.protocol)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ModelEndpoint> {
        self.endpoints.values()
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }
}

fn normalize_host(host: &str) -> Option<String> {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() || normalized.starts_with('[') {
        None
    } else {
        Some(normalized)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TargetSpec {
    host: Option<String>,
    port: Option<u16>,
}

fn upstream_target(url: &str) -> Option<TargetSpec> {
    let (scheme, rest) = url
        .split_once("://")
        .map_or((None, url), |(scheme, rest)| (Some(scheme), rest));
    let default_port = match scheme {
        Some("http") => Some(80),
        Some("https") => Some(443),
        _ => None,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.trim().is_empty() {
        return None;
    }
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = parse_host_port(host_port, default_port);
    Some(TargetSpec { host, port })
}

fn parse_host_port(host_port: &str, default_port: Option<u16>) -> (Option<String>, Option<u16>) {
    let (host, explicit_port) = host_port
        .rsplit_once(':')
        .map_or((host_port, None), |(host, port)| {
            (host, port.parse::<u16>().ok())
        });
    (normalize_host(host), explicit_port.or(default_port))
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRuleProfile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, AiProviderProfile>,
}

impl ProviderRuleProfile {
    pub fn builtin_security_defaults() -> SecurityRuleProfile {
        let profile = SecurityRuleProfile::parse_toml(DEFAULT_PROVIDER_RULES_TOML)
            .expect("built-in provider rule profile must parse");
        validate_builtin_default_contract(&profile)
            .expect("built-in provider rule profile must include default rules and plugins");
        profile
    }

    pub fn builtin_defaults() -> Self {
        let profile = Self::builtin_security_defaults();
        Self { ai: profile.ai }
    }

    pub fn parse_toml(input: &str) -> Result<Self, String> {
        let profile = SecurityRuleProfile::parse_toml(input)?;
        Ok(Self { ai: profile.ai })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.as_security_rule_profile().validate()
    }

    pub fn compile(&self, source: SecurityRuleSource) -> Result<Vec<CompiledSecurityRule>, String> {
        self.as_security_rule_profile().compile(source)
    }

    pub fn compile_rule_set(&self, source: SecurityRuleSource) -> Result<SecurityRuleSet, String> {
        SecurityRuleSet::compile_profile(&self.as_security_rule_profile(), source)
    }

    pub fn endpoint_registry(&self) -> Result<ModelEndpointRegistry, String> {
        ModelEndpointRegistry::from_provider_profile(self)
    }

    pub fn merge_override(base: &Self, overrides: &Self) -> Result<Self, String> {
        base.validate()?;
        overrides.validate()?;

        let mut merged = base.clone();
        for (provider_id, override_provider) in &overrides.ai {
            match merged.ai.get_mut(provider_id) {
                Some(base_provider) => {
                    if override_provider.name.is_some() {
                        base_provider.name = override_provider.name.clone();
                    }
                    if override_provider.protocol.is_some() {
                        base_provider.protocol = override_provider.protocol.clone();
                    }
                    if override_provider.url.is_some() {
                        base_provider.url = override_provider.url.clone();
                    }
                    if !override_provider.aliases.is_empty() {
                        base_provider.aliases = override_provider.aliases.clone();
                    }
                    if !override_provider.listen_ports.is_empty() {
                        base_provider.listen_ports = override_provider.listen_ports.clone();
                    }
                    if override_provider.credential_setting_id.is_some() {
                        base_provider.credential_setting_id =
                            override_provider.credential_setting_id.clone();
                    }
                    if override_provider.credential_ref.is_some() {
                        base_provider.credential_ref = override_provider.credential_ref.clone();
                    }
                    if !override_provider.allowed_remote_targets.is_empty() {
                        base_provider.allowed_remote_targets =
                            override_provider.allowed_remote_targets.clone();
                    }
                    if !override_provider.files.is_empty() {
                        base_provider.files = override_provider.files.clone();
                    }
                    if override_provider.discovery.is_some() {
                        base_provider.discovery = override_provider.discovery.clone();
                    }
                    for (rule_name, override_rule) in &override_provider.rules {
                        base_provider
                            .rules
                            .insert(rule_name.clone(), override_rule.clone());
                    }
                }
                None => {
                    merged
                        .ai
                        .insert(provider_id.clone(), override_provider.clone());
                }
            }
        }
        merged.validate()?;
        Ok(merged)
    }

    pub fn merge_user_and_corp(user: &Self, corp: &Self) -> Result<Self, String> {
        Self::merge_override(user, corp)
    }

    pub fn merge_defaults_user_and_corp(user: &Self, corp: &Self) -> Result<Self, String> {
        let defaults = Self::builtin_defaults();
        let with_user = Self::merge_override(&defaults, user)?;
        Self::merge_override(&with_user, corp)
    }

    fn as_security_rule_profile(&self) -> SecurityRuleProfile {
        SecurityRuleProfile {
            ai: self.ai.clone(),
            ..SecurityRuleProfile::default()
        }
    }
}

fn validate_builtin_default_contract(profile: &SecurityRuleProfile) -> Result<(), String> {
    for plugin_id in REQUIRED_BUILTIN_PLUGINS {
        if !profile.plugins.contains_key(*plugin_id) {
            return Err(format!(
                "built-in default profile must include [plugins.{plugin_id}]"
            ));
        }
    }
    for rule_key in REQUIRED_DEFAULT_RULE_KEYS {
        if !profile.profiles.defaults.contains_key(*rule_key) {
            return Err(format!(
                "built-in default profile must include [profiles.defaults.{rule_key}]"
            ));
        }
    }
    Ok(())
}

pub fn compile_provider_rules_to_security_rule_set(
    user: &ProviderRuleProfile,
    corp: &ProviderRuleProfile,
) -> Result<SecurityRuleSet, String> {
    let mut by_rule_id = BTreeMap::new();
    for rule in ProviderRuleProfile::builtin_security_defaults()
        .compile(SecurityRuleSource::BuiltinDefault)?
    {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    for rule in user.compile(SecurityRuleSource::User)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    for rule in corp.compile(SecurityRuleSource::Corp)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    Ok(SecurityRuleSet::new(by_rule_id.into_values().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::policy_config::{DetectionLevel, SecurityRuleAction};

    const DRAFT: &str = include_str!("default_provider_rules.toml");

    #[test]
    fn parses_real_provider_defaults_as_security_rules() {
        let profile = ProviderRuleProfile::parse_toml(DRAFT).expect("draft parses");
        assert_eq!(
            profile.ai.keys().cloned().collect::<Vec<_>>(),
            vec!["anthropic", "google", "ollama", "openai"]
        );
        let compiled = profile
            .compile(SecurityRuleSource::BuiltinDefault)
            .expect("draft compiles");
        assert!(compiled
            .iter()
            .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
        assert!(compiled.iter().any(|rule| {
            rule.provider == "google"
                && rule.rule_key == "config_credential_broker"
                && rule.plugin.as_deref() == Some("credential_broker")
        }));
        assert!(compiled
            .iter()
            .all(|rule| !rule.condition.contains("file.ingress")));
        assert!(compiled
            .iter()
            .all(|rule| !rule.condition.contains("credential.name")));
    }

    #[test]
    fn builtin_default_contract_requires_plugins_and_visible_default_rules() {
        let missing_plugins = SecurityRuleProfile::parse_toml(
            r#"
[profiles.defaults.default_http_requests]
name = "default_http_requests"
action = "allow"
priority = "default"
reason = "Default allow for HTTP requests."
match = 'has(http.host)'
"#,
        )
        .expect("profile without plugins parses before built-in contract");
        let err = validate_builtin_default_contract(&missing_plugins)
            .expect_err("built-in default profile requires plugin section");
        assert!(err.contains("[plugins.credential_broker]"), "{err}");

        let missing_defaults = SecurityRuleProfile::parse_toml(
            r#"
[plugins.credential_broker]
mode = "rewrite"

[profiles.rules.broker]
name = "broker"
action = "postprocess"
plugin = "credential_broker"
match = 'has(http.host)'
"#,
        )
        .expect("profile without defaults parses before built-in contract");
        let err = validate_builtin_default_contract(&missing_defaults)
            .expect_err("built-in default profile requires visible defaults");
        assert!(
            err.contains("[profiles.defaults.default_http_requests]"),
            "{err}"
        );
    }

    #[test]
    fn provider_defaults_build_settings_defined_endpoint_registry() {
        let registry = ProviderRuleProfile::builtin_defaults()
            .endpoint_registry()
            .expect("registry builds");
        assert_eq!(registry.len(), 4);
        assert_eq!(
            registry.get("openai").expect("openai").protocol,
            ModelProtocol::OpenAi
        );
        assert_eq!(
            registry.get("anthropic").expect("anthropic").protocol,
            ModelProtocol::Anthropic
        );
        assert_eq!(
            registry.get("google").expect("google").protocol,
            ModelProtocol::Google
        );
        assert_eq!(
            registry.get("ollama").expect("ollama").protocol,
            ModelProtocol::Ollama
        );
        assert_eq!(
            registry.protocol_for_host("api.openai.com"),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(
            registry.protocol_for_host("GENERATIVELANGUAGE.GOOGLEAPIS.COM."),
            Some(ModelProtocol::Google)
        );
        assert_eq!(
            registry.protocol_for_host("127.0.0.1"),
            Some(ModelProtocol::Ollama)
        );
        assert_eq!(
            registry.protocol_for_host("local.ollama"),
            Some(ModelProtocol::Ollama)
        );
        assert_eq!(
            registry.protocol_for_target("local.ollama", 11434),
            Some(ModelProtocol::Ollama)
        );
        assert_eq!(registry.protocol_for_target("local.ollama", 80), None);
        assert_eq!(
            registry.protocol_for_target("api.openai.com", 443),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(registry.protocol_for_target("api.openai.com", 80), None);
        let openai = registry.get("openai").expect("openai endpoint");
        assert_eq!(openai.aliases, vec!["api.openai.com"]);
        assert_eq!(openai.listen_ports, vec![443]);
        assert_eq!(
            openai.credential_setting_id.as_deref(),
            Some("ai.openai.api_key")
        );
        assert!(openai.credential_ref.is_none());
        assert_eq!(openai.allowed_remote_targets, vec!["api.openai.com:443"]);
    }

    #[test]
    fn custom_openai_compatible_endpoint_schema_requires_no_protocol_enum_growth() {
        let profile = ProviderRuleProfile::parse_toml(
            r#"
[ai.private_gateway]
name = "Private Gateway"
protocol = "openai-compatible"
url = "https://llm.internal.example/v1"
aliases = ["company-openai", "llm.internal.example"]
listen_ports = [443, 8443]
credential_setting_id = "ai.private_gateway.api_key"
credential_ref = "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
allowed_remote_targets = ["llm.internal.example:443", "company-openai:8443"]
files = ["/root/.config/private-gateway/config.toml"]

[ai.private_gateway.rules.http_api]
name = "private_gateway_http_seen"
action = "allow"
match = 'http.host == "llm.internal.example"'
"#,
        )
        .expect("profile parses");

        let registry = profile.endpoint_registry().expect("registry builds");
        let endpoint = registry
            .get("private_gateway")
            .expect("private endpoint exists");
        assert_eq!(endpoint.provider_id, "private_gateway");
        assert_eq!(endpoint.display_name, "Private Gateway");
        assert_eq!(endpoint.protocol, ModelProtocol::OpenAi);
        assert_eq!(endpoint.upstream_url, "https://llm.internal.example/v1");
        assert_eq!(
            endpoint.credential_setting_id.as_deref(),
            Some("ai.private_gateway.api_key")
        );
        assert_eq!(
            endpoint.credential_ref.as_deref(),
            Some("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222")
        );
        assert_eq!(
            endpoint.files,
            vec!["/root/.config/private-gateway/config.toml"]
        );
        assert_eq!(
            registry.protocol_for_host("llm.internal.example"),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(
            registry.protocol_for_host("company-openai"),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(
            registry.protocol_for_target("company-openai", 8443),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(registry.protocol_for_target("company-openai", 11434), None);
    }

    #[test]
    fn provider_override_uses_same_rule_contract() {
        let user = ProviderRuleProfile::parse_toml(
            r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_user"
action = "ask"
match = 'http.host == "api.openai.com"'
"#,
        )
        .expect("user provider parses");
        let corp = ProviderRuleProfile::parse_toml(
            r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_corp_block"
action = "block"
detection_level = "critical"
priority = -100
match = 'http.host == "api.openai.com"'
"#,
        )
        .expect("corp provider parses");

        let merged = ProviderRuleProfile::merge_override(&user, &corp).expect("merge succeeds");
        let compiled = merged
            .compile(SecurityRuleSource::Corp)
            .expect("merged profile compiles");
        let rule = compiled
            .iter()
            .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
            .expect("merged rule exists");
        assert_eq!(rule.name, "openai_http_corp_block");
        assert_eq!(rule.action, SecurityRuleAction::Block);
        assert_eq!(rule.detection_level, Some(DetectionLevel::Critical));
        assert_eq!(rule.priority, -100);
    }

    #[test]
    fn provider_owned_rules_compile_to_security_event_rule_contract() {
        let profile = ProviderRuleProfile::parse_toml(
            r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.detect_http]
name = "openai_detect_http"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\.)openai\.com$")'

[ai.openai.rules.capture_credential]
name = "openai_capture_credential"
plugin = "credential_broker"
action = "postprocess"
type = "api-key"
credential = "api_key"
match = 'http.host.matches("(^|.*\.)openai\.com$")'

[ai.openai.rules.redact_prompt]
name = "openai_redact_prompt"
plugin = "pii"
action = "preprocess"
match = 'model.provider == "openai"'
"#,
        )
        .expect("provider rules parse");

        let rules = profile
            .compile_rule_set(SecurityRuleSource::User)
            .expect("provider rules compile");
        let ids = rules
            .rules()
            .iter()
            .map(|rule| {
                (
                    rule.rule_id.as_str(),
                    rule.action,
                    rule.detection_level,
                    rule.priority,
                    rule.plugin.as_deref(),
                )
            })
            .collect::<Vec<_>>();

        assert!(ids.contains(&(
            "profiles.rules.ai_openai_detect_http",
            SecurityRuleAction::Allow,
            Some(DetectionLevel::Informational),
            10,
            None
        )));
        assert!(ids.contains(&(
            "profiles.rules.ai_openai_capture_credential",
            SecurityRuleAction::Postprocess,
            None,
            10,
            Some("credential_broker")
        )));
        assert!(ids.contains(&(
            "profiles.rules.ai_openai_redact_prompt",
            SecurityRuleAction::Preprocess,
            None,
            10,
            Some("pii")
        )));
    }
}
