//! Bytes-in / bytes-out DNS handler with security gating + telemetry hook.
//!
//! Receives a raw DNS query (decoded over the vsock envelope from the
//! guest agent), evaluates the canonical `dns.query` security event, and either:
//!   - synthesizes an NXDOMAIN response (decision = Denied), or
//!   - forwards the bytes verbatim to an upstream nameserver via
//!     [`DnsResolver`] and returns the upstream answer
//!     (decision = Allowed), or
//!   - returns SERVFAIL when the upstream is unreachable
//!     (decision = Error).
//!
//! All three paths produce a [`DnsHandlerResult`] carrying the answer
//! bytes plus the structured fields the eventual `dns_events` writer
//! (T3.3) and `mitm.dns_queries_total{decision}` counter need. The
//! handler never logs the dns_events row itself -- T3 splits the
//! schema migration into its own slice, and keeping the handler free
//! of `DbWriter` makes T3.1 testable without spinning up sqlite.
//!
//! Security semantics: CEL rules over `dns.qname` / `dns.qtype` are the
//! NXDOMAIN gate. Redirect and cache policy still use the network-policy
//! snapshot because those are resolver mechanics, not allow/block authority.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use capsem_logger::events::Decision;
use tracing::{debug, instrument, warn};

use crate::net::dns::cache::DnsAnswerCache;
use crate::net::dns::resolver::DnsResolver;
use crate::net::mitm_proxy::metrics as m;
use crate::net::parsers::dns_parser::{
    build_nxdomain, build_redirect_response, build_servfail, parse_query, DnsQuery,
};
use crate::net::policy::NetworkPolicy;
use crate::net::policy_config::{PolicyCallback, SecurityPluginConfig, SecurityRuleSet};
use crate::security_engine::{
    evaluate_security_boundary, DnsSecurityEvent, SecurityEnforcementDecision, SecurityEvent,
};

/// Result of handling one DNS query. The answer bytes are always
/// populated -- on every path we have something to send back to the
/// guest, even if it's a synthetic SERVFAIL covering an upstream
/// failure. The caller writes `answer_bytes` over the vsock envelope
/// and uses the structured fields to emit a `dns_events` row + a
/// `mitm.dns_queries_total{decision=...}` counter increment.
#[derive(Debug, Clone)]
pub struct DnsHandlerResult {
    /// Wire-format DNS response, ready to ship over the vsock envelope.
    pub answer_bytes: Vec<u8>,
    /// Parsed query metadata. `None` on a malformed input where the
    /// raw bytes didn't decode (in which case `decision` is Error and
    /// `answer_bytes` is empty -- the agent should drop the request).
    pub query: Option<DnsQuery>,
    /// Policy + resolver outcome.
    pub decision: Decision,
    /// Matched policy rule ("api.openai.com", "*.openai.com", "default")
    /// when the decision is Denied; None for Allowed/Error.
    pub matched_rule: Option<String>,
    /// Wall time of the upstream resolve attempt, in milliseconds.
    /// 0 when the policy short-circuits (Denied) or when input parsing
    /// fails (Error).
    pub upstream_resolver_ms: u64,
    /// DNS rcode for the answer (0 = NoError, 2 = ServFail,
    /// 3 = NXDomain). Surfaced for telemetry; the wire-format response
    /// already carries it.
    pub rcode: u16,
    /// Policy engine mode that produced this decision, if any.
    pub policy_mode: Option<String>,
    /// Typed security action (`allow`, `ask`, `block`, `rewrite`) when
    /// a rule matched.
    pub policy_action: Option<String>,
    /// Fully qualified security rule id, e.g. `profiles.rules.block_openai_dns`.
    pub policy_rule: Option<String>,
    /// Human-readable policy reason or fail-closed detail.
    pub policy_reason: Option<String>,
}

impl DnsHandlerResult {
    fn denied(answer_bytes: Vec<u8>, query: DnsQuery, matched_rule: String) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Denied,
            matched_rule: Some(matched_rule),
            upstream_resolver_ms: 0,
            rcode: 3, // NXDomain
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    fn allowed(answer_bytes: Vec<u8>, query: DnsQuery, upstream_ms: u64, rcode: u16) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Allowed,
            matched_rule: None,
            upstream_resolver_ms: upstream_ms,
            rcode,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    fn redirected(answer_bytes: Vec<u8>, query: DnsQuery, matched_rule: String) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Redirected,
            matched_rule: Some(matched_rule),
            upstream_resolver_ms: 0, // policy short-circuit, no upstream call
            rcode: 0,                // NoError
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    fn upstream_failed(answer_bytes: Vec<u8>, query: DnsQuery, upstream_ms: u64) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Error,
            matched_rule: None,
            upstream_resolver_ms: upstream_ms,
            rcode: 2, // ServFail
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }

    fn parse_failed() -> Self {
        Self {
            answer_bytes: Vec::new(),
            query: None,
            decision: Decision::Error,
            matched_rule: None,
            upstream_resolver_ms: 0,
            rcode: 1, // FormErr -- closest to "we couldn't even decode the question"
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        }
    }
}

fn apply_security_enforcement_fields(
    result: &mut DnsHandlerResult,
    enforcement: &SecurityEnforcementDecision,
) {
    result.policy_mode = Some("security_event".to_string());
    result.policy_action = Some(enforcement.action.as_str().to_string());
    result.policy_rule = enforcement.rule_id.clone();
    result.policy_reason = enforcement.reason.clone();
}

/// Hot-swappable network policy snapshot for DNS resolver mechanics.
///
/// The outer `Arc<RwLock<...>>` lets admins edit the policy at runtime
/// (frontend's policy editor → service → write lock); the inner
/// `Arc<NetworkPolicy>` is what each request snapshots before redirect/cache
/// checks so we never hold the read lock across an await point.
pub type SharedPolicy = Arc<std::sync::RwLock<Arc<NetworkPolicy>>>;
pub type SharedSecurityRules = Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>;
pub type SharedPluginPolicy = Arc<std::sync::RwLock<BTreeMap<String, SecurityPluginConfig>>>;

/// Async DNS handler shared across vsock connections.
///
/// `policy` is shared (not cloned) with the MITM proxy via the same
/// `SharedPolicy` handle for resolver mechanics such as redirects.
///
/// `cache` is optional: pass `Some(Arc<DnsAnswerCache>)` to enable
/// the TTL-honoring answer cache (T3.f) which short-circuits the
/// upstream UDP RTT on repeated queries to allowed names. The
/// production `with_default_resolver()` constructor enables it by
/// default; tests that want to assert the upstream path always
/// runs use `new(policy, resolver)` which leaves cache=None.
#[derive(Clone)]
pub struct DnsHandler {
    policy: SharedPolicy,
    security_rules: SharedSecurityRules,
    plugin_policy: SharedPluginPolicy,
    resolver: Arc<DnsResolver>,
    cache: Option<Arc<DnsAnswerCache>>,
}

impl DnsHandler {
    /// Build a handler with no answer cache. Tests use this so a
    /// cache hit can't accidentally hide an upstream-path
    /// regression.
    pub fn new(
        policy: SharedPolicy,
        security_rules: SharedSecurityRules,
        plugin_policy: SharedPluginPolicy,
        resolver: Arc<DnsResolver>,
    ) -> Self {
        Self {
            policy,
            security_rules,
            plugin_policy,
            resolver,
            cache: None,
        }
    }

    /// Build a handler with an explicit answer cache.
    pub fn with_cache(
        policy: SharedPolicy,
        security_rules: SharedSecurityRules,
        plugin_policy: SharedPluginPolicy,
        resolver: Arc<DnsResolver>,
        cache: Arc<DnsAnswerCache>,
    ) -> Self {
        Self {
            policy,
            security_rules,
            plugin_policy,
            resolver,
            cache: Some(cache),
        }
    }

    /// Build a production handler: default UDP forwarder
    /// (DEFAULT_UPSTREAMS, 5s timeout) + default-sized
    /// TTL-honoring answer cache.
    pub fn with_default_resolver(
        policy: SharedPolicy,
        security_rules: SharedSecurityRules,
        plugin_policy: SharedPluginPolicy,
    ) -> Self {
        Self::with_cache(
            policy,
            security_rules,
            plugin_policy,
            Arc::new(DnsResolver::new()),
            Arc::new(DnsAnswerCache::default()),
        )
    }

    /// Borrow the cache (debugging / metrics only).
    pub fn cache(&self) -> Option<&Arc<DnsAnswerCache>> {
        self.cache.as_ref()
    }

    /// Snapshot the current `NetworkPolicy` under the read lock,
    /// release the lock immediately, and return the cheap-Arc snapshot
    /// for use across the rest of the request lifecycle.
    fn policy_snapshot(&self) -> Arc<NetworkPolicy> {
        self.policy.read().unwrap().clone()
    }

    /// Process one DNS query message. Pure async, no background tasks.
    ///
    /// The contract: every input produces a `DnsHandlerResult`, even
    /// malformed bytes (in which case `decision = Error` and
    /// `answer_bytes` is empty -- caller drops the request).
    ///
    /// Observability (T3.f):
    /// * `mitm.dns.query` info-span wraps the call. Fields filled in
    ///   on exit: `qname`, `qtype`, `decision`, `rcode`,
    ///   `upstream_ms`. Lets `RUST_LOG=capsem::net::dns=debug`
    ///   trace one query from parse to answer.
    /// * `mitm.dns_queries_total{decision}` counter increments per query.
    /// * `mitm.dns_handle_duration_ms` histogram on every exit.
    /// * `mitm.dns_upstream_duration_ms` histogram only when the
    ///   upstream forward path runs.
    /// * `mitm.dns_upstream_failures_total` counter on resolver error.
    #[instrument(
        target = "capsem::net::dns",
        name = "mitm.dns.query",
        skip(self, query_bytes),
        fields(qname, qtype, decision, rcode, upstream_ms)
    )]
    pub async fn handle(&self, query_bytes: &[u8]) -> DnsHandlerResult {
        let handle_t0 = Instant::now();
        let result = self.handle_inner(query_bytes).await;

        // Stamp span fields + emit metrics on every exit path. Done
        // here so the four early-returns inside handle_inner stay
        // simple and we don't drift between paths.
        let span = tracing::Span::current();
        if let Some(q) = &result.query {
            span.record("qname", q.qname.as_str());
            span.record("qtype", q.qtype);
        }
        span.record("decision", result.decision.as_str());
        span.record("rcode", result.rcode);
        span.record("upstream_ms", result.upstream_resolver_ms);

        ::metrics::counter!(
            m::DNS_QUERIES_TOTAL,
            "decision" => result.decision.as_str(),
        )
        .increment(1);
        ::metrics::histogram!(m::DNS_HANDLE_DURATION_MS)
            .record(handle_t0.elapsed().as_secs_f64() * 1000.0);

        result
    }

    /// Inner handler -- the actual decision tree. Wrapped by
    /// `handle()` which owns the span + metric emission so every
    /// exit path is observed identically.
    async fn handle_inner(&self, query_bytes: &[u8]) -> DnsHandlerResult {
        let query = match parse_query(query_bytes) {
            Ok(q) => q,
            Err(e) => {
                warn!(error = %e, "dns handler: failed to parse query");
                return DnsHandlerResult::parse_failed();
            }
        };

        let dns_security_event =
            SecurityEvent::new(PolicyCallback::DnsQuery).with_dns(DnsSecurityEvent {
                qname: Some(query.qname.clone()),
                qtype: Some(query.qtype.to_string()),
            });
        let rules = self.security_rules.read().unwrap().clone();
        let plugin_policy = self.plugin_policy.read().unwrap().clone();
        let dns_evaluation = match evaluate_security_boundary(
            &rules,
            plugin_policy,
            dns_security_event,
        ) {
            Ok(evaluation) => evaluation,
            Err(error) => {
                warn!(error = %error, qname = %query.qname, "dns handler: security engine failed");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                return DnsHandlerResult::upstream_failed(sf, query, 0);
            }
        };
        if !dns_evaluation.enforcement.is_allowed() {
            let matched_rule = dns_evaluation
                .enforcement
                .rule_id
                .clone()
                .unwrap_or_else(|| "security.dns.block".to_string());
            debug!(
                qname = %query.qname,
                qtype = query.qtype,
                matched_rule = %matched_rule,
                "dns handler: blocking query (NXDOMAIN)"
            );
            // Synthesizing the response can technically fail if the
            // input was unparseable -- but we already parsed it
            // successfully above. On the off chance hickory rejects
            // re-encoding (e.g. a query with an unrepresentable name),
            // fall through to ServFail rather than panic.
            let nxd = match build_nxdomain(query_bytes) {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "dns handler: failed to encode NXDOMAIN");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    return DnsHandlerResult::upstream_failed(sf, query, 0);
                }
            };
            let mut result = DnsHandlerResult::denied(nxd, query, matched_rule);
            apply_security_enforcement_fields(&mut result, &dns_evaluation.enforcement);
            return result;
        }

        let policy = self.policy_snapshot();

        // T3.d -- DNS redirect rules. Checked AFTER security enforcement
        // (a blocked query stays NXDOMAIN; redirect never weakens a block)
        // and BEFORE the upstream forward (no network round trip when an
        // admin has pinned the answer locally).
        if let Some(redirect) = policy.find_dns_redirect(&query.qname, query.qtype) {
            let matched_rule = format!("redirect:{}", redirect.matcher.pattern_str());
            debug!(
                qname = %query.qname,
                qtype = query.qtype,
                matched_rule = %matched_rule,
                answer_count = redirect.answers.len(),
                ttl = redirect.ttl,
                "dns handler: redirecting query (synthetic answer)"
            );
            match build_redirect_response(query_bytes, &redirect.answers, redirect.ttl) {
                Ok(bytes) => {
                    return DnsHandlerResult::redirected(bytes, query, matched_rule);
                }
                Err(e) => {
                    // Re-encoding failed despite a successful parse --
                    // surface as an error rather than fall back to
                    // upstream (admin intent was "do not forward").
                    warn!(error = %e, "dns handler: failed to build redirect response");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    return DnsHandlerResult::upstream_failed(sf, query, 0);
                }
            }
        }

        // T3.f -- answer cache check. Only consulted on the
        // upstream-forward path (block + redirect already
        // short-circuited above, and we want to re-evaluate them
        // every query). Cache::get re-checks policy on every hit
        // for coherence -- a domain that becomes blocked or
        // redirected after we cached its answer must not serve
        // from cache. See `dns/cache.rs` for the full invariant.
        if let Some(cache) = &self.cache {
            if let Some(cached) =
                cache.get(&query.qname, query.qtype, query.qclass, query.id, &policy)
            {
                let rcode = response_rcode(&cached);
                debug!(
                    qname = %query.qname,
                    qtype = query.qtype,
                    "dns handler: answer cache hit"
                );
                return DnsHandlerResult::allowed(cached, query, 0, rcode);
            }
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
        }

        let t0 = Instant::now();
        match self.resolver.resolve(query_bytes).await {
            Ok((resp, elapsed)) => {
                ::metrics::histogram!(m::DNS_UPSTREAM_DURATION_MS)
                    .record(elapsed.as_secs_f64() * 1000.0);
                let rcode = response_rcode(&resp);
                // Only cache positive (NoError) responses --
                // SERVFAIL / NXDOMAIN from upstream may be
                // transient, and we don't want to amplify a
                // momentary upstream blip into 5 minutes of
                // wrong answers.
                if rcode == 0 {
                    if let Some(cache) = &self.cache {
                        cache.insert(&query.qname, query.qtype, query.qclass, &resp);
                    }
                }
                DnsHandlerResult::allowed(resp, query, elapsed.as_millis() as u64, rcode)
            }
            Err(e) => {
                ::metrics::counter!(m::DNS_UPSTREAM_FAILURES_TOTAL).increment(1);
                warn!(qname = %query.qname, error = %e, "dns handler: upstream resolve failed");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                DnsHandlerResult::upstream_failed(sf, query, t0.elapsed().as_millis() as u64)
            }
        }
    }
}

/// Extract the rcode from a DNS response without doing a full decode.
/// The rcode lives in the low 4 bits of byte 3 of the DNS header
/// (RFC 1035 section 4.1.1). Returns 2 (ServFail) when the bytes are
/// too short to inspect, which matches our intent on truncated
/// responses anyway.
fn response_rcode(bytes: &[u8]) -> u16 {
    if bytes.len() < 4 {
        return 2;
    }
    u16::from(bytes[3] & 0x0F)
}

#[cfg(test)]
mod tests;
