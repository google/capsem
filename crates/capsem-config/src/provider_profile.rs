use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{ModelProtocol, ProviderKind};

use crate::{
    CompiledSecurityRule, SecurityRuleProfile, SecurityRuleProvider, SecurityRuleSet,
    SecurityRuleSource,
};

const DEFAULT_PROVIDER_RULES_TOML: &str = include_str!("default_provider_rules.toml");
const REQUIRED_BUILTIN_PLUGINS: &[&str] = &["credential_broker", "log_sanitizer"];
const REQUIRED_DEFAULT_RULE_KEYS: &[&str] = &["http", "dns", "mcp", "model", "file", "process"];

pub type AiProviderProfile = SecurityRuleProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEndpoint {
    pub provider_id: String,
    pub provider_kind: ProviderKind,
    pub display_name: String,
    pub protocol: ModelProtocol,
    pub upstream_url: String,
    pub listen_ports: Vec<u16>,
    pub allowed_remote_targets: Vec<String>,
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
            .chain(
                self.allowed_remote_targets
                    .iter()
                    .map(|target| upstream_target(target).and_then(|target| target.host)),
            )
            .collect()
    }

    fn target_specs(&self) -> Vec<TargetSpec> {
        let upstream = upstream_target(&self.upstream_url).unwrap_or_default();
        std::iter::once(upstream)
            .chain(
                self.allowed_remote_targets
                    .iter()
                    .filter_map(|target| upstream_target(target)),
            )
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
                    provider_kind: ProviderKind::from_provider_id(provider_id),
                    display_name: provider.name.clone().unwrap_or_else(|| provider_id.clone()),
                    protocol: ModelProtocol::try_from(protocol)?,
                    upstream_url: url.to_string(),
                    listen_ports: provider.listen_ports.clone(),
                    allowed_remote_targets: provider.allowed_remote_targets.clone(),
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

    pub fn provider_for_host(&self, host: &str) -> Option<ProviderKind> {
        self.endpoint_for_host(host)
            .map(|endpoint| endpoint.provider_kind)
    }

    pub fn provider_for_target(&self, host: &str, port: u16) -> Option<ProviderKind> {
        self.endpoint_for_target(host, port)
            .map(|endpoint| endpoint.provider_kind)
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
        validate_builtin_profile_contract(&profile)
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
                    if !override_provider.listen_ports.is_empty() {
                        base_provider.listen_ports = override_provider.listen_ports.clone();
                    }
                    if !override_provider.allowed_remote_targets.is_empty() {
                        base_provider.allowed_remote_targets =
                            override_provider.allowed_remote_targets.clone();
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

fn validate_builtin_profile_contract(profile: &SecurityRuleProfile) -> Result<(), String> {
    for plugin_id in REQUIRED_BUILTIN_PLUGINS {
        if !profile.plugins.contains_key(*plugin_id) {
            return Err(format!(
                "built-in profile must include [plugins.{plugin_id}]"
            ));
        }
    }
    for rule_key in REQUIRED_DEFAULT_RULE_KEYS {
        if !profile.default.contains_key(*rule_key) {
            return Err(format!(
                "built-in profile must include visible default rule [default.{rule_key}]"
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
mod tests;
