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

use std::sync::Arc;
use std::time::Instant;

use capsem_logger::events::Decision;
use tracing::{debug, instrument, warn};

use crate::net::dns::cache::DnsAnswerCache;
use crate::net::dns::resolver::DnsResolver;
use crate::net::parsers::dns_parser::{
    build_nxdomain, build_ptr_response, build_redirect_response, build_servfail, parse_query, DnsQuery,
};
use crate::net::proxy_engine::ProxyPolicyHandle;
use crate::security_engine::{
    evaluate_security_boundary, DnsSecurityEvent, RuntimeSecurityEventType, SecurityBoundaryEvaluation, SecurityEvent,
    SecurityEventId,
};
use capsem_telemetry::dns as m;

const CAPSEM_LOCAL_NXDOMAIN_SUFFIX: &str = ".capsem-bogus";
const CAPSEM_LOCAL_NXDOMAIN_RULE: &str = "resolver.local_nxdomain.capsem_bogus";
const PRIVATE_ZONE_RULE: &str = "private:capsem.internal";
const PRIVATE_MISS_RULE: &str = "private.miss";

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
    /// Identity already persisted for the original held Ask operation.
    pub approval_event_id: Option<SecurityEventId>,
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
            approval_event_id: None,
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
            approval_event_id: None,
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
            approval_event_id: None,
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
            approval_event_id: None,
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
            approval_event_id: None,
        }
    }

    fn with_security(mut self, evaluation: &SecurityBoundaryEvaluation) -> Self {
        let enforcement = &evaluation.enforcement;
        if self.matched_rule.is_none() {
            self.matched_rule = enforcement.rule_id.clone();
        }
        self.policy_mode = Some("security_event".to_string());
        self.policy_action = Some(enforcement.action.as_str().to_string());
        self.policy_rule = enforcement.rule_id.clone();
        self.policy_reason = enforcement.reason.clone();
        self.approval_event_id = evaluation.approval_event_id.clone();
        self
    }
}

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
    policy: ProxyPolicyHandle,
    resolver: Arc<DnsResolver>,
    cache: Option<Arc<DnsAnswerCache>>,
    /// Identical lookups on their way upstream, shared by every clone so
    /// concurrent queries for one name cost one upstream round trip.
    in_flight: Arc<super::coalesce::InFlightLookups>,
    /// Who answers `capsem.internal`; without one the zone is empty.
    private_names: Option<Arc<dyn super::private::PrivateNames>>,
    engine: Option<Arc<crate::net::proxy_engine::ProxyEngine>>,
}

impl DnsHandler {
    /// Build a handler with no answer cache. Tests use this so a
    /// cache hit can't accidentally hide an upstream-path
    /// regression.
    pub fn new(policy: ProxyPolicyHandle, resolver: Arc<DnsResolver>) -> Self {
        Self {
            policy,
            resolver,
            cache: None,
            in_flight: Arc::default(),
            private_names: None,
            engine: None,
        }
    }

    /// Build a handler with an explicit answer cache.
    pub fn with_cache(policy: ProxyPolicyHandle, resolver: Arc<DnsResolver>, cache: Arc<DnsAnswerCache>) -> Self {
        Self {
            policy,
            resolver,
            cache: Some(cache),
            in_flight: Arc::default(),
            private_names: None,
            engine: None,
        }
    }

    /// Build a production handler: default UDP forwarder
    /// (DEFAULT_UPSTREAMS, 5s timeout) + default-sized
    /// TTL-honoring answer cache.
    pub fn with_default_resolver(policy: ProxyPolicyHandle) -> Self {
        Self::with_cache(
            policy,
            Arc::new(DnsResolver::new()),
            Arc::new(DnsAnswerCache::default()),
        )
    }

    /// Answer the private zone through `names`.
    pub fn with_private_names(mut self, names: Arc<dyn super::private::PrivateNames>) -> Self {
        self.private_names = Some(names);
        self
    }

    /// Production queries use the same approval and ledger capability as HTTP.
    pub fn with_engine(mut self, engine: Arc<crate::net::proxy_engine::ProxyEngine>) -> Self {
        self.policy = engine.policy().clone();
        self.engine = Some(engine);
        self
    }

    /// The private zone and the pool's reverse zone are the host's to
    /// answer: a member's name or address for the asker, or nothing. Zero
    /// TTL, no cache, never upstream.
    async fn answer_private(
        &self,
        query_bytes: &[u8],
        query: DnsQuery,
        question: super::private::PrivateQuestion,
    ) -> DnsHandlerResult {
        use super::private::PrivateQuestion;
        let answer = match (&self.private_names, question) {
            // Capsem's own name, in every VM whether or not it has peers.
            (_, PrivateQuestion::McpGateway) => Some(build_redirect_response(
                query_bytes,
                &[std::net::IpAddr::V4(super::private::MCP_ADDRESS)],
                0,
            )),
            (Some(names), PrivateQuestion::Name(name)) => names
                .address_of(&name)
                .await
                .map(|address| build_redirect_response(query_bytes, &[std::net::IpAddr::V4(address)], 0)),
            (Some(names), PrivateQuestion::Reverse(address)) => names
                .name_of(address)
                .await
                .map(|owner| build_ptr_response(query_bytes, &owner, 0)),
            (None, _) => None,
        };
        match answer {
            Some(Ok(bytes)) => DnsHandlerResult::redirected(bytes, query, PRIVATE_ZONE_RULE.to_string()),
            Some(Err(error)) => {
                warn!(error = %error, qname = %query.qname, "dns handler: failed to encode private answer");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                DnsHandlerResult::upstream_failed(sf, query, 0)
            }
            None => match build_nxdomain(query_bytes) {
                Ok(nxd) => DnsHandlerResult::denied(nxd, query, PRIVATE_MISS_RULE.to_string()),
                Err(error) => {
                    warn!(error = %error, "dns handler: failed to encode private NXDOMAIN");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    DnsHandlerResult::upstream_failed(sf, query, 0)
                }
            },
        }
    }

    /// Borrow the cache (debugging / metrics only).
    pub fn cache(&self) -> Option<&Arc<DnsAnswerCache>> {
        self.cache.as_ref()
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
    pub async fn handle(&self, query_bytes: &[u8]) -> DnsHandlerResult {
        self.handle_with_cancellation(query_bytes, None).await
    }

    pub(crate) async fn handle_with_cancellation(
        &self,
        query_bytes: &[u8],
        cancellation: Option<&tokio_util::sync::CancellationToken>,
    ) -> DnsHandlerResult {
        self.handle_registered(query_bytes, cancellation, None).await
    }

    #[instrument(
        target = "capsem::net::dns",
        name = "mitm.dns.query",
        skip(self, query_bytes, cancellation, registered),
        fields(qname, qtype, decision, rcode, upstream_ms)
    )]
    pub(crate) async fn handle_registered(
        &self,
        query_bytes: &[u8],
        cancellation: Option<&tokio_util::sync::CancellationToken>,
        registered: Option<tokio::sync::oneshot::Sender<capsem_proto::approval::ApprovalRegistration>>,
    ) -> DnsHandlerResult {
        let handle_t0 = Instant::now();
        let result = self.handle_inner(query_bytes, cancellation, registered).await;

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
        ::metrics::histogram!(m::DNS_HANDLE_DURATION_MS).record(handle_t0.elapsed().as_secs_f64() * 1000.0);

        result
    }

    /// Inner handler -- the actual decision tree. Wrapped by
    /// `handle()` which owns the span + metric emission so every
    /// exit path is observed identically.
    async fn handle_inner(
        &self,
        query_bytes: &[u8],
        cancellation: Option<&tokio_util::sync::CancellationToken>,
        registered: Option<tokio::sync::oneshot::Sender<capsem_proto::approval::ApprovalRegistration>>,
    ) -> DnsHandlerResult {
        let query = match parse_query(query_bytes) {
            Ok(q) => q,
            Err(e) => {
                debug!(error = %e, "dns handler: failed to parse query");
                return DnsHandlerResult::parse_failed();
            }
        };

        // Policy evaluates one standard question. Never forward an unchecked
        // second question, a DNS update, or a reflected response upstream.
        if query.extra_questions != 0 || query_bytes[2] & 0xF8 != 0 {
            let mut result = DnsHandlerResult::parse_failed();
            result.query = Some(query);
            return result;
        }

        let dns_security_event = SecurityEvent::new(RuntimeSecurityEventType::DnsQuery).with_dns(DnsSecurityEvent {
            qname: Some(query.qname.clone()),
            qtype: Some(query.qtype.to_string()),
        });
        let policy = self.policy.snapshot();
        let evaluation = if let Some(engine) = &self.engine {
            engine
                .evaluate_registered(
                    &policy,
                    dns_security_event,
                    query_bytes.len().saturating_mul(2),
                    cancellation,
                    registered,
                )
                .await
        } else {
            evaluate_security_boundary(
                policy.security_rules(),
                Arc::clone(policy.plugins()),
                dns_security_event,
            )
        };
        let dns_evaluation = match evaluation {
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
                    return DnsHandlerResult::upstream_failed(sf, query, 0).with_security(&dns_evaluation);
                }
            };
            return DnsHandlerResult::denied(nxd, query, matched_rule).with_security(&dns_evaluation);
        }

        if let Some(question) = super::private::private_question(&query.qname, query.qtype) {
            return self
                .answer_private(query_bytes, query, question)
                .await
                .with_security(&dns_evaluation);
        }

        if is_capsem_local_nxdomain_name(&query.qname) {
            debug!(
                qname = %query.qname,
                qtype = query.qtype,
                matched_rule = CAPSEM_LOCAL_NXDOMAIN_RULE,
                "dns handler: local NXDOMAIN fixture"
            );
            let nxd = match build_nxdomain(query_bytes) {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "dns handler: failed to encode local NXDOMAIN fixture");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    return DnsHandlerResult::upstream_failed(sf, query, 0).with_security(&dns_evaluation);
                }
            };
            return DnsHandlerResult::denied(nxd, query, CAPSEM_LOCAL_NXDOMAIN_RULE.to_string())
                .with_security(&dns_evaluation);
        }

        // T3.d -- DNS redirect rules. Checked AFTER security enforcement
        // (a blocked query stays NXDOMAIN; redirect never weakens a block)
        // and BEFORE the upstream forward (no network round trip when an
        // admin has pinned the answer locally).
        if let Some(redirect) = policy.network().find_dns_redirect(&query.qname, query.qtype) {
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
                    return DnsHandlerResult::redirected(bytes, query, matched_rule).with_security(&dns_evaluation);
                }
                Err(e) => {
                    // Re-encoding failed despite a successful parse --
                    // surface as an error rather than fall back to
                    // upstream (admin intent was "do not forward").
                    warn!(error = %e, "dns handler: failed to build redirect response");
                    let sf = build_servfail(query_bytes).unwrap_or_default();
                    return DnsHandlerResult::upstream_failed(sf, query, 0).with_security(&dns_evaluation);
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
        let key = super::coalesce::LookupKey::new(query_bytes);
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(&query, &key, policy.network()) {
                let rcode = response_rcode(&cached);
                debug!(
                    qname = %query.qname,
                    qtype = query.qtype,
                    "dns handler: answer cache hit"
                );
                return DnsHandlerResult::allowed(cached, query, 0, rcode).with_security(&dns_evaluation);
            }
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
        }

        // Every check that could refuse or redirect this query has passed;
        // only now may it share an upstream lookup with an identical one.
        let t0 = Instant::now();
        let (outcome, led) = match self.in_flight.join_or_lead(key.clone()) {
            super::coalesce::Role::Lead(lease) => {
                let outcome = self
                    .resolver
                    .resolve_for_policy(query_bytes, policy.digest())
                    .await
                    .map_err(|e| format!("{e:#}"));
                (lease.finish(outcome), true)
            }
            super::coalesce::Role::Follow(rx) => {
                ::metrics::counter!(m::DNS_UPSTREAM_COALESCED_TOTAL).increment(1);
                let outcome = match rx.await {
                    Ok(outcome) => outcome,
                    Err(_) => Arc::new(Err("upstream lookup leader vanished".to_string())),
                };
                (outcome, false)
            }
        };
        match outcome.as_ref() {
            Ok((resp, elapsed)) => {
                let mut resp = resp.clone();
                // A follower's answer was fetched under the leader's
                // transaction id; hand each query its own back, as the
                // cache does, or the client's resolver discards it.
                if resp.len() >= 2 {
                    resp[0..2].copy_from_slice(&query.id.to_be_bytes());
                }
                let rcode = response_rcode(&resp);
                if led {
                    ::metrics::histogram!(m::DNS_UPSTREAM_DURATION_MS).record(elapsed.as_secs_f64() * 1000.0);
                    if let Some(cache) = &self.cache {
                        match rcode {
                            // Positive answers keep their record TTLs.
                            0 => cache.insert(&key, &resp),
                            // A name that does not exist is remembered briefly, so a
                            // client retrying a dead name does not cost an upstream
                            // round trip each time. SERVFAIL and other codes may be a
                            // transient upstream fault and are never cached.
                            3 => cache.insert_negative(&key, &resp),
                            _ => {}
                        }
                    }
                }
                let waited_ms = if led {
                    elapsed.as_millis()
                } else {
                    t0.elapsed().as_millis()
                } as u64;
                DnsHandlerResult::allowed(resp, query, waited_ms, rcode).with_security(&dns_evaluation)
            }
            Err(e) => {
                ::metrics::counter!(m::DNS_UPSTREAM_FAILURES_TOTAL).increment(1);
                warn!(qname = %query.qname, error = %e, "dns handler: upstream resolve failed");
                let sf = build_servfail(query_bytes).unwrap_or_default();
                DnsHandlerResult::upstream_failed(sf, query, t0.elapsed().as_millis() as u64)
                    .with_security(&dns_evaluation)
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

fn is_capsem_local_nxdomain_name(qname: &str) -> bool {
    let name = qname.trim_end_matches('.').to_ascii_lowercase();
    name == "capsem-bogus" || name.ends_with(CAPSEM_LOCAL_NXDOMAIN_SUFFIX)
}

#[cfg(test)]
mod tests;
