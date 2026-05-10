//! Counter / histogram / gauge declarations for the MITM pipeline.
//!
//! All names from the plan's "Metrics (counters + histograms,
//! OTel-ready, zero-cost without a recorder)" section.
//!
//! No recorder is registered in this sprint -- every `counter!()` /
//! `histogram!()` / `gauge!()` call resolves to a single relaxed atomic
//! add against the global no-op recorder. T5 wires the exporter (likely
//! OTel via `opentelemetry-otlp`).
//!
//! Convention:
//! - `mitm.<area>_total{label=...}` for monotonic counters.
//! - `mitm.<measure>_ms{...}` for histograms with millisecond units.
//! - `mitm.<gauge>` for instantaneous gauges (active connections, pool size).
//!
//! All names are checked into `crates/capsem-core/benches/baselines/` via
//! the `mitm_pipeline` benchmark so a casual rename surfaces in CI.

#![allow(dead_code)]

use metrics::{describe_counter, describe_gauge, describe_histogram, Unit};

// ── Counter names ───────────────────────────────────────────────────

pub const CONNECTIONS_TOTAL: &str = "mitm.connections_total";
pub const REQUESTS_TOTAL: &str = "mitm.requests_total";
pub const POLICY_DECISIONS_TOTAL: &str = "mitm.policy_decisions_total";
pub const DNS_QUERIES_TOTAL: &str = "mitm.dns_queries_total";
pub const DNS_UPSTREAM_FAILURES_TOTAL: &str = "mitm.dns_upstream_failures_total";
pub const DNS_CACHE_HITS_TOTAL: &str = "mitm.dns_cache_hits_total";
pub const DNS_CACHE_MISSES_TOTAL: &str = "mitm.dns_cache_misses_total";
pub const DNS_CACHE_EVICTIONS_TOTAL: &str = "mitm.dns_cache_evictions_total";
pub const MCP_METHODS_TOTAL: &str = "mitm.mcp_methods_total";
pub const MCP_DISCONNECTS_TOTAL: &str = "mitm.mcp_disconnects_total";
pub const PARSER_EVENTS_TOTAL: &str = "mitm.parser_events_total";
pub const HOOK_INVOCATIONS_TOTAL: &str = "mitm.hook_invocations_total";
pub const TELEMETRY_DROPPED_TOTAL: &str = "mitm.telemetry_dropped_total";

// ── Histogram names ─────────────────────────────────────────────────

pub const TLS_HANDSHAKE_MS: &str = "mitm.tls_handshake_ms";
pub const UPSTREAM_DIAL_MS: &str = "mitm.upstream_dial_ms";
pub const HOOK_DURATION_MS: &str = "mitm.hook_duration_ms";
pub const REQUEST_BODY_BYTES: &str = "mitm.request_body_bytes";
pub const RESPONSE_BODY_BYTES: &str = "mitm.response_body_bytes";
pub const DNS_HANDLE_DURATION_MS: &str = "mitm.dns_handle_duration_ms";
pub const DNS_UPSTREAM_DURATION_MS: &str = "mitm.dns_upstream_duration_ms";

// ── Gauge names ─────────────────────────────────────────────────────

pub const ACTIVE_CONNECTIONS: &str = "mitm.active_connections";
pub const UPSTREAM_POOL_SIZE: &str = "mitm.upstream_pool_size";
pub const RUNTIME_BUSY_RATIO: &str = "mitm.runtime_busy_ratio";

/// Register descriptions with the metrics facade. Idempotent. No-op when
/// no recorder is installed (the default in this sprint).
///
/// Wired hooks register their own counter/histogram via
/// `metrics::counter!(NAME, "label" => value).increment(1)`; this function
/// just attaches human-readable text + units so a future exporter can
/// expose them with proper metadata.
pub fn describe_all() {
    describe_counter!(
        CONNECTIONS_TOTAL,
        Unit::Count,
        "Connections accepted by the MITM listener, partitioned by protocol (tls|http|dns)."
    );
    describe_counter!(
        REQUESTS_TOTAL,
        Unit::Count,
        "HTTP requests handled, partitioned by protocol + decision (allow|deny|stop)."
    );
    describe_counter!(
        POLICY_DECISIONS_TOTAL,
        Unit::Count,
        "Domain/HTTP policy evaluations, partitioned by decision."
    );
    describe_counter!(
        DNS_QUERIES_TOTAL,
        Unit::Count,
        "DNS queries handled by the resolver, partitioned by decision (allowed|denied|redirected|error)."
    );
    describe_counter!(
        DNS_UPSTREAM_FAILURES_TOTAL,
        Unit::Count,
        "Upstream DNS resolver failures (timeout, network error, all upstreams down)."
    );
    describe_counter!(
        DNS_CACHE_HITS_TOTAL,
        Unit::Count,
        "DNS answer cache hits (T3.f). Includes only Decision::Allowed entries -- block + redirect re-evaluate every query."
    );
    describe_counter!(
        DNS_CACHE_MISSES_TOTAL,
        Unit::Count,
        "DNS answer cache misses -- query not present, expired, or shape ineligible (denied / error / redirected)."
    );
    describe_counter!(
        DNS_CACHE_EVICTIONS_TOTAL,
        Unit::Count,
        "DNS answer cache LRU evictions (capacity full)."
    );
    describe_counter!(
        MCP_METHODS_TOTAL,
        Unit::Count,
        "MCP JSON-RPC method invocations seen by the proxy, partitioned by method."
    );
    describe_counter!(
        MCP_DISCONNECTS_TOTAL,
        Unit::Count,
        "Framed MCP transport disconnects, partitioned by reason."
    );
    describe_counter!(
        PARSER_EVENTS_TOTAL,
        Unit::Count,
        "Higher-level events emitted by parser hooks, partitioned by parser + kind."
    );
    describe_counter!(
        HOOK_INVOCATIONS_TOTAL,
        Unit::Count,
        "Hook on_event() calls dispatched, partitioned by hook name."
    );
    describe_counter!(
        TELEMETRY_DROPPED_TOTAL,
        Unit::Count,
        "Telemetry events dropped because the logger writer queue was full."
    );

    describe_histogram!(
        TLS_HANDSHAKE_MS,
        Unit::Milliseconds,
        "Time spent in TLS termination + cert lookup/mint."
    );
    describe_histogram!(
        UPSTREAM_DIAL_MS,
        Unit::Milliseconds,
        "Time spent dialing the upstream (TCP + TLS handshake)."
    );
    describe_histogram!(
        HOOK_DURATION_MS,
        Unit::Milliseconds,
        "Wall time spent inside a single hook on_event() call."
    );
    describe_histogram!(
        REQUEST_BODY_BYTES,
        Unit::Bytes,
        "Bytes observed in request bodies (post-decompression)."
    );
    describe_histogram!(
        RESPONSE_BODY_BYTES,
        Unit::Bytes,
        "Bytes observed in response bodies (post-decompression)."
    );
    describe_histogram!(
        DNS_HANDLE_DURATION_MS,
        Unit::Milliseconds,
        "End-to-end wall time inside DnsHandler::handle (parse + policy + upstream OR redirect synthesis)."
    );
    describe_histogram!(
        DNS_UPSTREAM_DURATION_MS,
        Unit::Milliseconds,
        "Wall time of one upstream DNS resolution attempt (UDP forward + receive). Only emitted on the upstream-forward path."
    );

    describe_gauge!(
        ACTIVE_CONNECTIONS,
        Unit::Count,
        "Currently active proxy connections."
    );
    describe_gauge!(
        UPSTREAM_POOL_SIZE,
        Unit::Count,
        "Size of the upstream connection pool, summed across all (domain, port) entries."
    );
    describe_gauge!(
        RUNTIME_BUSY_RATIO,
        Unit::Percent,
        "tokio-metrics busy ratio of the proxy runtime, sampled per scrape."
    );
}

#[cfg(test)]
mod tests;
