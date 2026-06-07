//! Bytes-in / bytes-out DNS handler with policy gating + telemetry hook.
//!
//! Receives a raw DNS query (decoded over the vsock envelope from the
//! guest agent), runs the shared `NetworkPolicy::is_fully_blocked` check
//! on the qname, and either:
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
//! Policy semantics: we use `is_fully_blocked` (both read AND write
//! denied) as the trigger for NXDOMAIN. A read-only domain (e.g.
//! pypi.org) is still resolvable -- the guest needs the IP to even
//! attempt the connection, after which the MITM proxy enforces the
//! verb-level policy. NXDOMAINing read-only domains would make a `pip
//! install` fail at name resolution rather than at the HTTP layer,
//! which loses the audit trail for the actual request shape.

use std::borrow::Cow;
use std::net::IpAddr;
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
use crate::net::policy_config::{
    MatchedPolicyRule, PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
    PolicySubject, PolicySubjectValue,
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
    /// Typed policy action (`allow`, `ask`, `block`, `rewrite`) when
    /// Policy V2 matched.
    pub policy_action: Option<String>,
    /// Fully qualified policy rule id, e.g. `policy.dns.block_openai`.
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

    fn policy_failed(answer_bytes: Vec<u8>, query: DnsQuery, matched_rule: String) -> Self {
        Self {
            answer_bytes,
            query: Some(query),
            decision: Decision::Error,
            matched_rule: Some(matched_rule),
            upstream_resolver_ms: 0,
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

    fn with_policy_v2(mut self, decision: DnsPolicyV2Decision) -> Self {
        self.policy_mode = decision.policy_mode;
        self.policy_action = decision.policy_action;
        self.policy_rule = decision.policy_rule;
        self.policy_reason = decision.policy_reason;
        self
    }
}

/// Hot-swappable network policy snapshot shared with the MITM proxy.
///
/// The outer `Arc<RwLock<...>>` lets admins edit the policy at runtime
/// (frontend's policy editor → service → write lock); the inner
/// `Arc<NetworkPolicy>` is what each request snapshots before evaluation
/// so we never hold the read lock across an await point.
pub type SharedPolicy = Arc<std::sync::RwLock<Arc<NetworkPolicy>>>;
pub type SharedPolicyV2 = Arc<tokio::sync::RwLock<Arc<PolicyConfig>>>;

/// Async DNS handler shared across vsock connections.
///
/// `policy` is shared (not cloned) with the MITM proxy via the same
/// `SharedPolicy` handle -- a domain rule change applied via the
/// frontend's policy editor takes effect for both protocols at once.
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
    policy_v2: SharedPolicyV2,
    resolver: Arc<DnsResolver>,
    cache: Option<Arc<DnsAnswerCache>>,
}

impl DnsHandler {
    /// Build a handler with no answer cache. Tests use this so a
    /// cache hit can't accidentally hide an upstream-path
    /// regression.
    pub fn new(policy: SharedPolicy, resolver: Arc<DnsResolver>) -> Self {
        Self::new_with_policy_v2(policy, default_policy_v2(), resolver)
    }

    /// Build a handler with no answer cache and an explicit Policy V2
    /// snapshot handle. Runtime code passes the same handle used by
    /// MCP/HTTP so settings reload updates every inspected boundary
    /// together.
    pub fn new_with_policy_v2(
        policy: SharedPolicy,
        policy_v2: SharedPolicyV2,
        resolver: Arc<DnsResolver>,
    ) -> Self {
        Self {
            policy,
            policy_v2,
            resolver,
            cache: None,
        }
    }

    /// Build a handler with an explicit answer cache.
    pub fn with_cache(
        policy: SharedPolicy,
        resolver: Arc<DnsResolver>,
        cache: Arc<DnsAnswerCache>,
    ) -> Self {
        Self::with_cache_and_policy_v2(policy, default_policy_v2(), resolver, cache)
    }

    /// Build a handler with an explicit answer cache and Policy V2 handle.
    pub fn with_cache_and_policy_v2(
        policy: SharedPolicy,
        policy_v2: SharedPolicyV2,
        resolver: Arc<DnsResolver>,
        cache: Arc<DnsAnswerCache>,
    ) -> Self {
        Self {
            policy,
            policy_v2,
            resolver,
            cache: Some(cache),
        }
    }

    /// Build a production handler: default UDP forwarder
    /// (DEFAULT_UPSTREAMS, 5s timeout) + default-sized
    /// TTL-honoring answer cache.
    pub fn with_default_resolver(policy: SharedPolicy) -> Self {
        Self::with_default_resolver_and_policy_v2(policy, default_policy_v2())
    }

    /// Build a production handler with the shared Policy V2 handle.
    pub fn with_default_resolver_and_policy_v2(
        policy: SharedPolicy,
        policy_v2: SharedPolicyV2,
    ) -> Self {
        Self::with_cache_and_policy_v2(
            policy,
            policy_v2,
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

    fn apply_policy_v2_rule(
        &self,
        query_bytes: &[u8],
        query: DnsQuery,
        matched: MatchedPolicyRule<'_>,
    ) -> Result<DnsPolicyV2Outcome, String> {
        let decision = DnsPolicyV2Decision::from_match(matched.name, matched.rule);
        let matched_rule = format!("policy.dns.{}", matched.name);
        match matched.rule.decision {
            PolicyDecisionKind::Action | PolicyDecisionKind::Allow => {
                Ok(DnsPolicyV2Outcome::Continue(decision))
            }
            PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                let nxd = build_nxdomain(query_bytes)
                    .map_err(|error| format!("failed to encode policy NXDOMAIN: {error}"))?;
                Ok(DnsPolicyV2Outcome::Respond(
                    DnsHandlerResult::denied(nxd, query, matched_rule).with_policy_v2(decision),
                ))
            }
            PolicyDecisionKind::Rewrite => {
                let answers = dns_rewrite_answers(matched.rule)?;
                let bytes = build_redirect_response(query_bytes, &answers, 60)
                    .map_err(|error| format!("failed to encode policy DNS rewrite: {error}"))?;
                Ok(DnsPolicyV2Outcome::Respond(
                    DnsHandlerResult::redirected(bytes, query, matched_rule)
                        .with_policy_v2(decision),
                ))
            }
        }
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

        let policy = self.policy_snapshot();
        if let Some(matched_rule) = policy.is_fully_blocked(&query.qname) {
            debug!(
                qname = %query.qname,
                qtype = query.qtype,
                matched_rule = %matched_rule,
                "dns handler: blocking domain (NXDOMAIN)"
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
            return DnsHandlerResult::denied(nxd, query, matched_rule);
        }

        let policy_v2 = self.policy_v2.read().await.clone();
        let subject = DnsQueryPolicySubject::new(&query);
        let matched =
            match policy_v2.find_matching_decision_rule(PolicyCallback::DnsQuery, &subject) {
                Ok(Some(matched)) => Some(matched),
                Ok(None) => None,
                Err(error) => {
                    warn!(
                        qname = %query.qname,
                        qtype = query.qtype,
                        error = %error,
                        "dns handler: Policy V2 condition failed closed"
                    );
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    let decision = DnsPolicyV2Decision::invalid_condition(error);
                    return DnsHandlerResult::policy_failed(
                        sf,
                        query,
                        "policy.dns.invalid_condition".to_string(),
                    )
                    .with_policy_v2(decision);
                }
            };
        let mut continuing_policy_v2 = None;
        if let Some(matched) = matched {
            match self.apply_policy_v2_rule(query_bytes, query.clone(), matched) {
                Ok(DnsPolicyV2Outcome::Respond(result)) => return result,
                Ok(DnsPolicyV2Outcome::Continue(decision)) => {
                    continuing_policy_v2 = Some(decision);
                }
                Err(error) => {
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    let decision =
                        DnsPolicyV2Decision::from_failure(matched.name, matched.rule, error);
                    return DnsHandlerResult::policy_failed(
                        sf,
                        query,
                        format!("policy.dns.{}", matched.name),
                    )
                    .with_policy_v2(decision);
                }
            }
        }

        // T3.d -- DNS redirect rules. Checked AFTER is_fully_blocked
        // (a blocked domain stays NXDOMAIN; redirect never weakens
        // a block) and BEFORE the upstream forward (no network round
        // trip when an admin has pinned the answer locally).
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
                    return with_optional_policy_v2(
                        DnsHandlerResult::redirected(bytes, query, matched_rule),
                        &continuing_policy_v2,
                    );
                }
                Err(e) => {
                    // Re-encoding failed despite a successful parse --
                    // surface as an error rather than fall back to
                    // upstream (admin intent was "do not forward").
                    warn!(error = %e, "dns handler: failed to build redirect response");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    return with_optional_policy_v2(
                        DnsHandlerResult::upstream_failed(sf, query, 0),
                        &continuing_policy_v2,
                    );
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
                return with_optional_policy_v2(
                    DnsHandlerResult::allowed(cached, query, 0, rcode),
                    &continuing_policy_v2,
                );
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
                with_optional_policy_v2(
                    DnsHandlerResult::allowed(resp, query, elapsed.as_millis() as u64, rcode),
                    &continuing_policy_v2,
                )
            }
            Err(e) => {
                ::metrics::counter!(m::DNS_UPSTREAM_FAILURES_TOTAL).increment(1);
                warn!(qname = %query.qname, error = %e, "dns handler: upstream resolve failed");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                with_optional_policy_v2(
                    DnsHandlerResult::upstream_failed(sf, query, t0.elapsed().as_millis() as u64),
                    &continuing_policy_v2,
                )
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

fn default_policy_v2() -> SharedPolicyV2 {
    Arc::new(tokio::sync::RwLock::new(Arc::new(PolicyConfig::default())))
}

#[derive(Clone, Debug, Default)]
struct DnsPolicyV2Decision {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

enum DnsPolicyV2Outcome {
    Continue(DnsPolicyV2Decision),
    Respond(DnsHandlerResult),
}

impl DnsPolicyV2Decision {
    fn from_match(name: &str, rule: &PolicyRuleConfig) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(policy_action(rule.decision).to_string()),
            policy_rule: Some(format!("policy.dns.{name}")),
            policy_reason: Some(
                rule.reason
                    .clone()
                    .unwrap_or_else(|| format!("Policy V2 DNS {:?} rule matched", rule.decision)),
            ),
        }
    }

    fn from_failure(name: &str, rule: &PolicyRuleConfig, error: String) -> Self {
        let mut decision = Self::from_match(name, rule);
        let base = decision.policy_reason.clone().unwrap_or_default();
        decision.policy_reason = Some(format!("{base}; policy failed closed: {error}"));
        decision
    }

    fn invalid_condition(error: String) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some("block".to_string()),
            policy_rule: Some("policy.dns.invalid_condition".to_string()),
            policy_reason: Some(format!("Policy V2 DNS condition failed closed: {error}")),
        }
    }
}

fn with_optional_policy_v2(
    result: DnsHandlerResult,
    decision: &Option<DnsPolicyV2Decision>,
) -> DnsHandlerResult {
    match decision {
        Some(decision) => result.with_policy_v2(decision.clone()),
        None => result,
    }
}

struct DnsQueryPolicySubject<'a> {
    query: &'a DnsQuery,
    qtype: String,
}

impl<'a> DnsQueryPolicySubject<'a> {
    fn new(query: &'a DnsQuery) -> Self {
        Self {
            query,
            qtype: dns_qtype_label(query.qtype).into_owned(),
        }
    }
}

impl PolicySubject for DnsQueryPolicySubject<'_> {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "qname" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.query.qname.as_str(),
            ))),
            "qtype" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.qtype.as_str(),
            ))),
            // The guest DNS proxy currently forwards UDP queries to this
            // byte-in/byte-out handler. The source protocol field is still
            // carried separately into telemetry by the vsock envelope.
            "protocol" => Some(PolicySubjectValue::String(Cow::Borrowed("udp"))),
            // Process attribution is unavailable at this DNS boundary today.
            "process.name" => None,
            _ => None,
        }
    }
}

fn dns_rewrite_answers(rule: &PolicyRuleConfig) -> Result<Vec<IpAddr>, String> {
    let target = rule
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    validate_dns_rewrite_target(target)?;
    let value = rule
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let mut answers = Vec::new();
    for raw in value.split(',') {
        let ip = raw.trim();
        if ip.is_empty() {
            return Err("DNS rewrite answer contains an empty IP".to_string());
        }
        answers.push(
            ip.parse::<IpAddr>()
                .map_err(|error| format!("DNS rewrite answer '{ip}' is not an IP: {error}"))?,
        );
    }
    Ok(answers)
}

fn validate_dns_rewrite_target(target: &str) -> Result<(), String> {
    let Some((field, regex_text)) = target.split_once("=~") else {
        return Err("DNS rewrite_target must use '<field> =~ <regex>'".to_string());
    };
    let field = field.trim();
    if field != "answer.ip" && field != "answer.ips" {
        return Err(format!("unsupported DNS rewrite target '{field}'"));
    }

    let regex_text = regex_text.trim();
    if regex_text.len() < 2 {
        return Err("DNS rewrite_target regex must be quoted".to_string());
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        return Err("DNS rewrite_target regex must be quoted".to_string());
    }
    let Some(end) = regex_text[1..].rfind(quote) else {
        return Err("DNS rewrite_target regex is missing a closing quote".to_string());
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        return Err(
            "DNS rewrite_target regex has trailing content after closing quote".to_string(),
        );
    }
    let pattern = &regex_text[1..=end];
    regex::Regex::new(pattern).map_err(|error| format!("invalid DNS rewrite regex: {error}"))?;
    Ok(())
}

fn dns_qtype_label(qtype: u16) -> Cow<'static, str> {
    match qtype {
        1 => Cow::Borrowed("A"),
        2 => Cow::Borrowed("NS"),
        5 => Cow::Borrowed("CNAME"),
        6 => Cow::Borrowed("SOA"),
        12 => Cow::Borrowed("PTR"),
        15 => Cow::Borrowed("MX"),
        16 => Cow::Borrowed("TXT"),
        28 => Cow::Borrowed("AAAA"),
        33 => Cow::Borrowed("SRV"),
        65 => Cow::Borrowed("HTTPS"),
        _ => Cow::Owned(qtype.to_string()),
    }
}

fn policy_action(decision: PolicyDecisionKind) -> &'static str {
    match decision {
        PolicyDecisionKind::Action => "action",
        PolicyDecisionKind::Allow => "allow",
        PolicyDecisionKind::Ask => "ask",
        PolicyDecisionKind::Block => "block",
        PolicyDecisionKind::Rewrite => "rewrite",
    }
}
