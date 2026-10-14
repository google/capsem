use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capsem_telemetry::cache as metric;
use metrics::{Counter, Gauge, Histogram};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadStatus {
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub refreshes: u64,
    pub invalidations: u64,
    pub evictions: u64,
    pub load_errors: u64,
    pub entries: u64,
}

#[derive(Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    insertions: AtomicU64,
    refreshes: AtomicU64,
    invalidations: AtomicU64,
    evictions: AtomicU64,
    load_errors: AtomicU64,
    entries: AtomicU64,
}

#[derive(Clone)]
struct ExportedMetrics {
    hit: Counter,
    miss: Counter,
    insert: Counter,
    refresh: Counter,
    invalidate: Counter,
    evict: Counter,
    load_error: Counter,
    load_success_duration: Histogram,
    load_error_duration: Histogram,
    entries: Gauge,
}

/// Uniform counters and exported metrics for a domain-specific cache.
///
/// Most callers should use the shared cache primitives, which record these
/// automatically. A cache with domain-specific async or database locking can
/// use this type while retaining its required freshness mechanics.
#[derive(Clone)]
pub struct CacheMetrics {
    counters: Arc<Counters>,
    exported: ExportedMetrics,
}

impl CacheMetrics {
    pub fn new(name: &'static str) -> Self {
        let telemetry = Self {
            counters: Arc::default(),
            exported: ExportedMetrics {
                hit: ::metrics::counter!(metric::CACHE_LOOKUPS_TOTAL, "cache" => name, "result" => "hit"),
                miss: ::metrics::counter!(metric::CACHE_LOOKUPS_TOTAL, "cache" => name, "result" => "miss"),
                insert: ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => name, "operation" => "insert"),
                refresh: ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => name, "operation" => "refresh"),
                invalidate: ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => name, "operation" => "invalidate"),
                evict: ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => name, "operation" => "evict"),
                load_error: ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => name, "operation" => "load_error"),
                load_success_duration: ::metrics::histogram!(
                    metric::CACHE_LOAD_DURATION_MS,
                    "cache" => name,
                    "status" => "success"
                ),
                load_error_duration: ::metrics::histogram!(
                    metric::CACHE_LOAD_DURATION_MS,
                    "cache" => name,
                    "status" => "error"
                ),
                entries: ::metrics::gauge!(metric::CACHE_ENTRIES, "cache" => name),
            },
        };
        telemetry.set_entries(0);
        telemetry
    }

    pub fn hit(&self) {
        self.counters.hits.fetch_add(1, Ordering::Relaxed);
        self.exported.hit.increment(1);
    }

    pub fn miss(&self) {
        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        self.exported.miss.increment(1);
    }

    pub fn insertion(&self) {
        self.mutation(&self.counters.insertions, &self.exported.insert, 1);
    }

    pub fn refresh(&self) {
        self.mutation(&self.counters.refreshes, &self.exported.refresh, 1);
    }

    pub fn invalidate(&self, count: u64) {
        self.mutation(&self.counters.invalidations, &self.exported.invalidate, count);
    }

    pub fn evict(&self) {
        self.mutation(&self.counters.evictions, &self.exported.evict, 1);
    }

    fn mutation(&self, counter: &AtomicU64, exported: &Counter, count: u64) {
        if count == 0 {
            return;
        }
        counter.fetch_add(count, Ordering::Relaxed);
        exported.increment(count);
    }

    pub fn load(&self, elapsed: Duration, status: LoadStatus) {
        let histogram = match status {
            LoadStatus::Success => &self.exported.load_success_duration,
            LoadStatus::Error => {
                self.mutation(&self.counters.load_errors, &self.exported.load_error, 1);
                &self.exported.load_error_duration
            }
        };
        histogram.record(elapsed.as_secs_f64() * 1000.0);
    }

    pub fn set_entries(&self, entries: usize) {
        let entries = u64::try_from(entries).unwrap_or(u64::MAX);
        self.counters.entries.store(entries, Ordering::Relaxed);
        self.exported.entries.set(entries as f64);
    }

    pub fn snapshot(&self) -> CacheStats {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        CacheStats {
            hits: load(&self.counters.hits),
            misses: load(&self.counters.misses),
            insertions: load(&self.counters.insertions),
            refreshes: load(&self.counters.refreshes),
            invalidations: load(&self.counters.invalidations),
            evictions: load(&self.counters.evictions),
            load_errors: load(&self.counters.load_errors),
            entries: load(&self.counters.entries),
        }
    }
}
