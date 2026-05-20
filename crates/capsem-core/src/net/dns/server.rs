//! Bytes-in / bytes-out DNS handler with policy gating + telemetry hook.
//!
//! Receives a raw DNS query (decoded over the vsock envelope from the
//! guest agent), evaluates Policy `dns.request` rules for the qname, and
//! either:
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
//! Policy semantics: `dns.request` rules are the DNS authority. A block or ask
//! decision returns NXDOMAIN; an allow decision continues to cache/upstream; a
//! rewrite decision synthesizes a DNS answer.

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
use crate::net::policy::{
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
    /// Policy matched.
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

    fn with_policy(mut self, decision: DnsPolicyDecision) -> Self {
        self.policy_mode = decision.policy_mode;
        self.policy_action = decision.policy_action;
        self.policy_rule = decision.policy_rule;
        self.policy_reason = decision.policy_reason;
        self
    }
}

/// Hot-swappable Policy snapshot shared with the MITM proxy.
pub type SharedPolicy = Arc<tokio::sync::RwLock<Arc<PolicyConfig>>>;

/// Async DNS handler shared across vsock connections.
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
    resolver: Arc<DnsResolver>,
    cache: Option<Arc<DnsAnswerCache>>,
}

impl DnsHandler {
    /// Build a handler with no answer cache. Tests use this so a
    /// cache hit can't accidentally hide an upstream-path
    /// regression.
    pub fn new(policy: SharedPolicy, resolver: Arc<DnsResolver>) -> Self {
        Self {
            policy,
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
        Self {
            policy,
            resolver,
            cache: Some(cache),
        }
    }

    /// Build a production handler: default UDP forwarder
    /// (DEFAULT_UPSTREAMS, 5s timeout) + default-sized
    /// TTL-honoring answer cache.
    pub fn with_default_resolver(policy: SharedPolicy) -> Self {
        Self::with_cache(
            policy,
            Arc::new(DnsResolver::new()),
            Arc::new(DnsAnswerCache::default()),
        )
    }

    /// Borrow the cache (debugging / metrics only).
    pub fn cache(&self) -> Option<&Arc<DnsAnswerCache>> {
        self.cache.as_ref()
    }

    fn apply_policy_rule(
        &self,
        query_bytes: &[u8],
        query: DnsQuery,
        matched: MatchedPolicyRule<'_>,
    ) -> Result<DnsPolicyOutcome, String> {
        let decision = DnsPolicyDecision::from_match(matched.name, matched.rule);
        let matched_rule = format!("policy.dns.{}", matched.name);
        match matched.rule.decision {
            PolicyDecisionKind::Allow => Ok(DnsPolicyOutcome::Continue(decision)),
            PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                let nxd = build_nxdomain(query_bytes)
                    .map_err(|error| format!("failed to encode policy NXDOMAIN: {error}"))?;
                Ok(DnsPolicyOutcome::Respond(
                    DnsHandlerResult::denied(nxd, query, matched_rule).with_policy(decision),
                ))
            }
            PolicyDecisionKind::Rewrite => {
                let answers = dns_rewrite_answers(matched.rule)?;
                let bytes = build_redirect_response(query_bytes, &answers, 60)
                    .map_err(|error| format!("failed to encode policy DNS rewrite: {error}"))?;
                Ok(DnsPolicyOutcome::Respond(
                    DnsHandlerResult::redirected(bytes, query, matched_rule).with_policy(decision),
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

        let policy = self.policy.read().await.clone();
        let subject = DnsQueryPolicySubject::new(&query);
        let matched = match policy.find_matching_rule(PolicyCallback::DnsQuery, &subject) {
            Ok(Some(matched)) => Some(matched),
            Ok(None) => None,
            Err(error) => {
                warn!(
                    qname = %query.qname,
                    qtype = query.qtype,
                    error = %error,
                    "dns handler: Policy condition failed closed"
                );
                let sf = build_servfail(query_bytes).unwrap_or_default();
                let decision = DnsPolicyDecision::invalid_condition(error);
                return DnsHandlerResult::policy_failed(
                    sf,
                    query,
                    "policy.dns.invalid_condition".to_string(),
                )
                .with_policy(decision);
            }
        };
        let mut continuing_policy = None;
        if let Some(matched) = matched {
            match self.apply_policy_rule(query_bytes, query.clone(), matched) {
                Ok(DnsPolicyOutcome::Respond(result)) => return result,
                Ok(DnsPolicyOutcome::Continue(decision)) => {
                    continuing_policy = Some(decision);
                }
                Err(error) => {
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    let decision =
                        DnsPolicyDecision::from_failure(matched.name, matched.rule, error);
                    return DnsHandlerResult::policy_failed(
                        sf,
                        query,
                        format!("policy.dns.{}", matched.name),
                    )
                    .with_policy(decision);
                }
            }
        }

        // T3.f -- answer cache check. Only consulted on the
        // upstream-forward path (block + rewrite already
        // short-circuited above, and we want to re-evaluate them
        // every query before consulting the cache.
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(&query.qname, query.qtype, query.qclass, query.id) {
                let rcode = response_rcode(&cached);
                debug!(
                    qname = %query.qname,
                    qtype = query.qtype,
                    "dns handler: answer cache hit"
                );
                return with_optional_policy(
                    DnsHandlerResult::allowed(cached, query, 0, rcode),
                    &continuing_policy,
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
                with_optional_policy(
                    DnsHandlerResult::allowed(resp, query, elapsed.as_millis() as u64, rcode),
                    &continuing_policy,
                )
            }
            Err(e) => {
                ::metrics::counter!(m::DNS_UPSTREAM_FAILURES_TOTAL).increment(1);
                warn!(qname = %query.qname, error = %e, "dns handler: upstream resolve failed");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                with_optional_policy(
                    DnsHandlerResult::upstream_failed(sf, query, t0.elapsed().as_millis() as u64),
                    &continuing_policy,
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

#[derive(Clone, Debug, Default)]
struct DnsPolicyDecision {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

enum DnsPolicyOutcome {
    Continue(DnsPolicyDecision),
    Respond(DnsHandlerResult),
}

impl DnsPolicyDecision {
    fn from_match(name: &str, rule: &PolicyRuleConfig) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(policy_action(rule.decision).to_string()),
            policy_rule: Some(format!("policy.dns.{name}")),
            policy_reason: Some(
                rule.reason
                    .clone()
                    .unwrap_or_else(|| format!("Policy DNS {:?} rule matched", rule.decision)),
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
            policy_reason: Some(format!("Policy DNS condition failed closed: {error}")),
        }
    }
}

fn with_optional_policy(
    result: DnsHandlerResult,
    decision: &Option<DnsPolicyDecision>,
) -> DnsHandlerResult {
    match decision {
        Some(decision) => result.with_policy(decision.clone()),
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
        PolicyDecisionKind::Allow => "allow",
        PolicyDecisionKind::Ask => "ask",
        PolicyDecisionKind::Block => "block",
        PolicyDecisionKind::Rewrite => "rewrite",
    }
}
