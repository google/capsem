/// HTTP-level policy engine: extends domain-level policy with method+path rules.
///
/// Evaluation order:
/// 1. Domain check via `DomainPolicy` (early reject before TLS handshake)
/// 2. HTTP rules for the domain (method + path pattern matching)
/// 3. If no rules match for an allowed domain, allow (backward compat)
use super::domain_policy::{Action, DomainPolicy};

/// A single HTTP-level rule for a domain.
#[derive(Debug, Clone)]
pub struct HttpRule {
    /// Domain this rule applies to (exact match, lowercase).
    pub domain: String,
    /// HTTP method to match: "GET", "POST", etc. or "*" for any.
    pub method: String,
    /// Path pattern: exact match or prefix wildcard (e.g., "/api/v1/*").
    pub path_pattern: String,
    /// Action to take when this rule matches.
    pub action: Action,
}

/// The result of an HTTP policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpPolicyDecision {
    pub action: Action,
    pub reason: String,
    /// Which stage made the decision: "domain" or "http-rule".
    pub stage: &'static str,
}

/// Combined domain + HTTP-level policy engine.
#[derive(Debug, Clone)]
pub struct HttpPolicy {
    domain_policy: DomainPolicy,
    rules: Vec<HttpRule>,
    /// Whether to log request/response bodies.
    pub log_bodies: bool,
    /// Maximum bytes of body to capture in telemetry.
    pub max_body_capture: usize,
}

/// Default max body capture size (4 KB).
const DEFAULT_MAX_BODY_CAPTURE: usize = 4096;

impl HttpPolicy {
    /// Create an HttpPolicy from a DomainPolicy with no HTTP rules (backward compat).
    pub fn from_domain_policy(dp: DomainPolicy) -> Self {
        Self {
            domain_policy: dp,
            rules: Vec::new(),
            log_bodies: false,
            max_body_capture: DEFAULT_MAX_BODY_CAPTURE,
        }
    }

    /// Create an HttpPolicy with domain policy and HTTP rules.
    pub fn new(
        dp: DomainPolicy,
        rules: Vec<HttpRule>,
        log_bodies: bool,
        max_body_capture: usize,
    ) -> Self {
        Self {
            domain_policy: dp,
            rules,
            log_bodies,
            max_body_capture,
        }
    }

    /// Evaluate at the domain level only (pre-TLS, before handshake).
    ///
    /// This is the fast path for early rejection of blocked domains.
    pub fn evaluate_domain(&self, domain: &str) -> HttpPolicyDecision {
        let (action, reason) = self.domain_policy.evaluate(domain);
        HttpPolicyDecision {
            action,
            reason: reason.to_string(),
            stage: "domain",
        }
    }

    /// Evaluate a full HTTP request: domain first, then HTTP rules.
    ///
    /// If the domain is denied, returns immediately (no HTTP check).
    /// If allowed at domain level and no HTTP rules exist for this domain,
    /// allows the request (backward compat).
    pub fn evaluate_request(
        &self,
        domain: &str,
        method: &str,
        path: &str,
    ) -> HttpPolicyDecision {
        // 1. Domain-level check first.
        let domain_decision = self.evaluate_domain(domain);
        if domain_decision.action == Action::Deny {
            return domain_decision;
        }

        // 2. Find HTTP rules for this domain.
        let domain_lower = domain.to_lowercase();
        let domain_rules: Vec<&HttpRule> = self
            .rules
            .iter()
            .filter(|r| r.domain == domain_lower)
            .collect();

        // No rules for this domain = allow all (backward compat).
        if domain_rules.is_empty() {
            return domain_decision;
        }

        // 3. Check HTTP rules.
        let method_upper = method.to_uppercase();
        for rule in &domain_rules {
            if matches_method(&rule.method, &method_upper)
                && matches_path(&rule.path_pattern, path)
            {
                return HttpPolicyDecision {
                    action: rule.action,
                    reason: format!(
                        "http-rule: {} {} -> {:?}",
                        rule.method, rule.path_pattern, rule.action
                    ),
                    stage: "http-rule",
                };
            }
        }

        // No matching rule = allow (domain was already allowed).
        domain_decision
    }

    /// Access the underlying domain policy (for pattern listing etc.).
    pub fn domain_policy(&self) -> &DomainPolicy {
        &self.domain_policy
    }
}

/// Check if a method rule matches the request method.
/// "*" matches any method.
fn matches_method(rule_method: &str, request_method: &str) -> bool {
    rule_method == "*" || rule_method.to_uppercase() == request_method
}

/// Check if a path pattern matches the request path.
/// - Exact match: "/api/v1/users" matches "/api/v1/users"
/// - Prefix wildcard: "/api/v1/*" matches "/api/v1/users" and "/api/v1/repos/foo"
fn matches_path(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        path == prefix || path.starts_with(&format!("{prefix}/"))
    } else if pattern == "*" {
        true
    } else {
        pattern == path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_policy() -> DomainPolicy {
        DomainPolicy::default_dev()
    }

    fn policy_with_rules(rules: Vec<HttpRule>) -> HttpPolicy {
        HttpPolicy::new(dev_policy(), rules, false, DEFAULT_MAX_BODY_CAPTURE)
    }

    // -- Domain-level tests --

    #[test]
    fn domain_deny_short_circuits() {
        let policy = HttpPolicy::from_domain_policy(dev_policy());
        let decision = policy.evaluate_request("evil.example.com", "GET", "/anything");
        assert_eq!(decision.action, Action::Deny);
        assert_eq!(decision.stage, "domain");
    }

    #[test]
    fn allowed_domain_no_rules_permits_all() {
        let policy = HttpPolicy::from_domain_policy(dev_policy());
        let decision = policy.evaluate_request("github.com", "POST", "/anything");
        assert_eq!(decision.action, Action::Allow);
        assert_eq!(decision.stage, "domain");
    }

    // -- HTTP rule tests --

    #[test]
    fn path_rule_blocks_post() {
        let rules = vec![
            HttpRule {
                domain: "github.com".into(),
                method: "POST".into(),
                path_pattern: "/repos/*".into(),
                action: Action::Deny,
            },
        ];
        let policy = policy_with_rules(rules);

        // POST to /repos/foo -> denied by rule
        let decision = policy.evaluate_request("github.com", "POST", "/repos/foo");
        assert_eq!(decision.action, Action::Deny);
        assert_eq!(decision.stage, "http-rule");

        // GET to /repos/foo -> no matching rule -> allowed by domain
        let decision = policy.evaluate_request("github.com", "GET", "/repos/foo");
        assert_eq!(decision.action, Action::Allow);
        assert_eq!(decision.stage, "domain");
    }

    #[test]
    fn path_wildcard_matches_prefix() {
        let rules = vec![HttpRule {
            domain: "github.com".into(),
            method: "*".into(),
            path_pattern: "/api/v1/*".into(),
            action: Action::Deny,
        }];
        let policy = policy_with_rules(rules);

        assert_eq!(
            policy.evaluate_request("github.com", "GET", "/api/v1/users").action,
            Action::Deny
        );
        assert_eq!(
            policy.evaluate_request("github.com", "GET", "/api/v1/repos/foo/bar").action,
            Action::Deny
        );
        // Exact prefix match (without trailing slash) should also match
        assert_eq!(
            policy.evaluate_request("github.com", "GET", "/api/v1").action,
            Action::Deny
        );
        // Different path -> allowed
        assert_eq!(
            policy.evaluate_request("github.com", "GET", "/api/v2/users").action,
            Action::Allow
        );
    }

    #[test]
    fn method_star_matches_any() {
        let rules = vec![HttpRule {
            domain: "github.com".into(),
            method: "*".into(),
            path_pattern: "/admin".into(),
            action: Action::Deny,
        }];
        let policy = policy_with_rules(rules);

        for method in &["GET", "POST", "PUT", "DELETE", "PATCH"] {
            assert_eq!(
                policy.evaluate_request("github.com", method, "/admin").action,
                Action::Deny,
                "{method} /admin should be denied"
            );
        }
    }

    #[test]
    fn exact_path_match() {
        let rules = vec![HttpRule {
            domain: "github.com".into(),
            method: "DELETE".into(),
            path_pattern: "/repos/owner/repo".into(),
            action: Action::Deny,
        }];
        let policy = policy_with_rules(rules);

        assert_eq!(
            policy.evaluate_request("github.com", "DELETE", "/repos/owner/repo").action,
            Action::Deny
        );
        // Sub-path should NOT match exact pattern
        assert_eq!(
            policy.evaluate_request("github.com", "DELETE", "/repos/owner/repo/issues").action,
            Action::Allow
        );
    }

    #[test]
    fn from_domain_policy_backward_compat() {
        let policy = HttpPolicy::from_domain_policy(dev_policy());
        assert!(!policy.log_bodies);
        assert_eq!(policy.max_body_capture, DEFAULT_MAX_BODY_CAPTURE);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn evaluate_domain_only() {
        let policy = HttpPolicy::from_domain_policy(dev_policy());
        let d = policy.evaluate_domain("github.com");
        assert_eq!(d.action, Action::Allow);
        assert_eq!(d.stage, "domain");

        let d = policy.evaluate_domain("evil.com");
        assert_eq!(d.action, Action::Deny);
        assert_eq!(d.stage, "domain");
    }

    #[test]
    fn rules_for_different_domain_dont_apply() {
        let rules = vec![HttpRule {
            domain: "example.com".into(),
            method: "*".into(),
            path_pattern: "*".into(),
            action: Action::Deny,
        }];
        let policy = policy_with_rules(rules);
        // github.com has no rules -> allowed by domain
        assert_eq!(
            policy.evaluate_request("github.com", "GET", "/").action,
            Action::Allow
        );
    }
}
