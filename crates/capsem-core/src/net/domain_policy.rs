/// Domain policy engine: decides whether a domain is allowed or denied
/// based on allow-list, block-list, and wildcard pattern matching.

/// The result of evaluating a domain against the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
}

/// A domain matching pattern: either exact ("github.com") or wildcard ("*.github.com").
#[derive(Debug, Clone)]
struct DomainPattern {
    /// The suffix to match (e.g., "github.com" for both exact and wildcard).
    suffix: String,
    /// Whether this is a wildcard pattern (*.suffix).
    is_wildcard: bool,
}

impl DomainPattern {
    fn new(pattern: &str) -> Self {
        let pattern = pattern.to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            Self {
                suffix: suffix.to_string(),
                is_wildcard: true,
            }
        } else {
            Self {
                suffix: pattern,
                is_wildcard: false,
            }
        }
    }

    /// Check if a domain matches this pattern.
    /// Exact: "github.com" matches "github.com" only.
    /// Wildcard: "*.github.com" matches "api.github.com" but NOT "github.com".
    fn matches(&self, domain: &str) -> bool {
        if self.is_wildcard {
            // Must have at least one subdomain label before the suffix
            domain.ends_with(&format!(".{}", self.suffix))
        } else {
            domain == self.suffix
        }
    }
}

/// Domain allow/deny policy with block-before-allow semantics.
#[derive(Debug, Clone)]
pub struct DomainPolicy {
    allowed: Vec<DomainPattern>,
    blocked: Vec<DomainPattern>,
    default_action: Action,
}

impl DomainPolicy {
    /// Create a policy from allow/block lists and a default action.
    pub fn new(
        allow_patterns: &[String],
        block_patterns: &[String],
        default_action: Action,
    ) -> Self {
        Self {
            allowed: allow_patterns.iter().map(|p| DomainPattern::new(p)).collect(),
            blocked: block_patterns.iter().map(|p| DomainPattern::new(p)).collect(),
            default_action,
        }
    }

    /// Create a policy with hardcoded defaults for development use.
    pub fn default_dev() -> Self {
        let allow = default_allow_list()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let block = default_block_list()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        Self::new(&allow, &block, Action::Deny)
    }

    /// Evaluate a domain against the policy.
    /// Returns the action and a human-readable reason.
    pub fn evaluate(&self, domain: &str) -> (Action, &'static str) {
        let domain = domain.to_lowercase();

        if domain.is_empty() {
            return (Action::Deny, "empty domain");
        }

        // Block-list checked first (block takes priority over allow)
        for pattern in &self.blocked {
            if pattern.matches(&domain) {
                return (Action::Deny, "domain in block-list");
            }
        }

        // Allow-list
        for pattern in &self.allowed {
            if pattern.matches(&domain) {
                return (Action::Allow, "domain in allow-list");
            }
        }

        // Default action
        match self.default_action {
            Action::Allow => (Action::Allow, "default allow"),
            Action::Deny => (Action::Deny, "domain not in allow-list"),
        }
    }

    /// Return the list of allowed patterns (for display/logging).
    pub fn allowed_patterns(&self) -> Vec<String> {
        self.allowed
            .iter()
            .map(|p| {
                if p.is_wildcard {
                    format!("*.{}", p.suffix)
                } else {
                    p.suffix.clone()
                }
            })
            .collect()
    }

    /// Number of allow-list patterns.
    pub fn allow_count(&self) -> usize {
        self.allowed.len()
    }

    /// Number of block-list patterns.
    pub fn block_count(&self) -> usize {
        self.blocked.len()
    }

    /// Return the list of blocked patterns (for display/logging).
    pub fn blocked_patterns(&self) -> Vec<String> {
        self.blocked
            .iter()
            .map(|p| {
                if p.is_wildcard {
                    format!("*.{}", p.suffix)
                } else {
                    p.suffix.clone()
                }
            })
            .collect()
    }
}

/// Hardcoded default allow-list for development.
pub fn default_allow_list() -> &'static [&'static str] {
    &[
        "github.com",
        "*.github.com",
        "*.githubusercontent.com",
        "registry.npmjs.org",
        "*.npmjs.org",
        "pypi.org",
        "files.pythonhosted.org",
        "crates.io",
        "static.crates.io",
        "deb.debian.org",
        "security.debian.org",
        "elie.net",
        "*.elie.net",
    ]
}

/// Hardcoded default block-list (AI providers forced through audit gateway).
pub fn default_block_list() -> &'static [&'static str] {
    &[
        "api.anthropic.com",
        "api.openai.com",
        "generativelanguage.googleapis.com",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_policy() -> DomainPolicy {
        DomainPolicy::default_dev()
    }

    // -- Exact match --

    #[test]
    fn allow_exact_match() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn allow_elie_net() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("elie.net");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn allow_pypi() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("pypi.org");
        assert_eq!(action, Action::Allow);
    }

    // -- Wildcard match --

    #[test]
    fn allow_wildcard_subdomain() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("api.github.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn allow_deep_wildcard_subdomain() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("raw.githubusercontent.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn wildcard_does_not_match_base_domain() {
        // "*.github.com" should NOT match "github.com" itself
        // (github.com is allowed via exact match, not wildcard)
        let policy = DomainPolicy::new(
            &["*.example.org".to_string()],
            &[],
            Action::Deny,
        );
        let (action, _) = policy.evaluate("example.org");
        assert_eq!(action, Action::Deny);
    }

    // -- Block-list --

    #[test]
    fn block_anthropic_api() {
        let policy = dev_policy();
        let (action, reason) = policy.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
        assert_eq!(reason, "domain in block-list");
    }

    #[test]
    fn block_openai_api() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn block_google_ai_api() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn block_takes_priority_over_allow() {
        // If a domain is in both lists, block wins
        let policy = DomainPolicy::new(
            &["evil.com".to_string()],
            &["evil.com".to_string()],
            Action::Allow,
        );
        let (action, reason) = policy.evaluate("evil.com");
        assert_eq!(action, Action::Deny);
        assert_eq!(reason, "domain in block-list");
    }

    // -- Default deny --

    #[test]
    fn deny_unknown_domain() {
        let policy = dev_policy();
        let (action, reason) = policy.evaluate("example.com");
        assert_eq!(action, Action::Deny);
        assert_eq!(reason, "domain not in allow-list");
    }

    #[test]
    fn deny_rfc2606_example_net() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("example.net");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn deny_rfc2606_example_org() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("example.org");
        assert_eq!(action, Action::Deny);
    }

    // -- Case insensitivity --

    #[test]
    fn case_insensitive_match() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("GitHub.COM");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn case_insensitive_block() {
        let policy = dev_policy();
        let (action, _) = policy.evaluate("API.ANTHROPIC.COM");
        assert_eq!(action, Action::Deny);
    }

    // -- Edge cases --

    #[test]
    fn empty_domain_denied() {
        let policy = dev_policy();
        let (action, reason) = policy.evaluate("");
        assert_eq!(action, Action::Deny);
        assert_eq!(reason, "empty domain");
    }

    #[test]
    fn default_allow_policy() {
        let policy = DomainPolicy::new(&[], &[], Action::Allow);
        let (action, reason) = policy.evaluate("anything.com");
        assert_eq!(action, Action::Allow);
        assert_eq!(reason, "default allow");
    }

    #[test]
    fn empty_policy_denies_all() {
        let policy = DomainPolicy::new(&[], &[], Action::Deny);
        let (action, _) = policy.evaluate("github.com");
        assert_eq!(action, Action::Deny);
    }

    // -- Pattern list accessors --

    #[test]
    fn allowed_patterns_returned() {
        let policy = dev_policy();
        let patterns = policy.allowed_patterns();
        assert!(patterns.contains(&"github.com".to_string()));
        assert!(patterns.contains(&"*.github.com".to_string()));
    }

    #[test]
    fn blocked_patterns_returned() {
        let policy = dev_policy();
        let patterns = policy.blocked_patterns();
        assert!(patterns.contains(&"api.anthropic.com".to_string()));
    }
}
