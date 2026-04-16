use std::collections::HashMap;
use super::types::*;
use super::loader::load_settings_files;
use super::resolver::resolve_settings;
use crate::net::domain_policy::{Action, DomainPolicy};
use crate::net::http_policy::{HttpPolicy, HttpRule};

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

/// Build a DomainPolicy from resolved settings.
///
/// - Bool toggles with domain metadata (registries) -> allow/block those domains
/// - `.domains` Text settings -> allow/block parsed domain patterns
/// - Corp-locked-off services use UNION of default + effective domains for blocking
/// - Default action from security.web.allow_read / security.web.allow_write
pub fn settings_to_domain_policy(resolved: &[ResolvedSetting]) -> DomainPolicy {
    let mut allow_list: Vec<String> = Vec::new();
    let mut block_list: Vec<String> = Vec::new();

    // Existing: Bool toggles with domain metadata (registries)
    for s in resolved {
        if s.metadata.domains.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if enabled {
            allow_list.extend(s.metadata.domains.clone());
        } else {
            block_list.extend(s.metadata.domains.clone());
        }
    }

    // Pass 1: collect corp-blocked domain patterns from .domains settings.
    // When corp locks .allow to false, use UNION of default + effective so
    // user can't shrink the block list below defaults.
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
            block_list.extend(all.clone());
            corp_blocked.extend(all);
        }
    }

    // Pass 2: process non-corp-locked .domains settings
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
            continue; // Already handled in pass 1
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        if toggle_on {
            // Filter: don't allow domains that corp has blocked
            for d in domains {
                if corp_blocked_matches(&d, &corp_blocked) {
                    block_list.push(d); // Override: corp says no
                } else {
                    allow_list.push(d);
                }
            }
        } else {
            block_list.extend(domains);
        }
    }

    // Custom allow/block lists from security.web.custom_allow / security.web.custom_block.
    // Block takes priority over allow for overlapping domains.
    let custom_allow = resolved
        .iter()
        .find(|s| s.id == "security.web.custom_allow")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_block = resolved
        .iter()
        .find(|s| s.id == "security.web.custom_block")
        .and_then(|s| s.effective_value.as_text())
        .unwrap_or("");
    let custom_allow_domains = parse_domain_list(custom_allow);
    let custom_block_domains = parse_domain_list(custom_block);

    // Block beats allow: any domain in custom_block goes to block_list only.
    for d in &custom_allow_domains {
        if corp_blocked_matches(d, &corp_blocked) || corp_blocked_matches(d, &custom_block_domains) {
            block_list.push(d.clone());
        } else {
            allow_list.push(d.clone());
        }
    }
    block_list.extend(custom_block_domains);

    let allow_read = resolved
        .iter()
        .find(|s| s.id == "security.web.allow_read")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);
    let allow_write = resolved
        .iter()
        .find(|s| s.id == "security.web.allow_write")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);
    // Domain policy only has a single default action: allow if either read or write is allowed.
    let default_action = if allow_read || allow_write {
        Action::Allow
    } else {
        Action::Deny
    };

    DomainPolicy::new(&allow_list, &block_list, default_action)
}

/// Build an HttpPolicy from resolved settings.
///
/// Generates HttpRules from setting metadata.rules for enabled toggles.
pub fn settings_to_http_policy(resolved: &[ResolvedSetting]) -> HttpPolicy {
    let domain_policy = settings_to_domain_policy(resolved);

    let mut http_rules: Vec<HttpRule> = Vec::new();

    for s in resolved {
        if s.metadata.rules.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if !enabled {
            continue;
        }

        // For each rule in metadata, generate HttpRules for the setting's domains
        let rule_domains: Vec<&str> = s.metadata.domains.iter().map(|d| d.as_str()).collect();

        for perms in s.metadata.rules.values() {
            let domains_for_rule = if perms.domains.is_empty() {
                rule_domains.clone()
            } else {
                perms.domains.iter().map(|d| d.as_str()).collect()
            };

            let path_pattern = perms.path.as_deref().unwrap_or("*").to_string();

            for domain in &domains_for_rule {
                // Skip wildcard domains for HTTP rules (they apply at domain level only)
                if domain.starts_with("*.") {
                    continue;
                }
                // Generate allow rules for each enabled method
                for (method, allowed) in [
                    ("GET", perms.get),
                    ("POST", perms.post),
                    ("PUT", perms.put),
                    ("DELETE", perms.delete),
                ] {
                    if allowed {
                        http_rules.push(HttpRule {
                            domain: domain.to_lowercase(),
                            method: method.to_string(),
                            path_pattern: path_pattern.clone(),
                            action: Action::Allow,
                        });
                    }
                }
            }
        }
    }

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    HttpPolicy::new(domain_policy, http_rules, log_bodies, max_body_capture)
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
        let text_value = s.effective_value.as_text().unwrap_or("");

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
                let val = if s.effective_value.as_bool().unwrap_or(false) { "1" } else { "0" };
                env.insert(var_name.to_string(), val.to_string());
            }
        }

        // Metadata-driven env var injection: if the setting declares env_vars
        // and the effective value is non-empty text, inject each env var.
        // For File values, the content is used as the env value.
        let env_text = match &s.effective_value {
            SettingValue::Text(t) => Some(t.as_str()),
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
        if let SettingValue::File { path: file_path, content: file_content } = &s.effective_value {
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
            if !text_value.is_empty() {
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
            .and_then(|s| s.effective_value.as_text())
            .unwrap_or("");
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
                    server_map.insert(
                        s.key.clone(),
                        serde_json::json!({"command": cmd}),
                    );
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
        let approved = r
            .entry("approved")
            .or_insert_with(|| serde_json::json!([]));
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
    pub domain: DomainPolicy,
    pub http: HttpPolicy,
    pub mcp: crate::mcp::policy::McpPolicy,
    pub guest: GuestConfig,
    pub vm: VmSettings,
}

impl MergedPolicies {
    /// Pure merge function. No I/O, fully testable.
    pub fn from_files(user: &SettingsFile, corp: &SettingsFile) -> Self {
        let resolved = resolve_settings(user, corp);
        let mcp_user = user.mcp.clone().unwrap_or_default();
        let mcp_corp = corp.mcp.clone().unwrap_or_default();

        let mut host_aliases = HashMap::new();
        if let Some(user_aliases) = &user.host_aliases {
            host_aliases.extend(user_aliases.clone());
        }
        if let Some(corp_aliases) = &corp.host_aliases {
            host_aliases.extend(corp_aliases.clone());
        }
        
        // Auto-map Ollama domains to local plaintext endpoint
        let ollama_domains = resolved
            .iter()
            .find(|s| s.id == "ai.ollama.domains")
            .and_then(|s| s.effective_value.as_text())
            .unwrap_or("");
        
        for domain in parse_domain_list(ollama_domains) {
            // Strip wildcard for routing if present
            let key = if let Some(stripped) = domain.strip_prefix("*.") {
                stripped.to_string()
            } else {
                domain.clone()
            };
            if !host_aliases.contains_key(&key) {
                host_aliases.insert(key, "http://127.0.0.1:11434".to_string());
            }
        }

        Self {
            network: build_network_policy(&resolved, host_aliases),
            domain: settings_to_domain_policy(&resolved),
            http: settings_to_http_policy(&resolved),
            mcp: mcp_user.to_policy(&mcp_corp),
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

/// Build a `NetworkPolicy` from resolved settings (pure, no I/O).
///
/// Bridges settings into per-domain read/write rules:
/// - Disabled toggles with domains get read=false, write=false
/// - Enabled toggles with domains get read=true, write=true
/// - Default action maps to default_allow_read and default_allow_write
pub fn build_network_policy(resolved: &[ResolvedSetting], host_aliases: HashMap<String, String>) -> crate::net::policy::NetworkPolicy {
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

    // Build rules from .domains text settings (AI providers)
    // Corp block enforcement: same two-pass approach as settings_to_domain_policy
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

    // Custom allow/block lists: same pattern as settings_to_domain_policy
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
    policy.log_bodies = log_bodies;
    policy.max_body_capture = max_body_capture;
    policy.host_aliases = host_aliases;
    policy
}

// ---------------------------------------------------------------------------
// High-level entry points (thin wrappers over MergedPolicies)
// ---------------------------------------------------------------------------

/// Load and merge settings, then build an HttpPolicy.
pub fn load_merged_policy() -> HttpPolicy {
    MergedPolicies::from_disk().http
}

/// Build a `DomainPolicy` from merged settings.
///
/// Convenience wrapper matching the `load_merged_network_policy()` pattern.
/// Used by the MCP gateway to check built-in HTTP tool domains.
pub fn load_merged_domain_policy() -> DomainPolicy {
    MergedPolicies::from_disk().domain
}

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
    use crate::net::domain_policy::Action;

    fn make_setting(id: &str, typ: SettingType, value: SettingValue) -> ResolvedSetting {
        ResolvedSetting {
            id: id.to_string(),
            category: "test".into(),
            name: id.to_string(),
            description: "".into(),
            setting_type: typ,
            default_value: value.clone(),
            effective_value: value,
            source: PolicySource::Default,
            modified: None,
            corp_locked: false,
            enabled_by: None,
            enabled: true,
            metadata: SettingMetadata::default(),
            collapsed: false,
            history: vec![],
        }
    }

    fn make_bool_setting(id: &str, value: bool, domains: Vec<String>) -> ResolvedSetting {
        let mut s = make_setting(id, SettingType::Bool, SettingValue::Bool(value));
        s.metadata.domains = domains;
        s
    }

    fn make_text_setting(id: &str, value: &str) -> ResolvedSetting {
        make_setting(id, SettingType::Text, SettingValue::Text(value.to_string()))
    }

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
        let blocked = vec![
            "evil.com".to_string(),
            "*.bad.org".to_string(),
        ];
        assert!(corp_blocked_matches("evil.com", &blocked));
        assert!(corp_blocked_matches("sub.bad.org", &blocked));
        assert!(!corp_blocked_matches("good.com", &blocked));
    }

    // -----------------------------------------------------------------------
    // settings_to_domain_policy
    // -----------------------------------------------------------------------

    #[test]
    fn domain_policy_empty_settings() {
        let policy = settings_to_domain_policy(&[]);
        // Empty settings: no allow_read, no allow_write -> default deny
        assert_eq!(policy.evaluate("example.com").0, Action::Deny);
    }

    #[test]
    fn domain_policy_allow_read_default_allow() {
        let settings = vec![
            make_setting("security.web.allow_read", SettingType::Bool, SettingValue::Bool(true)),
        ];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("unknown.com").0, Action::Allow);
    }

    #[test]
    fn domain_policy_bool_toggle_adds_domains() {
        let settings = vec![
            make_bool_setting("ai.anthropic.allow", true, vec!["api.anthropic.com".into()]),
            make_setting("security.web.allow_read", SettingType::Bool, SettingValue::Bool(false)),
        ];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("api.anthropic.com").0, Action::Allow);
    }

    #[test]
    fn domain_policy_bool_toggle_off_blocks_domains() {
        let settings = vec![
            make_bool_setting("ai.anthropic.allow", false, vec!["api.anthropic.com".into()]),
            make_setting("security.web.allow_read", SettingType::Bool, SettingValue::Bool(false)),
        ];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("api.anthropic.com").0, Action::Deny);
    }

    #[test]
    fn domain_policy_custom_block_beats_allow() {
        let settings = vec![
            make_setting("security.web.custom_allow", SettingType::Text, SettingValue::Text("example.com".into())),
            make_setting("security.web.custom_block", SettingType::Text, SettingValue::Text("example.com".into())),
            make_setting("security.web.allow_read", SettingType::Bool, SettingValue::Bool(true)),
        ];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("example.com").0, Action::Deny);
    }

    #[test]
    fn domain_policy_custom_allow_works() {
        let settings = vec![
            make_setting("security.web.custom_allow", SettingType::Text, SettingValue::Text("allowed.com".into())),
            make_setting("security.web.allow_read", SettingType::Bool, SettingValue::Bool(false)),
        ];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("allowed.com").0, Action::Allow);
    }

    #[test]
    fn domain_policy_corp_locked_off_blocks_union() {
        let mut toggle = make_bool_setting("test.provider.allow", false, vec![]);
        toggle.corp_locked = true;

        let mut domains = make_text_setting("test.provider.domains", "");
        domains.effective_value = SettingValue::Text("user-added.com".into());
        domains.default_value = SettingValue::Text("default.com".into());

        let settings = vec![toggle, domains];
        let policy = settings_to_domain_policy(&settings);
        assert_eq!(policy.evaluate("default.com").0, Action::Deny);
        assert_eq!(policy.evaluate("user-added.com").0, Action::Deny);
    }

    // -----------------------------------------------------------------------
    // settings_to_http_policy
    // -----------------------------------------------------------------------

    #[test]
    fn http_policy_empty_settings() {
        let policy = settings_to_http_policy(&[]);
        assert!(!policy.log_bodies);
    }

    #[test]
    fn http_policy_log_bodies_setting() {
        let settings = vec![
            make_setting("vm.resources.log_bodies", SettingType::Bool, SettingValue::Bool(true)),
        ];
        let policy = settings_to_http_policy(&settings);
        assert!(policy.log_bodies);
    }

    #[test]
    fn http_policy_max_body_capture_default() {
        let policy = settings_to_http_policy(&[]);
        assert_eq!(policy.max_body_capture, 4096);
    }

    #[test]
    fn http_policy_max_body_capture_custom() {
        let settings = vec![
            make_setting("vm.resources.max_body_capture", SettingType::Number, SettingValue::Number(8192)),
        ];
        let policy = settings_to_http_policy(&settings);
        assert_eq!(policy.max_body_capture, 8192);
    }
}
