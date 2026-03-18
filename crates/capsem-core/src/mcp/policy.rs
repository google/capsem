use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MCP user/corp config (stored in user.toml / corp.toml under [mcp])
// ---------------------------------------------------------------------------

/// MCP configuration from user.toml or corp.toml `[mcp]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct McpUserConfig {
    /// Global MCP policy: "allow" (default) or "block".
    #[serde(default)]
    pub global_policy: Option<String>,
    /// Default permission for tools not in the per-tool map.
    #[serde(default)]
    pub default_tool_permission: Option<ToolDecision>,
    /// Health check interval in seconds (default: 300).
    #[serde(default)]
    pub health_check_interval_secs: Option<u64>,
    /// Manually configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpManualServer>,
    /// Per-server enabled overrides (name -> enabled).
    #[serde(default)]
    pub server_enabled: HashMap<String, bool>,
    /// Per-tool permission overrides (namespaced_name -> decision).
    #[serde(default)]
    pub tool_permissions: HashMap<String, ToolDecision>,
}

impl McpUserConfig {
    /// Check if the global policy is "block".
    pub fn is_globally_blocked(&self) -> bool {
        self.global_policy.as_deref() == Some("block")
    }

    /// Build a runtime McpPolicy from this config merged with corp overrides.
    pub fn to_policy(&self, corp: &McpUserConfig) -> McpPolicy {
        // Corp global block overrides everything
        if corp.is_globally_blocked() || self.is_globally_blocked() {
            return McpPolicy {
                default_tool_decision: ToolDecision::Block,
                ..McpPolicy::new()
            };
        }

        // Default tool permission: corp > user > Allow
        let default_perm = corp.default_tool_permission
            .or(self.default_tool_permission)
            .unwrap_or(ToolDecision::Allow);

        // Merge server enabled: corp overrides user for same key
        let mut server_enabled = self.server_enabled.clone();
        for (k, v) in &corp.server_enabled {
            server_enabled.insert(k.clone(), *v);
        }

        // Build blocked servers from disabled entries
        let blocked_servers: Vec<String> = server_enabled
            .iter()
            .filter(|(_, enabled)| !*enabled)
            .map(|(name, _)| name.clone())
            .collect();

        // Merge tool permissions: corp overrides user for same key
        let mut tool_decisions = self.tool_permissions.clone();
        for (k, v) in &corp.tool_permissions {
            tool_decisions.insert(k.clone(), *v);
        }

        McpPolicy {
            blocked_servers,
            allowed_servers: Vec::new(),
            tool_decisions,
            default_tool_decision: default_perm,
        }
    }
}

/// A manually configured MCP server definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpManualServer {
    pub name: String,
    /// HTTP endpoint URL for the MCP server.
    pub url: String,
    /// Custom HTTP headers to send with every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Bearer token for Authorization header.
    #[serde(default)]
    pub bearer_token: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Per-tool policy decision
// ---------------------------------------------------------------------------

/// Per-tool policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolDecision {
    Allow,
    Warn,
    Block,
}

impl ToolDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolDecision::Allow => "allow",
            ToolDecision::Warn => "warn",
            ToolDecision::Block => "block",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "allow" => ToolDecision::Allow,
            "warn" => ToolDecision::Warn,
            "block" => ToolDecision::Block,
            _ => ToolDecision::Allow,
        }
    }

    /// Convert to the decision string stored in the mcp_calls table.
    pub fn to_log_decision(&self) -> &'static str {
        match self {
            ToolDecision::Allow => "allowed",
            ToolDecision::Warn => "warned",
            ToolDecision::Block => "denied",
        }
    }
}

/// MCP gateway policy: server-level and per-tool allow/warn/block.
#[derive(Debug, Clone)]
pub struct McpPolicy {
    /// Servers that are always blocked.
    pub blocked_servers: Vec<String>,
    /// If non-empty, only these servers are allowed.
    pub allowed_servers: Vec<String>,
    /// Per-tool decisions, keyed by namespaced name (e.g. "github__search_repos").
    pub tool_decisions: HashMap<String, ToolDecision>,
    /// Default decision for tools not in the map.
    pub default_tool_decision: ToolDecision,
}

impl McpPolicy {
    pub fn new() -> Self {
        Self {
            blocked_servers: Vec::new(),
            allowed_servers: Vec::new(),
            tool_decisions: HashMap::new(),
            default_tool_decision: ToolDecision::Allow,
        }
    }

    /// Evaluate policy for a given server and optional tool name.
    /// Block-before-allow at server level, then per-tool decision.
    pub fn evaluate(&self, server: &str, tool: Option<&str>) -> ToolDecision {
        // Server-level: block list takes priority
        if self.blocked_servers.iter().any(|s| s == server) {
            return ToolDecision::Block;
        }

        // Server-level: if allow list is non-empty, server must be in it
        if !self.allowed_servers.is_empty()
            && !self.allowed_servers.iter().any(|s| s == server)
        {
            return ToolDecision::Block;
        }

        // Per-tool decision
        if let Some(tool_name) = tool {
            if let Some(&decision) = self.tool_decisions.get(tool_name) {
                return decision;
            }
        }

        self.default_tool_decision
    }
}

impl Default for McpPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_allows_all() {
        let policy = McpPolicy::new();
        assert_eq!(policy.evaluate("github", None), ToolDecision::Allow);
        assert_eq!(
            policy.evaluate("github", Some("github__search")),
            ToolDecision::Allow
        );
    }

    #[test]
    fn blocked_server_denies_everything() {
        let policy = McpPolicy {
            blocked_servers: vec!["evil".to_string()],
            ..McpPolicy::new()
        };
        assert_eq!(policy.evaluate("evil", None), ToolDecision::Block);
        assert_eq!(
            policy.evaluate("evil", Some("evil__do_stuff")),
            ToolDecision::Block
        );
        // Other servers still allowed
        assert_eq!(policy.evaluate("github", None), ToolDecision::Allow);
    }

    #[test]
    fn block_overrides_allow() {
        let policy = McpPolicy {
            blocked_servers: vec!["github".to_string()],
            allowed_servers: vec!["github".to_string()],
            ..McpPolicy::new()
        };
        // Block list takes priority over allow list
        assert_eq!(policy.evaluate("github", None), ToolDecision::Block);
    }

    #[test]
    fn allow_list_restricts_to_listed_only() {
        let policy = McpPolicy {
            allowed_servers: vec!["github".to_string()],
            ..McpPolicy::new()
        };
        assert_eq!(policy.evaluate("github", None), ToolDecision::Allow);
        assert_eq!(policy.evaluate("slack", None), ToolDecision::Block);
    }

    #[test]
    fn per_tool_block() {
        let mut tool_decisions = HashMap::new();
        tool_decisions.insert("github__delete_repo".to_string(), ToolDecision::Block);
        tool_decisions.insert("github__admin_access".to_string(), ToolDecision::Warn);

        let policy = McpPolicy {
            tool_decisions,
            ..McpPolicy::new()
        };

        assert_eq!(
            policy.evaluate("github", Some("github__delete_repo")),
            ToolDecision::Block
        );
        assert_eq!(
            policy.evaluate("github", Some("github__admin_access")),
            ToolDecision::Warn
        );
        assert_eq!(
            policy.evaluate("github", Some("github__search")),
            ToolDecision::Allow
        );
    }

    #[test]
    fn tool_decision_roundtrip() {
        for d in [ToolDecision::Allow, ToolDecision::Warn, ToolDecision::Block] {
            assert_eq!(ToolDecision::parse_str(d.as_str()), d);
        }
    }

    #[test]
    fn tool_decision_log_strings() {
        assert_eq!(ToolDecision::Allow.to_log_decision(), "allowed");
        assert_eq!(ToolDecision::Warn.to_log_decision(), "warned");
        assert_eq!(ToolDecision::Block.to_log_decision(), "denied");
    }

    #[test]
    fn default_tool_decision_respected() {
        let policy = McpPolicy {
            default_tool_decision: ToolDecision::Warn,
            ..McpPolicy::new()
        };
        assert_eq!(
            policy.evaluate("github", Some("github__any_tool")),
            ToolDecision::Warn
        );
    }

    // ── McpUserConfig tests ──────────────────────────────────────────

    #[test]
    fn mcp_user_config_default() {
        let cfg = McpUserConfig::default();
        assert!(cfg.global_policy.is_none());
        assert!(cfg.default_tool_permission.is_none());
        assert!(cfg.servers.is_empty());
        assert!(cfg.server_enabled.is_empty());
        assert!(cfg.tool_permissions.is_empty());
        assert!(!cfg.is_globally_blocked());
    }

    #[test]
    fn mcp_user_config_serde_roundtrip() {
        let cfg = McpUserConfig {
            global_policy: Some("allow".into()),
            default_tool_permission: Some(ToolDecision::Warn),
            health_check_interval_secs: Some(600),
            servers: vec![McpManualServer {
                name: "test".into(),
                url: "https://mcp.example.com/v1".into(),
                headers: HashMap::new(),
                bearer_token: Some("tok_123".into()),
                enabled: true,
            }],
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("github".into(), false);
                m
            },
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("github__delete_repo".into(), ToolDecision::Block);
                m
            },
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let decoded: McpUserConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg, decoded);
    }

    #[test]
    fn mcp_user_config_backward_compat() {
        // Parse empty TOML -> defaults
        let cfg: McpUserConfig = toml::from_str("").unwrap();
        assert!(cfg.global_policy.is_none());
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn mcp_user_config_invalid_global_policy_treated_as_not_block() {
        let cfg = McpUserConfig {
            global_policy: Some("maybe".into()),
            ..Default::default()
        };
        // "maybe" is not "block", so is_globally_blocked is false
        assert!(!cfg.is_globally_blocked());
    }

    // ── to_policy() multi-layer tests ────────────────────────────────

    #[test]
    fn to_policy_global_block_blocks_all() {
        let user = McpUserConfig {
            global_policy: Some("block".into()),
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("any", Some("any__tool")), ToolDecision::Block);
    }

    #[test]
    fn to_policy_corp_global_block_overrides_user_allow() {
        let user = McpUserConfig {
            global_policy: Some("allow".into()),
            ..Default::default()
        };
        let corp = McpUserConfig {
            global_policy: Some("block".into()),
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("github", Some("github__search")), ToolDecision::Block);
    }

    #[test]
    fn to_policy_server_disabled_blocks_its_tools() {
        let user = McpUserConfig {
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("evil".into(), false);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("evil", Some("evil__do_stuff")), ToolDecision::Block);
        assert_eq!(policy.evaluate("github", Some("github__search")), ToolDecision::Allow);
    }

    #[test]
    fn to_policy_per_tool_override() {
        let user = McpUserConfig {
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("github__delete_repo".into(), ToolDecision::Block);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig::default();
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("github", Some("github__delete_repo")), ToolDecision::Block);
        assert_eq!(policy.evaluate("github", Some("github__search")), ToolDecision::Allow);
    }

    #[test]
    fn to_policy_corp_tool_overrides_user_tool() {
        let user = McpUserConfig {
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("github__search".into(), ToolDecision::Allow);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig {
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("github__search".into(), ToolDecision::Block);
                m
            },
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("github", Some("github__search")), ToolDecision::Block);
    }

    #[test]
    fn to_policy_corp_server_enabled_overrides_user() {
        let user = McpUserConfig {
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("github".into(), true);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig {
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("github".into(), false);
                m
            },
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("github", None), ToolDecision::Block);
    }

    #[test]
    fn to_policy_empty_config_allows_all() {
        let user = McpUserConfig::default();
        let corp = McpUserConfig::default();
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("any", Some("any__tool")), ToolDecision::Allow);
    }

    #[test]
    fn to_policy_all_layers_block() {
        let user = McpUserConfig {
            global_policy: Some("block".into()),
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("evil".into(), false);
                m
            },
            tool_permissions: {
                let mut m = HashMap::new();
                m.insert("evil__tool".into(), ToolDecision::Block);
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig {
            global_policy: Some("block".into()),
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("evil", Some("evil__tool")), ToolDecision::Block);
    }

    #[test]
    fn user_cannot_re_enable_corp_blocked_server() {
        let user = McpUserConfig {
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("evil".into(), true); // user wants it enabled
                m
            },
            ..Default::default()
        };
        let corp = McpUserConfig {
            server_enabled: {
                let mut m = HashMap::new();
                m.insert("evil".into(), false); // corp says no
                m
            },
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        // Corp block is final
        assert_eq!(policy.evaluate("evil", None), ToolDecision::Block);
    }

    #[test]
    fn corp_default_permission_overrides_user() {
        let user = McpUserConfig {
            default_tool_permission: Some(ToolDecision::Allow),
            ..Default::default()
        };
        let corp = McpUserConfig {
            default_tool_permission: Some(ToolDecision::Warn),
            ..Default::default()
        };
        let policy = user.to_policy(&corp);
        assert_eq!(policy.evaluate("any", Some("any__unknown_tool")), ToolDecision::Warn);
    }
}
