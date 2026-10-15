//! Shared in-memory cache metrics.

use metrics::Unit;

use crate::MetricSpec;

/// Cache lookups, partitioned by cache and result (`hit` or `miss`).
pub const CACHE_LOOKUPS_TOTAL: &str = "cache.lookups_total";
/// Cache mutations, partitioned by cache and operation.
pub const CACHE_MUTATIONS_TOTAL: &str = "cache.mutations_total";
/// Time spent computing a value after a cache miss, partitioned by cache and status.
pub const CACHE_LOAD_DURATION_MS: &str = "cache.load_duration_ms";
/// Current resident entries, partitioned by cache.
pub const CACHE_ENTRIES: &str = "cache.entries";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        CACHE_LOOKUPS_TOTAL,
        Unit::Count,
        "Cache lookups, partitioned by cache and result (hit or miss).",
    ),
    MetricSpec::counter(
        CACHE_MUTATIONS_TOTAL,
        Unit::Count,
        "Cache mutations, partitioned by cache and operation (insert, refresh, invalidate, evict, or load_error).",
    ),
    MetricSpec::histogram(
        CACHE_LOAD_DURATION_MS,
        Unit::Milliseconds,
        "Wall time spent computing a value after a cache miss, partitioned by cache and status.",
    ),
    MetricSpec::gauge(
        CACHE_ENTRIES,
        Unit::Count,
        "Current resident cache entries, partitioned by cache.",
    ),
];
