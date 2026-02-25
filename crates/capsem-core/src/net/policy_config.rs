/// Policy configuration loader for user.toml and corp.toml.
///
/// Locations:
///   - User: ~/.capsem/user.toml
///   - Corporate: /etc/capsem/corp.toml
///
/// Merge semantics: start with user.toml, override field-by-field with corp.toml.
/// If corp.toml specifies a field, it wins entirely (user's value is ignored for
/// that field). If corp.toml does not specify a field, user.toml's value is used.
/// If neither specifies a field, hardcoded defaults apply.
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::domain_policy::{self, Action, DomainPolicy};
use super::http_policy::{HttpPolicy, HttpRule};

/// Top-level structure for user.toml / corp.toml.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PolicyFile {
    pub network: Option<NetworkPolicyConfig>,
    pub guest: Option<GuestConfig>,
    pub vm: Option<VmSettings>,
}

/// VM resource settings section.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct VmSettings {
    /// Size of the ephemeral scratch disk in GB (default: 8).
    pub scratch_disk_size_gb: Option<u32>,
}

/// Network policy section within a TOML config file.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct NetworkPolicyConfig {
    /// Domains allowed to be accessed (exact or *.wildcard patterns).
    pub allow: Option<Vec<String>>,
    /// Domains explicitly blocked (checked before allow-list).
    pub block: Option<Vec<String>>,
    /// Default action when no rule matches: "allow" or "deny".
    pub default: Option<String>,
    /// Whether to log request/response bodies in telemetry.
    pub log_bodies: Option<bool>,
    /// Maximum bytes of body to capture in telemetry.
    pub max_body_capture: Option<usize>,
    /// HTTP-level rules (method + path matching per domain).
    pub rules: Option<Vec<HttpRuleConfig>>,
}

/// A single HTTP rule from TOML configuration.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HttpRuleConfig {
    pub domain: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub action: String,
}

/// Guest VM configuration section.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct GuestConfig {
    /// Custom environment variables injected into the guest at boot.
    pub env: Option<HashMap<String, String>>,
}

/// User config path: ~/.capsem/user.toml
pub fn user_config_path() -> Option<std::path::PathBuf> {
    dirs_path("HOME").map(|h| h.join(".capsem").join("user.toml"))
}

/// Corporate config path: /etc/capsem/corp.toml
pub fn corp_config_path() -> std::path::PathBuf {
    std::path::PathBuf::from("/etc/capsem/corp.toml")
}

fn dirs_path(env_var: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var).ok().map(std::path::PathBuf::from)
}

/// Load a policy file from disk. Returns Default if the file does not exist.
pub fn load_policy_file(path: &Path) -> Result<PolicyFile, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|e| format!("failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PolicyFile::default()),
        Err(e) => Err(format!("failed to read {}: {}", path.display(), e)),
    }
}

/// Parse a default action string ("allow" or "deny") into an Action.
fn parse_default_action(s: &str) -> Option<Action> {
    match s.to_lowercase().as_str() {
        "allow" => Some(Action::Allow),
        "deny" => Some(Action::Deny),
        _ => None,
    }
}

/// Merge user and corp policies into a DomainPolicy.
///
/// Corp fields override user fields entirely (field-level, not item-level).
/// Missing fields fall through to user, then to hardcoded defaults.
pub fn merge_policies(user: &PolicyFile, corp: &PolicyFile) -> DomainPolicy {
    let user_net = user.network.as_ref();
    let corp_net = corp.network.as_ref();

    let allow = corp_net
        .and_then(|n| n.allow.as_ref())
        .or_else(|| user_net.and_then(|n| n.allow.as_ref()));

    let block = corp_net
        .and_then(|n| n.block.as_ref())
        .or_else(|| user_net.and_then(|n| n.block.as_ref()));

    let default_str = corp_net
        .and_then(|n| n.default.as_ref())
        .or_else(|| user_net.and_then(|n| n.default.as_ref()));

    let allow_list: Vec<String> = match allow {
        Some(list) => list.clone(),
        None => domain_policy::default_allow_list()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    let block_list: Vec<String> = match block {
        Some(list) => list.clone(),
        None => domain_policy::default_block_list()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    let default_action = default_str
        .and_then(|s| parse_default_action(s))
        .unwrap_or(Action::Deny);

    DomainPolicy::new(&allow_list, &block_list, default_action)
}

/// Merge guest config from user and corp files.
///
/// Corp guest config overrides user guest config entirely (field-level).
pub fn merge_guest_config(user: &PolicyFile, corp: &PolicyFile) -> GuestConfig {
    let corp_guest = corp.guest.as_ref();
    let user_guest = user.guest.as_ref();

    let env = corp_guest
        .and_then(|g| g.env.as_ref())
        .or_else(|| user_guest.and_then(|g| g.env.as_ref()))
        .cloned();

    GuestConfig { env }
}

/// Write a policy file to disk as TOML. Creates parent dirs if needed.
pub fn write_policy_file(path: &Path, policy: &PolicyFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create dir {}: {}", parent.display(), e))?;
    }
    let content = toml::to_string_pretty(policy)
        .map_err(|e| format!("failed to serialize policy: {e}"))?;
    std::fs::write(path, content)
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Load both policy files from standard locations.
pub fn load_policy_files() -> (PolicyFile, PolicyFile) {
    let user = match user_config_path() {
        Some(path) => load_policy_file(&path).unwrap_or_else(|e| {
            tracing::warn!("user policy: {e}");
            PolicyFile::default()
        }),
        None => PolicyFile::default(),
    };

    let corp = load_policy_file(&corp_config_path()).unwrap_or_else(|e| {
        tracing::warn!("corp policy: {e}");
        PolicyFile::default()
    });

    (user, corp)
}

/// Merge user and corp policies into an HttpPolicy (domain + HTTP rules).
///
/// Corp fields override user fields entirely (field-level, not item-level).
pub fn merge_http_policy(user: &PolicyFile, corp: &PolicyFile) -> HttpPolicy {
    let domain_policy = merge_policies(user, corp);

    let user_net = user.network.as_ref();
    let corp_net = corp.network.as_ref();

    let log_bodies = corp_net
        .and_then(|n| n.log_bodies)
        .or_else(|| user_net.and_then(|n| n.log_bodies))
        .unwrap_or(false);

    let max_body_capture = corp_net
        .and_then(|n| n.max_body_capture)
        .or_else(|| user_net.and_then(|n| n.max_body_capture))
        .unwrap_or(4096);

    let rules_config = corp_net
        .and_then(|n| n.rules.as_ref())
        .or_else(|| user_net.and_then(|n| n.rules.as_ref()));

    let rules = rules_config
        .map(|configs| {
            configs
                .iter()
                .filter_map(|rc| {
                    let action = match parse_default_action(&rc.action) {
                        Some(a) => a,
                        None => {
                            tracing::warn!("invalid action '{}' in HTTP rule, skipping", rc.action);
                            return None;
                        }
                    };
                    Some(HttpRule {
                        domain: rc.domain.to_lowercase(),
                        method: rc.method.as_deref().unwrap_or("*").to_uppercase(),
                        path_pattern: rc.path.as_deref().unwrap_or("*").to_string(),
                        action,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    HttpPolicy::new(domain_policy, rules, log_bodies, max_body_capture)
}

/// Load and merge policies from the standard locations.
///
/// Reads ~/.capsem/user.toml and /etc/capsem/corp.toml, merges them,
/// and returns the resulting DomainPolicy. Logs warnings for parse errors
/// but always returns a usable policy (falls back to hardcoded defaults).
pub fn load_merged_policy() -> HttpPolicy {
    let (user, corp) = load_policy_files();
    merge_http_policy(&user, &corp)
}

/// Build a `NetworkPolicy` (new policy engine) from merged TOML config.
///
/// Bridges the TOML allow/block lists into per-domain read/write rules:
/// - Blocked domains get read=false, write=false
/// - Allowed domains get read=true, write=true
/// - Default action maps to default_allow_read and default_allow_write
///
/// Falls back to `NetworkPolicy::default_dev()` when no config files exist.
pub fn load_merged_network_policy() -> super::policy::NetworkPolicy {
    use super::policy::{NetworkPolicy, PolicyRule, DomainMatcher};

    let (user, corp) = load_policy_files();
    let user_net = user.network.as_ref();
    let corp_net = corp.network.as_ref();

    // If neither file has a network section, use hardcoded dev defaults.
    if user_net.is_none() && corp_net.is_none() {
        return NetworkPolicy::default_dev();
    }

    let allow = corp_net
        .and_then(|n| n.allow.as_ref())
        .or_else(|| user_net.and_then(|n| n.allow.as_ref()));

    let block = corp_net
        .and_then(|n| n.block.as_ref())
        .or_else(|| user_net.and_then(|n| n.block.as_ref()));

    let default_str = corp_net
        .and_then(|n| n.default.as_ref())
        .or_else(|| user_net.and_then(|n| n.default.as_ref()));

    let log_bodies = corp_net
        .and_then(|n| n.log_bodies)
        .or_else(|| user_net.and_then(|n| n.log_bodies))
        .unwrap_or(true);

    let max_body_capture = corp_net
        .and_then(|n| n.max_body_capture)
        .or_else(|| user_net.and_then(|n| n.max_body_capture))
        .unwrap_or(4096);

    // Build rules: blocked domains first (read=false, write=false),
    // then allowed domains (read=true, write=true).
    let mut rules = Vec::new();

    if let Some(block_list) = block {
        for pattern in block_list {
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(pattern),
                allow_read: false,
                allow_write: false,
            });
        }
    }

    if let Some(allow_list) = allow {
        for pattern in allow_list {
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(pattern),
                allow_read: true,
                allow_write: true,
            });
        }
    }

    // Default action: "allow" -> both true, "deny" -> both false.
    let default_allow = default_str
        .map(|s| s.to_lowercase() == "allow")
        .unwrap_or(false);

    let mut policy = NetworkPolicy::new(rules, default_allow, default_allow);
    policy.log_bodies = log_bodies;
    policy.max_body_capture = max_body_capture;
    policy
}

/// Load and merge guest config from the standard locations.
pub fn load_merged_guest_config() -> GuestConfig {
    let (user, corp) = load_policy_files();
    merge_guest_config(&user, &corp)
}

/// Default scratch disk size in GB.
const DEFAULT_SCRATCH_DISK_SIZE_GB: u32 = 8;

/// Merge VM settings from user and corp files.
///
/// Corp vm settings override user vm settings entirely (field-level).
pub fn merge_vm_settings(user: &PolicyFile, corp: &PolicyFile) -> VmSettings {
    let corp_vm = corp.vm.as_ref();
    let user_vm = user.vm.as_ref();

    let scratch_disk_size_gb = corp_vm
        .and_then(|v| v.scratch_disk_size_gb)
        .or_else(|| user_vm.and_then(|v| v.scratch_disk_size_gb))
        .or(Some(DEFAULT_SCRATCH_DISK_SIZE_GB));

    VmSettings { scratch_disk_size_gb }
}

/// Load and merge VM settings from the standard locations.
///
/// Returns merged `VmSettings` with defaults applied.
pub fn load_merged_vm_settings() -> VmSettings {
    let (user, corp) = load_policy_files();
    merge_vm_settings(&user, &corp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_user(allow: Option<Vec<&str>>, block: Option<Vec<&str>>, default: Option<&str>) -> PolicyFile {
        PolicyFile {
            network: Some(NetworkPolicyConfig {
                allow: allow.map(|v| v.into_iter().map(String::from).collect()),
                block: block.map(|v| v.into_iter().map(String::from).collect()),
                default: default.map(String::from),
                log_bodies: None,
                max_body_capture: None,
                rules: None,
            }),
            guest: None,
            vm: None,
        }
    }

    fn empty_policy() -> PolicyFile {
        PolicyFile::default()
    }

    // -- Corp overrides user --

    #[test]
    fn corp_allow_overrides_user_allow() {
        let user = make_user(
            Some(vec!["github.com", "pypi.org", "elie.net"]),
            None,
            None,
        );
        let corp = make_user(
            Some(vec!["github.com"]),
            None,
            None,
        );
        let policy = merge_policies(&user, &corp);
        // Corp specified allow, so only github.com is allowed
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = policy.evaluate("pypi.org");
        assert_eq!(action, Action::Deny); // NOT in corp allow-list
        let (action, _) = policy.evaluate("elie.net");
        assert_eq!(action, Action::Deny); // NOT in corp allow-list
    }

    #[test]
    fn corp_block_overrides_user_block() {
        let user = make_user(
            Some(vec!["github.com"]),
            Some(vec!["evil.com"]),
            None,
        );
        let corp = make_user(
            None, // not specified -> user's allow-list used
            Some(vec!["github.com"]), // corp blocks github!
            None,
        );
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Deny); // blocked by corp
    }

    #[test]
    fn corp_default_overrides_user_default() {
        let user = make_user(None, None, Some("deny"));
        let corp = make_user(None, None, Some("allow"));
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("unknown.com");
        assert_eq!(action, Action::Allow); // corp set default=allow
    }

    // -- Corp unspecified fields fall through to user --

    #[test]
    fn unspecified_corp_uses_user_allow() {
        let user = make_user(
            Some(vec!["elie.net"]),
            None,
            None,
        );
        let corp = empty_policy(); // no network section at all
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("elie.net");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn corp_with_no_network_section_uses_user() {
        let user = make_user(
            Some(vec!["github.com"]),
            Some(vec!["evil.com"]),
            Some("deny"),
        );
        let corp = PolicyFile { network: None, guest: None, vm: None };
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = policy.evaluate("evil.com");
        assert_eq!(action, Action::Deny);
    }

    // -- Neither specified -> hardcoded defaults --

    #[test]
    fn both_empty_uses_hardcoded_defaults() {
        let user = empty_policy();
        let corp = empty_policy();
        let policy = merge_policies(&user, &corp);
        // Default allow-list includes github.com
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        // Default block-list includes API providers
        let (action, _) = policy.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
        // Unknown domains denied by default
        let (action, _) = policy.evaluate("example.com");
        assert_eq!(action, Action::Deny);
    }

    // -- TOML parsing --

    #[test]
    fn parse_user_toml() {
        let toml_str = r#"
[network]
allow = ["github.com", "*.github.com"]
block = ["evil.com"]
default = "deny"
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        let net = policy.network.unwrap();
        assert_eq!(net.allow.unwrap(), vec!["github.com", "*.github.com"]);
        assert_eq!(net.block.unwrap(), vec!["evil.com"]);
        assert_eq!(net.default.unwrap(), "deny");
    }

    #[test]
    fn parse_partial_toml() {
        let toml_str = r#"
[network]
allow = ["github.com"]
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        let net = policy.network.unwrap();
        assert!(net.allow.is_some());
        assert!(net.block.is_none());
        assert!(net.default.is_none());
    }

    #[test]
    fn parse_empty_toml() {
        let policy: PolicyFile = toml::from_str("").unwrap();
        assert!(policy.network.is_none());
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let result: Result<PolicyFile, _> = toml::from_str("{{{{not valid");
        assert!(result.is_err());
    }

    // -- File loading --

    #[test]
    fn load_missing_file_returns_default() {
        let policy = load_policy_file(Path::new("/nonexistent/path/user.toml")).unwrap();
        assert!(policy.network.is_none());
    }

    // -- Invalid default action --

    #[test]
    fn invalid_default_action_falls_back_to_deny() {
        let user = make_user(None, None, Some("invalid"));
        let corp = empty_policy();
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("unknown.com");
        assert_eq!(action, Action::Deny);
    }

    // -- Corp allow-list is THE list, user can't expand it --

    #[test]
    fn user_cannot_allow_list_over_corp() {
        let user = make_user(
            Some(vec!["github.com", "elie.net", "pypi.org"]),
            None,
            None,
        );
        let corp = make_user(
            Some(vec!["github.com"]), // corp says ONLY github.com
            None,
            None,
        );
        let policy = merge_policies(&user, &corp);
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        // elie.net is in user's allow but corp overrides -> not allowed
        let (action, _) = policy.evaluate("elie.net");
        assert_eq!(action, Action::Deny);
        let (action, _) = policy.evaluate("pypi.org");
        assert_eq!(action, Action::Deny);
    }

    // -- Guest config --

    #[test]
    fn guest_config_from_user() {
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());
        let user = PolicyFile {
            network: None,
            guest: Some(GuestConfig { env: Some(env) }),
            vm: None,
        };
        let corp = empty_policy();
        let gc = merge_guest_config(&user, &corp);
        let env = gc.env.unwrap();
        assert_eq!(env.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn guest_config_corp_overrides_user() {
        let mut user_env = HashMap::new();
        user_env.insert("FOO".into(), "user_val".into());
        user_env.insert("BAR".into(), "user_bar".into());
        let user = PolicyFile {
            network: None,
            guest: Some(GuestConfig { env: Some(user_env) }),
            vm: None,
        };
        let mut corp_env = HashMap::new();
        corp_env.insert("FOO".into(), "corp_val".into());
        let corp = PolicyFile {
            network: None,
            guest: Some(GuestConfig { env: Some(corp_env) }),
            vm: None,
        };
        let gc = merge_guest_config(&user, &corp);
        let env = gc.env.unwrap();
        // Corp env overrides entirely -- BAR from user is gone
        assert_eq!(env.get("FOO").unwrap(), "corp_val");
        assert!(!env.contains_key("BAR"));
    }

    #[test]
    fn guest_config_both_empty() {
        let gc = merge_guest_config(&empty_policy(), &empty_policy());
        assert!(gc.env.is_none());
    }

    #[test]
    fn parse_guest_section_toml() {
        let toml_str = r#"
[guest]
env = { EDITOR = "vim", MY_VAR = "hello" }
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        let guest = policy.guest.unwrap();
        let env = guest.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        assert_eq!(env.get("MY_VAR").unwrap(), "hello");
    }

    #[test]
    fn parse_toml_with_both_sections() {
        let toml_str = r#"
[network]
allow = ["github.com"]

[guest]
env = { TERM = "screen" }
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        assert!(policy.network.is_some());
        assert!(policy.guest.is_some());
        let env = policy.guest.unwrap().env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "screen");
    }

    // -- HTTP rules --

    #[test]
    fn toml_roundtrip_with_rules() {
        let toml_str = r#"
[network]
allow = ["github.com", "*.github.com"]
default = "deny"

[[network.rules]]
domain = "github.com"
method = "GET"
path = "/repos/*"
action = "allow"

[[network.rules]]
domain = "github.com"
method = "POST"
path = "/repos/*"
action = "deny"
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        let rules = policy.network.as_ref().unwrap().rules.as_ref().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].domain, "github.com");
        assert_eq!(rules[0].method.as_deref(), Some("GET"));
        assert_eq!(rules[0].path.as_deref(), Some("/repos/*"));
        assert_eq!(rules[0].action, "allow");
        assert_eq!(rules[1].method.as_deref(), Some("POST"));
        assert_eq!(rules[1].action, "deny");
    }

    #[test]
    fn merge_http_policy_no_rules() {
        let user = empty_policy();
        let corp = empty_policy();
        let hp = merge_http_policy(&user, &corp);
        // Should allow default allow-listed domains
        let d = hp.evaluate_domain("github.com");
        assert_eq!(d.action, Action::Allow);
    }

    #[test]
    fn merge_http_policy_with_rules() {
        let toml_str = r#"
[network]
allow = ["github.com"]
default = "deny"

[[network.rules]]
domain = "github.com"
method = "POST"
path = "/admin/*"
action = "deny"
"#;
        let user: PolicyFile = toml::from_str(toml_str).unwrap();
        let corp = empty_policy();
        let hp = merge_http_policy(&user, &corp);

        // Domain allowed, POST /admin/* denied by rule
        let d = hp.evaluate_request("github.com", "POST", "/admin/users");
        assert_eq!(d.action, Action::Deny);
        assert_eq!(d.stage, "http-rule");

        // GET /admin is fine (no matching rule)
        let d = hp.evaluate_request("github.com", "GET", "/admin/users");
        assert_eq!(d.action, Action::Allow);
    }

    #[test]
    fn corp_rules_override_user_rules() {
        let user_toml = r#"
[network]
allow = ["github.com"]

[[network.rules]]
domain = "github.com"
method = "POST"
action = "allow"
"#;
        let corp_toml = r#"
[network]

[[network.rules]]
domain = "github.com"
method = "POST"
action = "deny"
"#;
        let user: PolicyFile = toml::from_str(user_toml).unwrap();
        let corp: PolicyFile = toml::from_str(corp_toml).unwrap();
        let hp = merge_http_policy(&user, &corp);

        // Corp rules override user rules
        let d = hp.evaluate_request("github.com", "POST", "/anything");
        assert_eq!(d.action, Action::Deny);
        assert_eq!(d.stage, "http-rule");
    }

    #[test]
    fn merge_http_policy_log_bodies() {
        let toml_str = r#"
[network]
log_bodies = true
max_body_capture = 8192
"#;
        let user: PolicyFile = toml::from_str(toml_str).unwrap();
        let corp = empty_policy();
        let hp = merge_http_policy(&user, &corp);
        assert!(hp.log_bodies);
        assert_eq!(hp.max_body_capture, 8192);
    }

    // -- VM settings --

    #[test]
    fn vm_settings_default_scratch_size() {
        let user = empty_policy();
        let corp = empty_policy();
        let vs = merge_vm_settings(&user, &corp);
        assert_eq!(vs.scratch_disk_size_gb, Some(DEFAULT_SCRATCH_DISK_SIZE_GB));
    }

    #[test]
    fn vm_settings_from_user() {
        let toml_str = r#"
[vm]
scratch_disk_size_gb = 16
"#;
        let user: PolicyFile = toml::from_str(toml_str).unwrap();
        let corp = empty_policy();
        let vs = merge_vm_settings(&user, &corp);
        assert_eq!(vs.scratch_disk_size_gb, Some(16));
    }

    #[test]
    fn vm_settings_corp_overrides_user() {
        let user_toml = r#"
[vm]
scratch_disk_size_gb = 16
"#;
        let corp_toml = r#"
[vm]
scratch_disk_size_gb = 4
"#;
        let user: PolicyFile = toml::from_str(user_toml).unwrap();
        let corp: PolicyFile = toml::from_str(corp_toml).unwrap();
        let vs = merge_vm_settings(&user, &corp);
        assert_eq!(vs.scratch_disk_size_gb, Some(4));
    }

    #[test]
    fn vm_settings_corp_unspecified_uses_user() {
        let user_toml = r#"
[vm]
scratch_disk_size_gb = 12
"#;
        let user: PolicyFile = toml::from_str(user_toml).unwrap();
        let corp = empty_policy();
        let vs = merge_vm_settings(&user, &corp);
        assert_eq!(vs.scratch_disk_size_gb, Some(12));
    }

    #[test]
    fn parse_vm_section_toml() {
        let toml_str = r#"
[vm]
scratch_disk_size_gb = 32
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        let vm = policy.vm.unwrap();
        assert_eq!(vm.scratch_disk_size_gb, Some(32));
    }

    #[test]
    fn parse_toml_with_all_sections() {
        let toml_str = r#"
[network]
allow = ["github.com"]

[guest]
env = { TERM = "screen" }

[vm]
scratch_disk_size_gb = 10
"#;
        let policy: PolicyFile = toml::from_str(toml_str).unwrap();
        assert!(policy.network.is_some());
        assert!(policy.guest.is_some());
        assert!(policy.vm.is_some());
        assert_eq!(policy.vm.unwrap().scratch_disk_size_gb, Some(10));
    }
}
