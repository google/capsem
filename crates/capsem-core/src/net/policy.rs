//! Network policy mechanics: derived domain metadata, body capture settings,
//! plain-HTTP port mechanics, and DNS-level redirects.
//!
//! `DnsRedirect` rules let an admin override DNS resolution for a
//! specific qname (and optionally qtype) -- useful for redirecting
//! telemetry domains to a local trap, simulating a domain that would
//! otherwise need real internet, or pinning a name to a known IP for
//! deterministic test runs. The DNS handler checks security-rule
//! enforcement before redirects, then applies redirects before the
//! upstream forward.

use std::net::IpAddr;

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
            DomainMatcher::Wildcard(suffix) => domain.ends_with(&format!(".{suffix}")),
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

/// A DNS-level redirect rule (T3.d). When the DNS handler sees a
/// query whose qname matches `matcher` and (if set) whose qtype
/// matches `qtype`, the answer is synthesized locally from `answers`
/// + `ttl` instead of being forwarded to the upstream resolver.
///
/// `qtype = None` means "any qtype" -- e.g. a redirect with
/// `answers = [10.20.30.40]` and `qtype = None` will answer A queries
/// with that IP and AAAA queries with NoError + zero answers (no
/// matching record), which is the standard "this name exists but has
/// no record of the type you asked for" DNS shape.
#[derive(Debug, Clone)]
pub struct DnsRedirect {
    pub matcher: DomainMatcher,
    /// `Some(rfc_qtype)` to restrict the redirect to one record type
    /// (1 = A, 28 = AAAA, ...). `None` matches any qtype.
    pub qtype: Option<u16>,
    /// IP addresses to return in the synthetic answer. Empty list
    /// means "the rule matches but there's no IP to give back" --
    /// used to spoof "name exists, no record" via a NoError + zero
    /// answers response.
    pub answers: Vec<IpAddr>,
    /// TTL to advertise in the synthetic answer, in seconds. Use a
    /// short TTL (e.g. 60) so the guest's resolver re-queries
    /// promptly when the policy is edited.
    pub ttl: u32,
}

impl DnsRedirect {
    /// Convenience: build an A/AAAA redirect for a domain pattern.
    /// `qtype = None` means the redirect applies to any qtype.
    pub fn new(pattern: &str, qtype: Option<u16>, answers: Vec<IpAddr>, ttl: u32) -> Self {
        Self {
            matcher: DomainMatcher::parse(pattern),
            qtype,
            answers,
            ttl,
        }
    }
}

/// Network mechanics derived from profile/corp config.
///
/// Security decisions live in the security-rule engine. This type must not
/// carry allow/ask/block/default semantics.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    /// Whether to log request/response body previews.
    pub log_bodies: bool,
    /// Maximum bytes of body preview to capture in telemetry.
    pub max_body_capture: usize,
    /// Plain-HTTP upstream port allowlist (T2.2). Plain-HTTP requests
    /// whose Host header carries a port not on this list are denied
    /// before the upstream dial. Default: `[80]`. Extend for Ollama
    /// (11434) or other local-LLM servers via config / dev defaults.
    pub http_upstream_ports: Vec<u16>,
    /// DNS redirect rules (T3.d). Evaluated in order, first match wins after
    /// security-rule enforcement has allowed the query. Empty by default.
    pub dns_redirects: Vec<DnsRedirect>,
}

/// Default max body capture size (4 KB).
const DEFAULT_MAX_BODY_CAPTURE: usize = 4096;

/// Default plain-HTTP upstream port allowlist. Pre-T2.2 behavior was
/// "no plain HTTP at all". Post-T2.2 defaults match the guest-side
/// iptables redirect list in `capsem-init`: port 80 (generic plain
/// HTTP) plus 11434 (Ollama default; the canonical local-LLM
/// workflow this protocol path was designed for). Adding a new port
/// to this list and to the iptables redirects in tandem is the
/// "configurable allowlist" promise from the T2.2 plan; a config
/// plumb to `policy_config` is the final form (deferred follow-up).
const DEFAULT_HTTP_UPSTREAM_PORTS: &[u16] = &[80, 11434];

impl NetworkPolicy {
    /// Create network mechanics with default capture and upstream-port settings.
    pub fn new() -> Self {
        Self {
            log_bodies: true,
            max_body_capture: DEFAULT_MAX_BODY_CAPTURE,
            http_upstream_ports: DEFAULT_HTTP_UPSTREAM_PORTS.to_vec(),
            dns_redirects: Vec::new(),
        }
    }

    /// Find the first matching DNS redirect for `(qname, qtype)`.
    /// Returns `None` if no redirect rule matches.
    ///
    /// A rule with `qtype = None` matches any qtype. A rule with
    /// `qtype = Some(t)` matches only when `t == qtype`. The qname
    /// match honors `DomainMatcher` semantics (exact / wildcard).
    /// First match wins; admins order their rules.
    pub fn find_dns_redirect(&self, qname: &str, qtype: u16) -> Option<&DnsRedirect> {
        self.dns_redirects
            .iter()
            .find(|r| r.matcher.matches(qname) && r.qtype.is_none_or(|t| t == qtype))
    }

    /// Create a policy with hardcoded defaults for development.
    pub fn default_dev() -> Self {
        Self::new()
    }
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_policy() -> NetworkPolicy {
        NetworkPolicy::default_dev()
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

    // -- log_bodies default --

    #[test]
    fn log_bodies_default_true() {
        let policy = dev_policy();
        assert!(policy.log_bodies);
    }

    // =====================================================================
    // (T3.d) -- DnsRedirect rule tests
    // =====================================================================

    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn redirect(pattern: &str, qtype: Option<u16>, ips: Vec<IpAddr>) -> DnsRedirect {
        DnsRedirect::new(pattern, qtype, ips, 60)
    }

    #[test]
    fn find_redirect_exact_match_a_qtype() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "anthropic.com",
            Some(1),
            vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
        ));
        let r = p.find_dns_redirect("anthropic.com", 1).unwrap();
        assert_eq!(r.matcher.pattern_str(), "anthropic.com");
        assert_eq!(r.answers.len(), 1);
        assert_eq!(r.ttl, 60);
    }

    #[test]
    fn find_redirect_qtype_filter_misses() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "anthropic.com",
            Some(1), // A only
            vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
        ));
        // AAAA query (qtype=28) on the same name -- no match.
        assert!(p.find_dns_redirect("anthropic.com", 28).is_none());
    }

    #[test]
    fn find_redirect_any_qtype_matches_aaaa() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "anthropic.com",
            None, // any qtype
            vec![IpAddr::V6(Ipv6Addr::LOCALHOST)],
        ));
        let r_a = p.find_dns_redirect("anthropic.com", 1).unwrap();
        assert!(r_a.qtype.is_none());
        let r_aaaa = p.find_dns_redirect("anthropic.com", 28).unwrap();
        assert!(r_aaaa.qtype.is_none());
    }

    #[test]
    fn find_redirect_wildcard_subdomain_match() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "*.openai.com",
            None,
            vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
        ));
        assert!(p.find_dns_redirect("api.openai.com", 1).is_some());
        assert!(p.find_dns_redirect("foo.openai.com", 28).is_some());
        // Wildcard does NOT match the base.
        assert!(p.find_dns_redirect("openai.com", 1).is_none());
    }

    #[test]
    fn find_redirect_first_match_wins() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "anthropic.com",
            None,
            vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
        ));
        p.dns_redirects.push(redirect(
            "anthropic.com",
            None,
            vec![IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2))],
        ));
        let r = p.find_dns_redirect("anthropic.com", 1).unwrap();
        assert_eq!(r.answers, vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]);
    }

    #[test]
    fn find_redirect_no_match_returns_none() {
        let mut p = NetworkPolicy::new();
        p.dns_redirects.push(redirect(
            "anthropic.com",
            Some(1),
            vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        ));
        assert!(p.find_dns_redirect("example.com", 1).is_none());
    }

    #[test]
    fn find_redirect_empty_list_returns_none() {
        let p = NetworkPolicy::new();
        assert!(p.find_dns_redirect("anything.com", 1).is_none());
    }

    #[test]
    fn dns_redirects_default_empty() {
        let p = NetworkPolicy::new();
        assert!(p.dns_redirects.is_empty());
        let p2 = NetworkPolicy::default_dev();
        assert!(p2.dns_redirects.is_empty());
    }

    #[test]
    fn dns_redirect_empty_answers_is_legal() {
        // Empty `answers` is the "name exists, no record of that
        // type" signal -- still a valid policy entry.
        let mut p = NetworkPolicy::new();
        p.dns_redirects
            .push(redirect("nodata.example.com", None, vec![]));
        let r = p.find_dns_redirect("nodata.example.com", 1).unwrap();
        assert!(r.answers.is_empty());
    }
}
