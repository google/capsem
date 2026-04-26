//! Network policy engine: per-domain read/write verb control.
//!
//! Each rule matches a domain pattern and specifies whether read methods
//! (GET, HEAD, OPTIONS) and write methods (POST, PUT, DELETE, PATCH) are
//! allowed. Rules are evaluated in order; first match wins. If no rule
//! matches, the default applies.

/// How a domain pattern matches incoming requests.
#[derive(Debug, Clone)]
pub enum DomainMatcher {
    /// Exact domain match (case-insensitive): "github.com"
    Exact(String),
    /// Wildcard: "*.github.com" matches subdomains but NOT the base domain.
    Wildcard(String),
}

impl DomainMatcher {
    /// Parse a pattern string into a matcher.
    /// Patterns starting with `*.` become wildcards; all others are exact.
    pub fn parse(pattern: &str) -> Self {
        let lower = pattern.to_lowercase();
        if let Some(suffix) = lower.strip_prefix("*.") {
            DomainMatcher::Wildcard(suffix.to_string())
        } else {
            DomainMatcher::Exact(lower)
        }
    }

    /// Check if a domain matches this pattern.
    pub fn matches(&self, domain: &str) -> bool {
        let domain = domain.to_lowercase();
        match self {
            DomainMatcher::Exact(exact) => domain == *exact,
            DomainMatcher::Wildcard(suffix) => {
                domain.ends_with(&format!(".{suffix}"))
            }
        }
    }

    /// Return the pattern string for display (e.g., in matched_rule).
    pub fn pattern_str(&self) -> String {
        match self {
            DomainMatcher::Exact(s) => s.clone(),
            DomainMatcher::Wildcard(s) => format!("*.{s}"),
        }
    }
}

/// A single policy rule: domain pattern + read/write permissions.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub matcher: DomainMatcher,
    /// Allow read methods (GET, HEAD, OPTIONS).
    pub allow_read: bool,
    /// Allow write methods (POST, PUT, DELETE, PATCH).
    pub allow_write: bool,
}

/// The result of evaluating a request against the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// The rule pattern that matched (e.g., "*.github.com" or "default").
    pub matched_rule: String,
    /// Human-readable reason (e.g., "write denied by rule api.openai.com").
    pub reason: String,
}

/// Network policy: per-domain read/write verb control with defaults.
///
/// Rules are evaluated in order; first match wins.
/// If no rule matches, the default read/write permissions apply.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    pub rules: Vec<PolicyRule>,
    /// Allow read methods (GET, HEAD, OPTIONS) by default.
    pub default_allow_read: bool,
    /// Allow write methods (POST, PUT, DELETE, PATCH) by default.
    pub default_allow_write: bool,
    /// Whether to log request/response body previews.
    pub log_bodies: bool,
    /// Maximum bytes of body preview to capture in telemetry.
    pub max_body_capture: usize,
    /// Upstream aliases for local routing (e.g. "ollama.local" -> "http://127.0.0.1:11434")
    pub host_aliases: std::collections::HashMap<String, String>,
}

/// Default max body capture size (4 KB).
const DEFAULT_MAX_BODY_CAPTURE: usize = 4096;

impl NetworkPolicy {
    /// Create a policy with explicit rules and defaults.
    pub fn new(
        rules: Vec<PolicyRule>,
        default_allow_read: bool,
        default_allow_write: bool,
    ) -> Self {
        Self {
            rules,
            default_allow_read,
            default_allow_write,
            log_bodies: true,
            max_body_capture: DEFAULT_MAX_BODY_CAPTURE,
            host_aliases: std::collections::HashMap::new(),
        }
    }

    /// Create a policy with hardcoded defaults for development.
    pub fn default_dev() -> Self {
        let rules = vec![
            // Blocked: AI providers (all verbs)
            rule("api.openai.com", false, false),
            rule("api.anthropic.com", false, false),
            // Full access: code hosting
            rule("github.com", true, true),
            rule("*.github.com", true, true),
            rule("*.githubusercontent.com", true, true),
            // Read-only: package registries
            rule("registry.npmjs.org", true, false),
            rule("*.npmjs.org", true, false),
            rule("pypi.org", true, false),
            rule("files.pythonhosted.org", true, false),
            rule("crates.io", true, false),
            rule("static.crates.io", true, false),
            // Read-only: OS packages
            rule("deb.debian.org", true, false),
            rule("security.debian.org", true, false),
            // Full access: Gemini (testing)
            rule("generativelanguage.googleapis.com", true, true),
            // Full access: dev
            rule("elie.net", true, true),
            rule("*.elie.net", true, true),
        ];
        Self::new(rules, true, false)
    }

    /// Evaluate a request against the policy.
    ///
    /// Classifies the method as read (GET, HEAD, OPTIONS) or write
    /// (POST, PUT, DELETE, PATCH, etc.), then checks rules in order.
    pub fn evaluate(&self, domain: &str, method: &str) -> PolicyDecision {
        let is_read = is_read_method(method);

        for rule in &self.rules {
            if rule.matcher.matches(domain) {
                let pattern = rule.matcher.pattern_str();
                let allowed = if is_read {
                    rule.allow_read
                } else {
                    rule.allow_write
                };
                let verb_class = if is_read { "read" } else { "write" };
                let action = if allowed { "allowed" } else { "denied" };
                return PolicyDecision {
                    allowed,
                    matched_rule: pattern.clone(),
                    reason: format!("{verb_class} {action} by rule {pattern}"),
                };
            }
        }

        // No rule matched -- use defaults.
        let allowed = if is_read {
            self.default_allow_read
        } else {
            self.default_allow_write
        };
        let verb_class = if is_read { "read" } else { "write" };
        let action = if allowed { "allowed" } else { "denied" };
        PolicyDecision {
            allowed,
            matched_rule: "default".to_string(),
            reason: format!("{verb_class} {action} by default policy"),
        }
    }

    /// Check if a domain is fully blocked (both read and write denied).
    ///
    /// Used to decide whether to proceed with TLS handshake at all.
    /// If a domain is fully blocked, we can skip the expensive cert minting.
    pub fn is_fully_blocked(&self, domain: &str) -> Option<String> {
        for rule in &self.rules {
            if rule.matcher.matches(domain) {
                if !rule.allow_read && !rule.allow_write {
                    return Some(rule.matcher.pattern_str());
                }
                return None;
            }
        }
        if !self.default_allow_read && !self.default_allow_write {
            return Some("default".to_string());
        }
        None
    }
}

/// Classify a method as "read" (safe, idempotent).
fn is_read_method(method: &str) -> bool {
    matches!(
        method.to_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

/// Helper to build a rule from a pattern string.
fn rule(pattern: &str, allow_read: bool, allow_write: bool) -> PolicyRule {
    PolicyRule {
        matcher: DomainMatcher::parse(pattern),
        allow_read,
        allow_write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_policy() -> NetworkPolicy {
        NetworkPolicy::default_dev()
    }

    // -- Read access --

    #[test]
    fn get_to_github_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("github.com", "GET");
        assert!(d.allowed);
        assert_eq!(d.matched_rule, "github.com");
    }

    #[test]
    fn get_to_unknown_domain_allowed_by_default() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "GET");
        assert!(d.allowed);
        assert_eq!(d.matched_rule, "default");
        assert!(d.reason.contains("read allowed by default"));
    }

    #[test]
    fn head_is_read() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "HEAD");
        assert!(d.allowed);
    }

    #[test]
    fn options_is_read() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "OPTIONS");
        assert!(d.allowed);
    }

    // -- Write access --

    #[test]
    fn post_to_github_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("github.com", "POST");
        assert!(d.allowed);
        assert_eq!(d.matched_rule, "github.com");
    }

    #[test]
    fn post_to_unknown_domain_denied_by_default() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "POST");
        assert!(!d.allowed);
        assert_eq!(d.matched_rule, "default");
        assert!(d.reason.contains("write denied by default"));
    }

    #[test]
    fn put_is_write() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "PUT");
        assert!(!d.allowed);
    }

    #[test]
    fn delete_is_write() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "DELETE");
        assert!(!d.allowed);
    }

    #[test]
    fn patch_is_write() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "PATCH");
        assert!(!d.allowed);
    }

    // -- Blocked domains --

    #[test]
    fn openai_fully_blocked() {
        let policy = dev_policy();
        let d = policy.evaluate("api.openai.com", "GET");
        assert!(!d.allowed);
        assert_eq!(d.matched_rule, "api.openai.com");
        assert!(d.reason.contains("denied"));
    }

    #[test]
    fn openai_post_blocked() {
        let policy = dev_policy();
        let d = policy.evaluate("api.openai.com", "POST");
        assert!(!d.allowed);
    }

    #[test]
    fn anthropic_fully_blocked() {
        let policy = dev_policy();
        let d = policy.evaluate("api.anthropic.com", "GET");
        assert!(!d.allowed);
    }

    // -- Gemini allowed --

    #[test]
    fn gemini_get_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("generativelanguage.googleapis.com", "GET");
        assert!(d.allowed);
    }

    #[test]
    fn gemini_post_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("generativelanguage.googleapis.com", "POST");
        assert!(d.allowed);
    }

    // -- Wildcards --

    #[test]
    fn wildcard_subdomain_match() {
        let policy = dev_policy();
        let d = policy.evaluate("api.github.com", "GET");
        assert!(d.allowed);
        assert_eq!(d.matched_rule, "*.github.com");
    }

    #[test]
    fn wildcard_does_not_match_base() {
        let policy = NetworkPolicy::new(
            vec![rule("*.example.com", true, false)],
            false,
            false,
        );
        let d = policy.evaluate("example.com", "GET");
        assert!(!d.allowed);
        assert_eq!(d.matched_rule, "default");
    }

    #[test]
    fn deep_subdomain_matches_wildcard() {
        let policy = dev_policy();
        let d = policy.evaluate("raw.githubusercontent.com", "GET");
        assert!(d.allowed);
    }

    // -- First match wins --

    #[test]
    fn first_match_wins() {
        let policy = NetworkPolicy::new(
            vec![
                rule("example.com", false, false), // block
                rule("example.com", true, true),    // allow (never reached)
            ],
            true,
            true,
        );
        let d = policy.evaluate("example.com", "GET");
        assert!(!d.allowed);
    }

    // -- Case insensitivity --

    #[test]
    fn case_insensitive_domain() {
        let policy = dev_policy();
        let d = policy.evaluate("GitHub.COM", "GET");
        assert!(d.allowed);
    }

    #[test]
    fn case_insensitive_method() {
        let policy = dev_policy();
        let d = policy.evaluate("example.com", "get");
        assert!(d.allowed);
    }

    // -- Read-only package registries --

    #[test]
    fn pypi_get_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("pypi.org", "GET");
        assert!(d.allowed);
    }

    #[test]
    fn pypi_post_denied() {
        let policy = dev_policy();
        let d = policy.evaluate("pypi.org", "POST");
        assert!(!d.allowed);
        assert_eq!(d.matched_rule, "pypi.org");
    }

    #[test]
    fn crates_io_get_allowed() {
        let policy = dev_policy();
        let d = policy.evaluate("crates.io", "GET");
        assert!(d.allowed);
    }

    #[test]
    fn crates_io_post_denied() {
        let policy = dev_policy();
        let d = policy.evaluate("crates.io", "POST");
        assert!(!d.allowed);
    }

    // -- is_fully_blocked --

    #[test]
    fn openai_is_fully_blocked() {
        let policy = dev_policy();
        assert!(policy.is_fully_blocked("api.openai.com").is_some());
    }

    #[test]
    fn github_not_fully_blocked() {
        let policy = dev_policy();
        assert!(policy.is_fully_blocked("github.com").is_none());
    }

    #[test]
    fn unknown_domain_not_fully_blocked() {
        // default_allow_read=true, so not fully blocked
        let policy = dev_policy();
        assert!(policy.is_fully_blocked("example.com").is_none());
    }

    #[test]
    fn fully_blocked_when_both_defaults_false() {
        let policy = NetworkPolicy::new(vec![], false, false);
        assert!(policy.is_fully_blocked("anything.com").is_some());
    }

    // -- Custom policy --

    #[test]
    fn custom_default_all_allowed() {
        let policy = NetworkPolicy::new(vec![], true, true);
        let d = policy.evaluate("anything.com", "POST");
        assert!(d.allowed);
    }

    #[test]
    fn custom_default_all_denied() {
        let policy = NetworkPolicy::new(vec![], false, false);
        let d = policy.evaluate("anything.com", "GET");
        assert!(!d.allowed);
    }

    // -- DomainMatcher::parse --

    #[test]
    fn parse_exact() {
        let m = DomainMatcher::parse("github.com");
        assert!(matches!(m, DomainMatcher::Exact(_)));
        assert_eq!(m.pattern_str(), "github.com");
    }

    #[test]
    fn parse_wildcard() {
        let m = DomainMatcher::parse("*.github.com");
        assert!(matches!(m, DomainMatcher::Wildcard(_)));
        assert_eq!(m.pattern_str(), "*.github.com");
    }

    #[test]
    fn parse_uppercased_normalized() {
        let m = DomainMatcher::parse("GitHub.COM");
        assert!(m.matches("github.com"));
    }

    // -- elie.net --

    #[test]
    fn elie_net_full_access() {
        let policy = dev_policy();
        assert!(policy.evaluate("elie.net", "GET").allowed);
        assert!(policy.evaluate("elie.net", "POST").allowed);
    }

    #[test]
    fn elie_subdomain_full_access() {
        let policy = dev_policy();
        assert!(policy.evaluate("blog.elie.net", "POST").allowed);
    }

    // -- log_bodies default --

    #[test]
    fn log_bodies_default_true() {
        let policy = dev_policy();
        assert!(policy.log_bodies);
    }
}
