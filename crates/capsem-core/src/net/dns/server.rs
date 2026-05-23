//! Bytes-in / bytes-out DNS handler with telemetry hook.
//!
//! Receives a raw DNS query (decoded over the vsock envelope from the
//! guest agent), forwards the bytes verbatim to an upstream nameserver via
//! [`DnsResolver`], and returns the upstream answer or SERVFAIL when the
//! upstream is unreachable.
//!
//! All three paths produce a [`DnsHandlerResult`] carrying the answer
//! bytes plus the structured fields the eventual `dns_events` writer
//! (T3.3) and `mitm.dns_queries_total{decision}` counter need. The
//! handler never logs the dns_events row itself -- T3 splits the
//! schema migration into its own slice, and keeping the handler free
//! of `DbWriter` makes T3.1 testable without spinning up sqlite.
//!
use std::sync::Arc;
use std::time::Instant;

use capsem_logger::events::Decision;
use tracing::{debug, instrument, warn};

use crate::net::dns::cache::DnsAnswerCache;
use crate::net::dns::resolver::DnsResolver;
use crate::net::mitm_proxy::metrics as m;
use capsem_network_engine::dns_parser::{build_servfail, parse_query, DnsQuery};

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
    /// Resolver outcome.
    pub decision: Decision,
    /// Matched policy rule. Always `None` until DNS is wired through the
    /// canonical Security Engine.
    pub matched_rule: Option<String>,
    /// Wall time of the upstream resolve attempt, in milliseconds.
    /// 0 when input parsing fails (Error).
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

/// Async DNS handler shared across vsock connections.
///
/// `cache` is optional: pass `Some(Arc<DnsAnswerCache>)` to enable
/// the TTL-honoring answer cache (T3.f) which short-circuits the
/// upstream UDP RTT on repeated queries to allowed names. The
/// production `with_default_resolver()` constructor enables it by
/// default; tests that want to assert the upstream path always
/// runs use `new(resolver)` which leaves cache=None.
#[derive(Clone)]
pub struct DnsHandler {
    resolver: Arc<DnsResolver>,
    cache: Option<Arc<DnsAnswerCache>>,
}

impl DnsHandler {
    /// Build a handler with no answer cache. Tests use this so a
    /// cache hit can't accidentally hide an upstream-path
    /// regression.
    pub fn new(resolver: Arc<DnsResolver>) -> Self {
        Self {
            resolver,
            cache: None,
        }
    }

    /// Build a handler with an explicit answer cache.
    pub fn with_cache(resolver: Arc<DnsResolver>, cache: Arc<DnsAnswerCache>) -> Self {
        Self {
            resolver,
            cache: Some(cache),
        }
    }

    /// Build a production handler: default UDP forwarder
    /// (DEFAULT_UPSTREAMS, 5s timeout) + default-sized
    /// TTL-honoring answer cache.
    pub fn with_default_resolver() -> Self {
        Self::with_cache(
            Arc::new(DnsResolver::new()),
            Arc::new(DnsAnswerCache::default()),
        )
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

        // T3.f -- answer cache check. Consulted before the upstream-forward
        // path until DNS is wired through the canonical Security Engine.
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(&query.qname, query.qtype, query.qclass, query.id) {
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
