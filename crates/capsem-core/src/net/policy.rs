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

use std::collections::BTreeMap;
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

/// Upstream transport used after a routing override chooses the dial target.
///
/// This is network routing only: security decisions still evaluate the
/// original observed host/port/path before any upstream dial happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamOverrideProtocol {
    /// Dial the override target as plain HTTP/1.1.
    Http,
    /// Dial the override target with TLS.
    Tls,
}

/// Exact upstream routing override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamOverride {
    pub dial: String,
    pub protocol: UpstreamOverrideProtocol,
}

/// Network mechanics derived from profile/corp config.
///
/// Security decisions live in the security-rule engine. This type must not
/// carry allow/ask/block/default semantics.
#[derive(Debug, Clone)]
pub struct NetworkMechanics {
    /// Whether to log request/response body previews.
    pub log_bodies: bool,
    /// Maximum bytes of body preview to capture in telemetry.
    pub max_body_capture: usize,
    /// Plain-HTTP upstream port allowlist (T2.2). Plain-HTTP requests
    /// whose Host header carries a port not on this list are denied
    /// before the upstream dial. Defaults include generic HTTP, common
    /// local proxy/dev ports, the doctor fixture port, and Ollama.
    pub http_upstream_ports: Vec<u16>,
    /// DNS redirect rules (T3.d). Evaluated in order, first match wins after
    /// security-rule enforcement has allowed the query. Empty by default.
    pub dns_redirects: Vec<DnsRedirect>,
    /// Exact upstream dial overrides keyed by `host:port`.
    ///
    /// Used for corp/dev controlled routing such as hermetic replay. It must
    /// not change the event host/port observed by CEL or the ledger.
    pub upstream_overrides: BTreeMap<String, UpstreamOverride>,
}

/// Default max body capture size (4 KB).
const DEFAULT_MAX_BODY_CAPTURE: usize = 4096;

/// Default plain-HTTP upstream port allowlist. Pre-T2.2 behavior was
/// "no plain HTTP at all". Post-T2.2 defaults match the guest-side
/// iptables redirect list in `capsem-init`: port 80 (generic plain
/// HTTP), common HTTP proxy/dev ports 3128 and 8080, the deterministic
/// local mock-server fixture port 3713, and 11434 (Ollama default;
/// the canonical local-LLM workflow this protocol path was designed
/// for). Adding a new port to this list and to the iptables redirects
/// in tandem is the configurable allowlist promise from the T2.2 plan.
const DEFAULT_HTTP_UPSTREAM_PORTS: &[u16] = &[80, 3128, 3713, 8080, 11434];

impl NetworkMechanics {
    /// Create network mechanics with default capture and upstream-port settings.
    pub fn new() -> Self {
        Self {
            log_bodies: true,
            max_body_capture: DEFAULT_MAX_BODY_CAPTURE,
            http_upstream_ports: DEFAULT_HTTP_UPSTREAM_PORTS.to_vec(),
            dns_redirects: Vec::new(),
            upstream_overrides: BTreeMap::new(),
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

    /// Find an exact upstream override for `(domain, port)`.
    pub fn find_upstream_override(&self, domain: &str, port: u16) -> Option<&UpstreamOverride> {
        self.upstream_overrides
            .get(&format!("{}:{port}", domain.to_lowercase()))
    }

    /// Create a policy with hardcoded defaults for development.
    pub fn default_dev() -> Self {
        Self::new()
    }
}

impl Default for NetworkMechanics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
