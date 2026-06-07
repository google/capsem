use super::loader::load_settings_files;
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

/// Parse a comma-separated domain list into trimmed individual entries.
fn parse_domain_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

fn parse_http_upstream_ports(values: &[i64]) -> Vec<u16> {
    values
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect()
}

/// Check if a candidate domain matches any corp-blocked pattern.
/// Uses the same wildcard logic as DomainPattern: suffix match for `*.foo.com`,
/// exact match otherwise.
fn corp_blocked_matches(candidate: &str, corp_blocked: &[String]) -> bool {
    let candidate = candidate.to_lowercase();
    for pattern in corp_blocked {
        let pattern = pattern.to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            if candidate.ends_with(&format!(".{suffix}")) || candidate == suffix {
                return true;
            }
        } else if candidate == pattern {
            return true;
        }
    }
    false
}

/// Extract guest config from resolved settings.
///
/// Dynamic keys with prefix `guest.env.` become environment variables.
/// AI provider API keys and boot files are always injected when the key/value
/// is non-empty, regardless of the provider toggle. The toggle controls network
/// access (domain policy), not whether credentials are available in the VM.
/// This ensures the user can enable a provider at runtime without rebooting.
pub fn settings_to_guest_config(resolved: &[ResolvedSetting]) -> GuestConfig {
    use capsem_proto::{validate_env_key, validate_env_value, validate_file_path};

    let mut env = HashMap::new();
    let mut files = Vec::new();

    for s in resolved {
        let text_value = resolved_text_for_guest(s);

        // Provider allow toggles: inject CAPSEM_<PROVIDER>_ALLOWED=1|0
        // so the guest banner can show which AI tools are enabled.
        // Also surface the default web read/write toggles so in-VM
        // diagnostics can adapt their "denied domain" assertions when
        // the user has opted to let unknown domains through.
        if s.setting_type == SettingType::Bool {
            let bool_env = match s.id.as_str() {
                SETTING_ANTHROPIC_ALLOW => Some("CAPSEM_ANTHROPIC_ALLOWED"),
                SETTING_OPENAI_ALLOW => Some("CAPSEM_OPENAI_ALLOWED"),
                SETTING_GOOGLE_ALLOW => Some("CAPSEM_GOOGLE_ALLOWED"),
                "security.web.allow_read" => Some("CAPSEM_WEB_ALLOW_READ"),
                "security.web.allow_write" => Some("CAPSEM_WEB_ALLOW_WRITE"),
                _ => None,
            };
            if let Some(var_name) = bool_env {
                let val = if s.effective_value.as_bool().unwrap_or(false) {
                    "1"
                } else {
                    "0"
                };
                env.insert(var_name.to_string(), val.to_string());
            }
        }

        // Metadata-driven env var injection: if the setting declares env_vars
        // and the effective value is non-empty text, inject each env var.
        // For File values, the content is used as the env value.
        let env_text = match &s.effective_value {
            SettingValue::Text(_) => text_value.as_deref(),
            SettingValue::File { content, .. } => Some(content.as_str()),
            _ => None,
        };
        if let Some(ev) = env_text {
            if !s.metadata.env_vars.is_empty() && !ev.is_empty() {
                for var_name in &s.metadata.env_vars {
                    if let Err(e) = validate_env_key(var_name) {
                        tracing::warn!("skipping invalid env var from metadata: {e}");
                        continue;
                    }
                    if let Err(e) = validate_env_value(ev) {
                        tracing::warn!("skipping env var {var_name}: invalid value: {e}");
                        continue;
                    }
                    env.insert(var_name.clone(), ev.to_string());
                }
            }
        }

        // Boot files: File values with non-empty content.
        // Always inject if non-empty -- the allow toggle controls network
        // policy, not file availability.
        if let SettingValue::File {
            path: file_path,
            content: file_content,
        } = &s.effective_value
        {
            if !file_content.is_empty() {
                if let Err(e) = validate_file_path(file_path) {
                    tracing::warn!("skipping boot file: {e}");
                    continue;
                }

                // Inject capsem MCP server into AI CLI config files:
                // - settings.json: Claude Code + Gemini CLI (JSON mcpServers)
                // - .claude.json: Claude Code state file (JSON mcpServers + API key approval)
                // - config.toml: Codex CLI (TOML mcp_servers)
                //
                // Pattern-match on the guest path (not the setting ID) since
                // the path is the source of truth for what the file represents.
                let content = if file_path.ends_with("/settings.json") {
                    inject_capsem_mcp_server(file_content)
                } else if file_path == "/root/.claude.json" {
                    let with_mcp = inject_capsem_mcp_server(file_content);
                    if let Some(api_key) = env.get("ANTHROPIC_API_KEY") {
                        inject_api_key_approval(&with_mcp, api_key)
                    } else {
                        with_mcp
                    }
                } else if file_path.ends_with("/config.toml") {
                    inject_capsem_mcp_server_toml(file_content)
                } else {
                    file_content.clone()
                };

                // Settings files may contain API keys or sensitive config --
                // restrict to owner-only (0o600) rather than world-readable.
                files.push(GuestFile {
                    path: file_path.clone(),
                    content,
                    mode: 0o600,
                });
            }
        }

        // Dynamic guest.env.* settings (not in registry)
        if let Some(var_name) = s.id.strip_prefix("guest.env.") {
            if let Some(text_value) = text_value.as_deref().filter(|v| !v.is_empty()) {
                if let Err(e) = validate_env_key(var_name) {
                    tracing::warn!("skipping dynamic env var: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(text_value) {
                    tracing::warn!("skipping dynamic env var {var_name}: invalid value: {e}");
                    continue;
                }
                env.insert(var_name.to_string(), text_value.to_string());
            }
        }
    }

    // .git-credentials generation: inject credentials for git push over HTTPS.
    // Format: https://oauth2:TOKEN@github.com (one line per provider).
    // Requires credential.helper=store in .gitconfig (generated below).
    let token_providers = [
        (SETTING_GITHUB_TOKEN, SETTING_GITHUB_ALLOW, "github.com"),
        (SETTING_GITLAB_TOKEN, SETTING_GITLAB_ALLOW, "gitlab.com"),
    ];

    let mut credential_lines: Vec<String> = Vec::new();
    for (token_id, allow_id, host) in &token_providers {
        let allowed = resolved
            .iter()
            .find(|s| s.id == *allow_id)
            .and_then(|s| s.effective_value.as_bool())
            .unwrap_or(false);
        if !allowed {
            continue;
        }
        let token = resolved
            .iter()
            .find(|s| s.id == *token_id)
            .and_then(resolved_text_for_guest)
            .unwrap_or_default();
        if token.is_empty() {
            continue;
        }
        // Security: reject tokens with newlines, @, or : to prevent URL injection.
        if token.contains('\n')
            || token.contains('\r')
            || token.contains('@')
            || token.contains(':')
        {
            tracing::warn!(
                "skipping git credential for {host}: token contains forbidden characters"
            );
            continue;
        }
        credential_lines.push(format!("https://oauth2:{token}@{host}"));
    }

    if !credential_lines.is_empty() {
        files.push(GuestFile {
            path: "/root/.git-credentials".to_string(),
            content: credential_lines.join("\n") + "\n",
            mode: 0o600,
        });
        // Generate .gitconfig with credential.helper = store so git reads .git-credentials.
        // Also include safe.directory = * to avoid "dubious ownership" errors in the sandbox.
        files.push(GuestFile {
            path: "/root/.gitconfig".to_string(),
            content: "[credential]\n\thelper = store\n[safe]\n\tdirectory = *\n".to_string(),
            mode: 0o644,
        });
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

/// Inject MCP server entries into a JSON config string (Claude Code, Gemini CLI).
///
/// For each server with a stdio transport and command, inserts
/// `mcpServers.{key}.command = "{command}"` preserving any user-provided entries.
/// Returns the original string unchanged if parsing fails.
pub(super) fn inject_mcp_servers_json(json_str: &str, servers: &[McpServerDef]) -> String {
    let mut json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return json_str.to_string(),
    };

    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return json_str.to_string(),
    };

    let mcp_servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if let Some(server_map) = mcp_servers.as_object_mut() {
        for s in servers {
            if s.transport == McpTransport::Stdio {
                if let Some(cmd) = &s.command {
                    server_map.insert(s.key.clone(), serde_json::json!({"command": cmd}));
                }
            }
        }
    }

    serde_json::to_string(&json).unwrap_or_else(|_| json_str.to_string())
}

/// Backward-compatible wrapper: inject capsem MCP server (delegates to generic version).
pub(super) fn inject_capsem_mcp_server(json_str: &str) -> String {
    let servers = super::loader::load_mcp_servers();
    inject_mcp_servers_json(json_str, &servers)
}

/// Inject MCP server entries into a TOML config string (Codex CLI).
///
/// For each server with a stdio transport and command, inserts
/// `[mcp_servers.{key}] command = "{command}"` preserving user-provided entries.
/// Returns the original string unchanged if parsing fails.
pub(super) fn inject_mcp_servers_toml(toml_str: &str, servers: &[McpServerDef]) -> String {
    let mut doc: toml::Value = match toml::from_str(toml_str) {
        Ok(v) => v,
        Err(_) => return toml_str.to_string(),
    };
    let table = match doc.as_table_mut() {
        Some(t) => t,
        None => return toml_str.to_string(),
    };
    let mcp = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if let Some(server_map) = mcp.as_table_mut() {
        for s in servers {
            if s.transport == McpTransport::Stdio {
                if let Some(cmd) = &s.command {
                    let mut entry = toml::map::Map::new();
                    entry.insert("command".into(), toml::Value::String(cmd.clone()));
                    server_map.insert(s.key.clone(), toml::Value::Table(entry));
                }
            }
        }
    }
    toml::to_string(&doc).unwrap_or_else(|_| toml_str.to_string())
}

/// Backward-compatible wrapper: inject capsem MCP server into TOML (delegates to generic version).
pub(super) fn inject_capsem_mcp_server_toml(toml_str: &str) -> String {
    let servers = super::loader::load_mcp_servers();
    inject_mcp_servers_toml(toml_str, &servers)
}

/// Inject `customApiKeyResponses` into Claude state JSON.
///
/// Pre-approves the last 20 characters of the API key so Claude Code doesn't
/// prompt the user to "trust" it on first use. Returns the original string
/// unchanged if parsing fails.
pub(super) fn inject_api_key_approval(json_str: &str, api_key: &str) -> String {
    let mut json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return json_str.to_string(),
    };

    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return json_str.to_string(),
    };

    let key_suffix: String = if api_key.len() > 20 {
        api_key[api_key.len() - 20..].to_string()
    } else {
        api_key.to_string()
    };

    let responses = obj
        .entry("customApiKeyResponses")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(r) = responses.as_object_mut() {
        let approved = r.entry("approved").or_insert_with(|| serde_json::json!([]));
        if let Some(arr) = approved.as_array_mut() {
            if !arr.iter().any(|v| v.as_str() == Some(&key_suffix)) {
                arr.push(serde_json::json!(key_suffix));
            }
        }
        r.entry("rejected").or_insert_with(|| serde_json::json!([]));
    }

    serde_json::to_string(&json).unwrap_or_else(|_| json_str.to_string())
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
    pub network: crate::net::policy::NetworkPolicy,
    pub mcp: crate::mcp::policy::McpPolicy,
    pub security_rules: SecurityRuleSet,
    pub plugins: BTreeMap<String, SecurityPluginConfig>,
    pub model_endpoints: ModelEndpointRegistry,
    pub guest: GuestConfig,
    pub vm: VmSettings,
}

impl MergedPolicies {
    /// Pure merge function. No I/O, fully testable.
    pub fn from_files(user: &SettingsFile, corp: &SettingsFile) -> Self {
        let resolved = resolve_settings(user, corp);
        let mcp_user = user.mcp.clone().unwrap_or_default();
        let mcp_corp = corp.mcp.clone().unwrap_or_default();
        let security_rules = match compile_merged_security_rules(user, corp) {
            Ok(rules) => rules,
            Err(error) => {
                tracing::warn!("security rules ignored: {error}");
                SecurityRuleSet::new(Vec::new())
            }
        };
        let model_endpoints = match compile_model_endpoint_registry(user, corp) {
            Ok(registry) => registry,
            Err(error) => {
                tracing::warn!("model endpoint registry ignored: {error}");
                ModelEndpointRegistry::default()
            }
        };
        let plugins = merge_plugin_policy(user, corp);
        Self {
            network: build_network_policy(&resolved),
            mcp: mcp_user.to_policy(&mcp_corp),
            security_rules,
            plugins,
            model_endpoints,
            guest: settings_to_guest_config(&resolved),
            vm: settings_to_vm_settings(&resolved),
        }
    }

    /// Load from disk then merge. Falls back to defaults on any I/O error.
    pub fn from_disk() -> Self {
        let (user, corp) = load_settings_files();
        Self::from_files(&user, &corp)
    }
}

fn merge_plugin_policy(
    user: &SettingsFile,
    corp: &SettingsFile,
) -> BTreeMap<String, SecurityPluginConfig> {
    let mut plugins = user.plugins.clone();
    for (plugin_id, mode) in &corp.plugins {
        plugins.insert(plugin_id.clone(), *mode);
    }
    plugins
}

fn compile_model_endpoint_registry(
    user: &SettingsFile,
    corp: &SettingsFile,
) -> Result<ModelEndpointRegistry, String> {
    let merged = ProviderRuleProfile::merge_defaults_user_and_corp(
        &ProviderRuleProfile {
            ai: user.ai.clone(),
        },
        &ProviderRuleProfile {
            ai: corp.ai.clone(),
        },
    )?;
    merged.endpoint_registry()
}

fn compile_merged_security_rules(
    user: &SettingsFile,
    corp: &SettingsFile,
) -> Result<SecurityRuleSet, String> {
    let mut by_rule_id = std::collections::BTreeMap::new();
    let provider_rules = compile_provider_rules_to_security_rule_set(
        &ProviderRuleProfile {
            ai: user.ai.clone(),
        },
        &ProviderRuleProfile {
            ai: corp.ai.clone(),
        },
    )?;
    for rule in provider_rules.rules() {
        by_rule_id.insert(rule.rule_id.clone(), rule.clone());
    }
    let user_profile = SecurityRuleProfile {
        profiles: user.profiles.clone(),
        ..SecurityRuleProfile::default()
    };
    for rule in user_profile.compile(SecurityRuleSource::User)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    let corp_profile = SecurityRuleProfile {
        corp: corp.corp.clone(),
        profiles: corp.profiles.clone(),
        ..SecurityRuleProfile::default()
    };
    for rule in corp_profile.compile(SecurityRuleSource::Corp)? {
        by_rule_id.insert(rule.rule_id.clone(), rule);
    }
    Ok(SecurityRuleSet::new(by_rule_id.into_values().collect()))
}

/// Build a `NetworkPolicy` from resolved settings (pure, no I/O).
///
/// Bridges settings into per-domain read/write rules:
/// - Disabled toggles with domains get read=false, write=false
/// - Enabled toggles with domains get read=true, write=true
/// - Default action maps to default_allow_read and default_allow_write
pub fn build_network_policy(resolved: &[ResolvedSetting]) -> crate::net::policy::NetworkPolicy {
    use crate::net::policy::{DomainMatcher, NetworkPolicy, PolicyRule};

    let mut rules = Vec::new();

    // Build rules from settings with domain metadata (registries)
    for s in resolved {
        if s.metadata.domains.is_empty() || s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        for domain in &s.metadata.domains {
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    // Build network mechanics from .domains text settings (AI providers).
    // Security allow/block decisions live in SecurityRuleSet.
    let mut corp_blocked: Vec<String> = Vec::new();
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            let defaults = parse_domain_list(s.default_value.as_text().unwrap_or(""));
            let effective = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
            let mut all: Vec<String> = defaults;
            for d in effective {
                if !all.contains(&d) {
                    all.push(d);
                }
            }
            for domain in &all {
                rules.push(PolicyRule {
                    matcher: DomainMatcher::parse(domain),
                    allow_read: false,
                    allow_write: false,
                });
            }
            corp_blocked.extend(all);
        }
    }
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            continue;
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        for domain in &domains {
            let blocked = corp_blocked_matches(domain, &corp_blocked);
            let enabled = toggle_on && !blocked;
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    // Custom allow/block network mechanics mirror the settings state.
    let custom_allow_text = resolved
        .iter()
        .find(|s| s.id == "security.web.custom_allow")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_block_text = resolved
        .iter()
        .find(|s| s.id == "security.web.custom_block")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_allow_domains = parse_domain_list(custom_allow_text);
    let custom_block_domains = parse_domain_list(custom_block_text);

    for domain in &custom_allow_domains {
        let blocked = corp_blocked_matches(domain, &corp_blocked)
            || corp_blocked_matches(domain, &custom_block_domains);
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(domain),
            allow_read: !blocked,
            allow_write: !blocked,
        });
    }
    for domain in &custom_block_domains {
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(domain),
            allow_read: false,
            allow_write: false,
        });
    }

    let default_allow_read = resolved
        .iter()
        .find(|s| s.id == "security.web.allow_read")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);
    let default_allow_write = resolved
        .iter()
        .find(|s| s.id == "security.web.allow_write")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);

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

    let mut policy = NetworkPolicy::new(rules, default_allow_read, default_allow_write);
    if let Some(ports) = resolved
        .iter()
        .find(|s| s.id == "security.web.http_upstream_ports")
        .and_then(|s| s.effective_value.as_int_list())
    {
        policy.http_upstream_ports = parse_http_upstream_ports(ports);
    }
    policy.log_bodies = log_bodies;
    policy.max_body_capture = max_body_capture;
    policy
}

// ---------------------------------------------------------------------------
// High-level entry points (thin wrappers over MergedPolicies)
// ---------------------------------------------------------------------------

/// Build a `NetworkPolicy` (new policy engine) from merged settings.
pub fn load_merged_network_policy() -> crate::net::policy::NetworkPolicy {
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
    let (user, corp) = load_settings_files();
    resolve_settings(&user, &corp)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // parse_domain_list
    // -----------------------------------------------------------------------

    #[test]
    fn parse_domain_list_basic() {
        let result = parse_domain_list("foo.com, bar.com, baz.com");
        assert_eq!(result, vec!["foo.com", "bar.com", "baz.com"]);
    }

    #[test]
    fn parse_domain_list_trims_whitespace() {
        let result = parse_domain_list("  foo.com  ,  bar.com  ");
        assert_eq!(result, vec!["foo.com", "bar.com"]);
    }

    #[test]
    fn parse_domain_list_empty_string() {
        let result = parse_domain_list("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_domain_list_skips_empty_entries() {
        let result = parse_domain_list("foo.com,,bar.com,,");
        assert_eq!(result, vec!["foo.com", "bar.com"]);
    }

    #[test]
    fn parse_domain_list_single() {
        let result = parse_domain_list("single.com");
        assert_eq!(result, vec!["single.com"]);
    }

    #[test]
    fn parse_domain_list_wildcards() {
        let result = parse_domain_list("*.example.com, api.test.com");
        assert_eq!(result, vec!["*.example.com", "api.test.com"]);
    }

    // -----------------------------------------------------------------------
    // corp_blocked_matches
    // -----------------------------------------------------------------------

    #[test]
    fn corp_blocked_exact_match() {
        let blocked = vec!["evil.com".to_string()];
        assert!(corp_blocked_matches("evil.com", &blocked));
        assert!(!corp_blocked_matches("good.com", &blocked));
    }

    #[test]
    fn corp_blocked_wildcard_match() {
        let blocked = vec!["*.evil.com".to_string()];
        assert!(corp_blocked_matches("sub.evil.com", &blocked));
        assert!(corp_blocked_matches("deep.sub.evil.com", &blocked));
        assert!(corp_blocked_matches("evil.com", &blocked)); // bare domain matches *.
        assert!(!corp_blocked_matches("notevil.com", &blocked));
    }

    #[test]
    fn corp_blocked_case_insensitive() {
        let blocked = vec!["Evil.Com".to_string()];
        assert!(corp_blocked_matches("evil.com", &blocked));
        assert!(corp_blocked_matches("EVIL.COM", &blocked));
    }

    #[test]
    fn corp_blocked_empty_list() {
        let blocked: Vec<String> = vec![];
        assert!(!corp_blocked_matches("anything.com", &blocked));
    }

    #[test]
    fn corp_blocked_multiple_patterns() {
        let blocked = vec!["evil.com".to_string(), "*.bad.org".to_string()];
        assert!(corp_blocked_matches("evil.com", &blocked));
        assert!(corp_blocked_matches("sub.bad.org", &blocked));
        assert!(!corp_blocked_matches("good.com", &blocked));
    }

}
