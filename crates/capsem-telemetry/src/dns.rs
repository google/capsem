//! DNS resolver and answer-cache metrics.
//!
//! The resolver runs inside the MITM proxy, so its names carry the `mitm.`
//! prefix they have always been recorded under.

use metrics::Unit;

use crate::MetricSpec;

pub const DNS_QUERIES_TOTAL: &str = "mitm.dns_queries_total";
pub const DNS_UPSTREAM_FAILURES_TOTAL: &str = "mitm.dns_upstream_failures_total";
pub const DNS_UPSTREAM_COALESCED_TOTAL: &str = "mitm.dns_upstream_coalesced_total";
pub const DNS_CACHE_HITS_TOTAL: &str = "mitm.dns_cache_hits_total";
pub const DNS_CACHE_MISSES_TOTAL: &str = "mitm.dns_cache_misses_total";
pub const DNS_CACHE_EVICTIONS_TOTAL: &str = "mitm.dns_cache_evictions_total";
pub const DNS_HANDLE_DURATION_MS: &str = "mitm.dns_handle_duration_ms";
pub const DNS_UPSTREAM_DURATION_MS: &str = "mitm.dns_upstream_duration_ms";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        DNS_QUERIES_TOTAL,
        Unit::Count,
        "DNS queries handled by the resolver, partitioned by decision (allowed|denied|redirected|error).",
    ),
    MetricSpec::counter(
        DNS_UPSTREAM_FAILURES_TOTAL,
        Unit::Count,
        "Upstream DNS resolver failures (timeout, network error, all upstreams down).",
    ),
    MetricSpec::counter(
        DNS_UPSTREAM_COALESCED_TOTAL,
        Unit::Count,
        "DNS queries that joined an identical in-flight upstream lookup instead of dialing their own.",
    ),
    MetricSpec::counter(
        DNS_CACHE_HITS_TOTAL,
        Unit::Count,
        "DNS answer cache hits (T3.f). Includes only Decision::Allowed entries -- block + redirect re-evaluate every query.",
    ),
    MetricSpec::counter(
        DNS_CACHE_MISSES_TOTAL,
        Unit::Count,
        "DNS answer cache misses -- query not present, expired, or shape ineligible (denied / error / redirected).",
    ),
    MetricSpec::counter(
        DNS_CACHE_EVICTIONS_TOTAL,
        Unit::Count,
        "DNS answer cache LRU evictions (capacity full).",
    ),
    MetricSpec::histogram(
        DNS_HANDLE_DURATION_MS,
        Unit::Milliseconds,
        "End-to-end wall time inside DnsHandler::handle (parse + policy + upstream OR redirect synthesis).",
    ),
    MetricSpec::histogram(
        DNS_UPSTREAM_DURATION_MS,
        Unit::Milliseconds,
        "Wall time of one upstream DNS resolution attempt (UDP forward + receive). Only emitted on the upstream-forward path.",
    ),
];
