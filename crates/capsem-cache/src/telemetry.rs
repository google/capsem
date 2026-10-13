use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capsem_telemetry::cache as metric;

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

/// Uniform counters and exported metrics for a domain-specific cache.
///
/// Most callers should use the shared cache primitives, which record these
/// automatically. A cache with domain-specific async or database locking can
/// use this type while retaining its required freshness mechanics.
#[derive(Clone)]
pub struct CacheMetrics {
    name: &'static str,
    counters: Arc<Counters>,
}

impl CacheMetrics {
    pub fn new(name: &'static str) -> Self {
        let telemetry = Self {
            name,
            counters: Arc::default(),
        };
        telemetry.set_entries(0);
        telemetry
    }

    pub fn hit(&self) {
        self.counters.hits.fetch_add(1, Ordering::Relaxed);
        ::metrics::counter!(metric::CACHE_LOOKUPS_TOTAL, "cache" => self.name, "result" => "hit").increment(1);
    }

    pub fn miss(&self) {
        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        ::metrics::counter!(metric::CACHE_LOOKUPS_TOTAL, "cache" => self.name, "result" => "miss").increment(1);
    }

    pub fn insertion(&self) {
        self.mutation(&self.counters.insertions, "insert", 1);
    }

    pub fn refresh(&self) {
        self.mutation(&self.counters.refreshes, "refresh", 1);
    }

    pub fn invalidate(&self, count: u64) {
        self.mutation(&self.counters.invalidations, "invalidate", count);
    }

    pub fn evict(&self) {
        self.mutation(&self.counters.evictions, "evict", 1);
    }

    fn mutation(&self, counter: &AtomicU64, operation: &'static str, count: u64) {
        if count == 0 {
            return;
        }
        counter.fetch_add(count, Ordering::Relaxed);
        ::metrics::counter!(metric::CACHE_MUTATIONS_TOTAL, "cache" => self.name, "operation" => operation)
            .increment(count);
    }

    pub fn load(&self, elapsed: Duration, status: LoadStatus) {
        let status = match status {
            LoadStatus::Success => "success",
            LoadStatus::Error => {
                self.mutation(&self.counters.load_errors, "load_error", 1);
                "error"
            }
        };
        ::metrics::histogram!(metric::CACHE_LOAD_DURATION_MS, "cache" => self.name, "status" => status)
            .record(elapsed.as_secs_f64() * 1000.0);
    }

    pub fn set_entries(&self, entries: usize) {
        let entries = u64::try_from(entries).unwrap_or(u64::MAX);
        self.counters.entries.store(entries, Ordering::Relaxed);
        ::metrics::gauge!(metric::CACHE_ENTRIES, "cache" => self.name).set(entries as f64);
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
