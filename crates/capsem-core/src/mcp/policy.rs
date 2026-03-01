use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
}
