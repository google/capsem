//! MITM proxy metrics: connections, requests, hooks, MCP framing and the
//! telemetry handoff.
//!
//! Convention: `mitm.<area>_total` for counters, `mitm.<measure>_ms` for
//! millisecond histograms, `mitm.<gauge>` for instantaneous gauges. The DNS
//! resolver's `mitm.dns_*` metrics live in [`crate::dns`].

use metrics::Unit;

use crate::MetricSpec;

pub const CONNECTIONS_TOTAL: &str = "mitm.connections_total";
pub const REQUESTS_TOTAL: &str = "mitm.requests_total";
pub const POLICY_DECISIONS_TOTAL: &str = "mitm.policy_decisions_total";
pub const MCP_METHODS_TOTAL: &str = "mitm.mcp_methods_total";
pub const MCP_DISCONNECTS_TOTAL: &str = "mitm.mcp_disconnects_total";
pub const PARSER_EVENTS_TOTAL: &str = "mitm.parser_events_total";
pub const HOOK_INVOCATIONS_TOTAL: &str = "mitm.hook_invocations_total";
pub const TELEMETRY_DROPPED_TOTAL: &str = "mitm.telemetry_dropped_total";

pub const TLS_HANDSHAKE_MS: &str = "mitm.tls_handshake_ms";
pub const UPSTREAM_DIAL_MS: &str = "mitm.upstream_dial_ms";
pub const HOOK_DURATION_MS: &str = "mitm.hook_duration_ms";
pub const TELEMETRY_RESPONSE_END_DURATION_MS: &str = "mitm.telemetry_response_end_duration_ms";
pub const TELEMETRY_STAGE_DURATION_MS: &str = "mitm.telemetry_stage_duration_ms";
pub const REQUEST_BODY_BYTES: &str = "mitm.request_body_bytes";
pub const RESPONSE_BODY_BYTES: &str = "mitm.response_body_bytes";

pub const ACTIVE_CONNECTIONS: &str = "mitm.active_connections";
pub const UPSTREAM_POOL_SIZE: &str = "mitm.upstream_pool_size";
pub const RUNTIME_BUSY_RATIO: &str = "mitm.runtime_busy_ratio";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        CONNECTIONS_TOTAL,
        Unit::Count,
        "Connections accepted by the MITM listener, partitioned by protocol (tls|http|dns).",
    ),
    MetricSpec::counter(
        REQUESTS_TOTAL,
        Unit::Count,
        "HTTP requests handled, partitioned by protocol + decision (allow|deny|stop).",
    ),
    MetricSpec::counter(
        POLICY_DECISIONS_TOTAL,
        Unit::Count,
        "Domain/HTTP policy evaluations, partitioned by decision.",
    ),
    MetricSpec::counter(
        MCP_METHODS_TOTAL,
        Unit::Count,
        "MCP JSON-RPC method invocations seen by the proxy, partitioned by method.",
    ),
    MetricSpec::counter(
        MCP_DISCONNECTS_TOTAL,
        Unit::Count,
        "Framed MCP transport disconnects, partitioned by reason.",
    ),
    MetricSpec::counter(
        PARSER_EVENTS_TOTAL,
        Unit::Count,
        "Higher-level events emitted by parser hooks, partitioned by parser + kind.",
    ),
    MetricSpec::counter(
        HOOK_INVOCATIONS_TOTAL,
        Unit::Count,
        "Hook on_event() calls dispatched, partitioned by hook name.",
    ),
    MetricSpec::counter(
        TELEMETRY_DROPPED_TOTAL,
        Unit::Count,
        "Telemetry events dropped because the logger writer queue was full.",
    ),
    MetricSpec::histogram(
        TLS_HANDSHAKE_MS,
        Unit::Milliseconds,
        "Time spent in TLS termination + cert lookup/mint.",
    ),
    MetricSpec::histogram(
        UPSTREAM_DIAL_MS,
        Unit::Milliseconds,
        "Time spent dialing the upstream (TCP + TLS handshake).",
    ),
    MetricSpec::histogram(
        HOOK_DURATION_MS,
        Unit::Milliseconds,
        "Wall time spent inside a single hook on_event() call.",
    ),
    MetricSpec::histogram(
        TELEMETRY_RESPONSE_END_DURATION_MS,
        Unit::Milliseconds,
        "Wall time spent in the HTTP telemetry response-end handler before handoff to async ledger work.",
    ),
    MetricSpec::histogram(
        TELEMETRY_STAGE_DURATION_MS,
        Unit::Milliseconds,
        "Wall time spent in one named HTTP telemetry stage before handoff to async ledger work.",
    ),
    MetricSpec::histogram(
        REQUEST_BODY_BYTES,
        Unit::Bytes,
        "Bytes observed in request bodies (post-decompression).",
    ),
    MetricSpec::histogram(
        RESPONSE_BODY_BYTES,
        Unit::Bytes,
        "Bytes observed in response bodies (post-decompression).",
    ),
    MetricSpec::gauge(ACTIVE_CONNECTIONS, Unit::Count, "Currently active proxy connections."),
    MetricSpec::gauge(
        UPSTREAM_POOL_SIZE,
        Unit::Count,
        "Size of the upstream connection pool, summed across all (domain, port) entries.",
    ),
    MetricSpec::gauge(
        RUNTIME_BUSY_RATIO,
        Unit::Percent,
        "tokio-metrics busy ratio of the proxy runtime, sampled per scrape.",
    ),
];
