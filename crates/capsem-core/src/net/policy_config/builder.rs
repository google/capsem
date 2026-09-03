use super::loader::load_settings_and_corp_files;
use super::provider_profile::{
    compile_provider_rules_to_security_rule_set, ModelEndpointRegistry, ProviderRuleProfile,
};
use super::resolver::resolve_settings;
use super::types::*;
use super::{SecurityPluginConfig, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Translation: settings -> policy objects
// ---------------------------------------------------------------------------

fn parse_http_upstream_ports(values: &[i64]) -> Vec<u16> {
    values.iter().filter_map(|port| u16::try_from(*port).ok()).collect()
}

/// Extract guest config from resolved settings.
///
/// Dynamic keys with prefix `guest.env.` become environment variables.
/// Brokered credentials and AI/tool config files are deliberately excluded:
/// profile/runtime plugin plumbing owns those paths, not settings.toml.
pub fn settings_to_guest_config(resolved: &[ResolvedSetting]) -> GuestConfig {
    use capsem_proto::{validate_env_key, validate_env_value, validate_file_path};

    let mut env = HashMap::new();
    let mut files = Vec::new();

    for s in resolved {
        let text_value = resolved_text_for_guest(s);

        // Metadata-driven env var injection for non-credential settings. Brokered
        // credential settings are opaque references and must never materialize
        // into the VM as raw API keys.
        if is_brokered_credential_setting_id(&s.id) {
            continue;
        }

        let env_text = match &s.effective_value {
            SettingValue::Text(_) => text_value.as_deref(),
            SettingValue::File { content, .. } => Some(content.as_str()),
            _ => None,
        };
        if let Some(ev) = env_text {
            if !s.metadata.env_vars.is_empty() && !ev.is_empty() {
                for var_name in &s.metadata.env_vars {
                    if let Err(e) = validate_env_key(var_name) {
                        tracing::warn!(error = %e, "skipping invalid env var from metadata");
                        continue;
                    }
                    if let Err(e) = validate_env_value(ev) {
                        tracing::warn!(error = %e, "skipping env var {var_name}: invalid value");
                        continue;
                    }
                    env.insert(var_name.clone(), ev.to_string());
                }
            }
        }

        // Boot files: non-AI File values with non-empty content. AI/tool config
        // belongs to profile/runtime plugin machinery, not settings.toml.
        if let SettingValue::File {
            path: file_path,
            content: file_content,
        } = &s.effective_value
        {
            if s.id.starts_with("ai.") {
                continue;
            }
            if !file_content.is_empty() {
                if let Err(e) = validate_file_path(file_path) {
                    tracing::warn!(error = %e, "skipping boot file");
                    continue;
                }

                files.push(GuestFile {
                    path: file_path.clone(),
                    content: file_content.clone(),
                    mode: 0o600,
                });
            }
        }

        // Dynamic guest.env.* settings (not in registry)
        if let Some(var_name) = s.id.strip_prefix("guest.env.") {
            if let Some(text_value) = text_value.as_deref().filter(|v| !v.is_empty()) {
                if let Err(e) = validate_env_key(var_name) {
                    tracing::warn!(error = %e, "skipping dynamic env var");
                    continue;
                }
                if let Err(e) = validate_env_value(text_value) {
                    tracing::warn!(error = %e, "skipping dynamic env var {var_name}: invalid value");
                    continue;
                }
                env.insert(var_name.to_string(), text_value.to_string());
            }
        }
    }

    // SSH public key: write to /root/.ssh/authorized_keys if set.
    let ssh_key = resolved
        .iter()
        .find(|s| s.id == SETTING_SSH_PUBLIC_KEY)
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    if !ssh_key.is_empty() {
        files.push(GuestFile {
            path: "/root/.ssh/authorized_keys".to_string(),
            content: ssh_key.to_string() + "\n",
            mode: 0o600,
        });
    }

    GuestConfig {
        env: if env.is_empty() { None } else { Some(env) },
        files: if files.is_empty() { None } else { Some(files) },
    }
}

fn resolved_text_for_guest(s: &ResolvedSetting) -> Option<String> {
    let text = s.effective_value.as_text()?;
    Some(text.to_string())
}

/// Extract VM settings from resolved settings.
pub fn settings_to_vm_settings(resolved: &[ResolvedSetting]) -> VmSettings {
    let cpu_count = resolved
        .iter()
        .find(|s| s.id == "vm.resources.cpu_count")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    let scratch_disk_size_gb = resolved
        .iter()
        .find(|s| s.id == "vm.resources.scratch_disk_size_gb")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    let ram_gb = resolved
        .iter()
        .find(|s| s.id == "vm.resources.ram_gb")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    let max_concurrent_vms = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_concurrent_vms")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    VmSettings {
        cpu_count: Some(cpu_count.unwrap_or(4)),
        scratch_disk_size_gb: Some(scratch_disk_size_gb.unwrap_or(16)),
        ram_gb: Some(ram_gb.unwrap_or(4)),
        max_concurrent_vms: Some(max_concurrent_vms.unwrap_or(10)),
    }
}

// ---------------------------------------------------------------------------
// High-level entry points
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// MergedPolicies: single struct owning all merged policies
// ---------------------------------------------------------------------------

/// All merged policies from user + corp settings.
///
/// Built via `from_files()` (pure, hermetic) or `from_disk()` (loads from
/// standard paths). Every policy type is derived from a single
/// `resolve_settings()` call, ensuring consistency.
pub struct MergedPolicies {
    pub network: crate::net::policy::NetworkMechanics,
    pub security_rules: SecurityRuleSet,
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
    pub model_endpoints: ModelEndpointRegistry,
    pub guest: GuestConfig,
    pub vm: VmSettings,
}

impl MergedPolicies {
    /// Pure merge function. No I/O, fully testable.
    ///
    /// Fails when the security rules or the model endpoint registry do not
    /// compile. The engine allows any event no rule matches, so an empty rule
    /// set is allow-everything; it used to be substituted for a broken one
    /// behind a warning, and every VM on that profile ran without policy.
    pub fn from_files(user: &SettingsFile, corp: &SettingsFile) -> Result<Self, String> {
        let resolved = resolve_settings(user, corp);
        let security_rules = compile_merged_security_rules(user, corp)?;
        let model_endpoints = compile_model_endpoint_registry(user, corp)?;
        let plugins = merge_plugin_policy(user, corp);
        Ok(Self {
            network: build_network_policy(&resolved),
            security_rules,
            plugins,
            model_endpoints,
            guest: settings_to_guest_config(&resolved),
            vm: settings_to_vm_settings(&resolved),
        })
    }

    /// Load from disk then merge. Falls back to built-in defaults when the
    /// files cannot be read or their rules do not compile. The callers here
    /// read network, guest, and VM settings only, never the rule set, so the
    /// fallback cannot widen policy; the rule set reaches a VM only through
    /// `ActiveProfileFile::compile_security_rule_set`, which fails closed.
    pub fn from_disk() -> Self {
        let (user, corp) = load_settings_and_corp_files();
        Self::from_files(&user, &corp).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "settings do not compile; using built-in defaults for runtime settings");
            Self::from_files(&SettingsFile::default(), &SettingsFile::default())
                .expect("built-in default settings compile")
        })
    }
}

fn merge_plugin_policy(user: &SettingsFile, corp: &SettingsFile) -> BTreeMap<String, SecurityPluginConfig> {
    let mut plugins = ProviderRuleProfile::builtin_security_defaults().plugins;
    for (plugin_id, mode) in &user.plugins {
        plugins.insert(plugin_id.clone(), *mode);
    }
    for (plugin_id, mode) in &corp.plugins {
        plugins.insert(plugin_id.clone(), *mode);
    }
    plugins
}

fn compile_model_endpoint_registry(user: &SettingsFile, corp: &SettingsFile) -> Result<ModelEndpointRegistry, String> {
    let merged = ProviderRuleProfile::merge_defaults_user_and_corp(
        &ProviderRuleProfile { ai: user.ai.clone() },
        &ProviderRuleProfile { ai: corp.ai.clone() },
    )?;
    merged.endpoint_registry()
}

fn compile_merged_security_rules(user: &SettingsFile, corp: &SettingsFile) -> Result<SecurityRuleSet, String> {
    let mut by_rule_id = std::collections::BTreeMap::new();
    let provider_rules = compile_provider_rules_to_security_rule_set(
        &ProviderRuleProfile { ai: user.ai.clone() },
        &ProviderRuleProfile { ai: corp.ai.clone() },
    )?;
    for rule in provider_rules.rules() {
        by_rule_id.insert(rule.rule_id.clone(), rule.clone());
    }
    let user_profile = SecurityRuleProfile {
        default: user.default.clone(),
        profiles: user.profiles.clone(),
        ..SecurityRuleProfile::default()
    };
    for rule in user_profile.compile(SecurityRuleSource::User)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    let corp_profile = SecurityRuleProfile {
        default: corp.default.clone(),
        corp: corp.corp.clone(),
        profiles: corp.profiles.clone(),
        ..SecurityRuleProfile::default()
    };
    for rule in corp_profile.compile(SecurityRuleSource::Corp)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    Ok(SecurityRuleSet::new(by_rule_id.into_values().collect()))
}

/// Build network mechanics from resolved settings (pure, no I/O).
///
/// Security allow/block/default behavior compiles into `SecurityRuleSet`.
/// This builder carries only non-decision mechanics used by the network engine.
pub fn build_network_policy(resolved: &[ResolvedSetting]) -> crate::net::policy::NetworkMechanics {
    use crate::net::policy::NetworkMechanics;

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(true);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    let mut mechanics = NetworkMechanics::new();
    if let Some(ports) = resolved
        .iter()
        .find(|s| s.id == "security.web.http_upstream_ports")
        .and_then(|s| s.effective_value.as_int_list())
    {
        mechanics.http_upstream_ports = parse_http_upstream_ports(ports);
    }
    mechanics.log_bodies = log_bodies;
    mechanics.max_body_capture = max_body_capture;
    mechanics
}

pub fn network_config_from_policy_and_dns(
    mechanics: &crate::net::policy::NetworkMechanics,
    dns: DnsNetworkConfig,
) -> NetworkConfig {
    NetworkConfig {
        log_bodies: Some(mechanics.log_bodies),
        max_body_capture: Some(mechanics.max_body_capture),
        http_upstream_ports: mechanics.http_upstream_ports.clone(),
        dns,
        upstream_overrides: mechanics
            .upstream_overrides
            .iter()
            .map(|(target, route)| {
                (
                    target.clone(),
                    UpstreamOverrideConfig {
                        dial: route.dial.clone(),
                        protocol: match route.protocol {
                            crate::net::policy::UpstreamOverrideProtocol::Http => UpstreamOverrideProtocolConfig::Http,
                            crate::net::policy::UpstreamOverrideProtocol::Tls => UpstreamOverrideProtocolConfig::Tls,
                        },
                    },
                )
            })
            .collect(),
    }
}

pub fn apply_network_config(config: &NetworkConfig, mechanics: &mut crate::net::policy::NetworkMechanics) {
    if let Some(log_bodies) = config.log_bodies {
        mechanics.log_bodies = log_bodies;
    }
    if let Some(max_body_capture) = config.max_body_capture {
        mechanics.max_body_capture = max_body_capture;
    }
    if !config.http_upstream_ports.is_empty() {
        mechanics.http_upstream_ports = config.http_upstream_ports.clone();
    }
    if !config.upstream_overrides.is_empty() {
        mechanics.upstream_overrides = config
            .upstream_overrides
            .iter()
            .map(|(target, route)| {
                (
                    target.to_lowercase(),
                    crate::net::policy::UpstreamOverride {
                        dial: route.dial.clone(),
                        protocol: match route.protocol {
                            UpstreamOverrideProtocolConfig::Http => crate::net::policy::UpstreamOverrideProtocol::Http,
                            UpstreamOverrideProtocolConfig::Tls => crate::net::policy::UpstreamOverrideProtocol::Tls,
                        },
                    },
                )
            })
            .collect();
    }
}

// ---------------------------------------------------------------------------
// High-level entry points (thin wrappers over MergedPolicies)
// ---------------------------------------------------------------------------

/// Build network mechanics from merged settings.
pub fn load_merged_network_policy() -> crate::net::policy::NetworkMechanics {
    MergedPolicies::from_disk().network
}

/// Load and merge guest config from standard locations.
pub fn load_merged_guest_config() -> GuestConfig {
    MergedPolicies::from_disk().guest
}

/// Load and merge VM settings from standard locations.
pub fn load_merged_vm_settings() -> VmSettings {
    MergedPolicies::from_disk().vm
}

/// Load all resolved settings (for UI).
pub fn load_merged_settings() -> Vec<ResolvedSetting> {
    let (user, corp) = load_settings_and_corp_files();
    resolve_settings(&user, &corp)
}
