//! TTL-honoring LRU answer cache for the DNS proxy (T3.f).
//!
//! The cache shaves the upstream UDP RTT off repeated queries to the
//! same allowed name. Cache shape:
//!
//! * **Key**: `(qname, qtype, qclass)` -- the operationally relevant
//!   slice of a DNS question.
//! * **Value**: the wire-format answer bytes + an `expires_at`
//!   `Instant` derived from `min(answer_TTL_seconds, max_cache_ttl)`.
//!   Expiry is enforced lazily on lookup: an expired entry is
//!   removed and counted as a miss.
//! * **Eligibility**: only `Decision::Allowed` answers are cached.
//!   Security blocks run before the cache. Redirect settings are still
//!   re-checked on every query, and SERVFAIL responses should not be
//!   persisted.
//! * **Bound**: an LRU on entry count (default 1024). Evictions are
//!   counted via the `mitm.dns_cache_evictions_total` counter.
//!
//! The cache **does** read the network-policy snapshot on every hit so
//! redirect/cache mechanics stay coherent without a per-policy version
//! counter.

use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hickory_proto::op::Message;
use lru::LruCache;
use tracing::trace;

use crate::net::mitm_proxy::metrics as m;
use crate::net::policy::NetworkPolicy;

/// Default cache capacity (entries). Picked to keep ~64 KB of memory
/// in the worst case (1024 * 64-byte answers); bounds RSS without
/// constraining real workloads (a single curl invocation typically
/// resolves <= 5 names, so 1024 covers many minutes of agent
/// activity).
pub const DEFAULT_CAPACITY: usize = 1024;

/// Default upper bound on cache TTL, in seconds. DNS records can
/// advertise TTLs up to 7 days; a long-lived cache entry would
/// outlive most agent sessions and risk staleness on infrastructure
/// change. Cap at 5 minutes -- aligns with the typical interactive
/// agent loop and the existing CDN TTLs (Cloudflare default 5 min).
pub const DEFAULT_MAX_TTL_SECS: u32 = 300;

/// Minimum cache TTL, in seconds. Some authoritative servers set a
/// 0-second TTL ("don't cache") which would make the cache useless
/// on retry storms; clamp to at least 60s so a burst still benefits.
pub const MIN_TTL_SECS: u32 = 60;

#[derive(Clone)]
struct Entry {
    bytes: Vec<u8>,
    expires_at: Instant,
}

/// Bounded LRU cache for DNS answer bytes.
///
/// Thread-safe via a single `Mutex<LruCache<...>>`. Lookups and
/// inserts go through the same lock; under contention this is the
/// bottleneck, but the hot-path cost (one HashMap lookup + one
/// Instant::now()) is sub-microsecond on modern hardware.
pub struct DnsAnswerCache {
    inner: Mutex<LruCache<CacheKey, Entry>>,
    max_ttl: Duration,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct CacheKey {
    qname: String,
    qtype: u16,
    qclass: u16,
}

impl Default for DnsAnswerCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_MAX_TTL_SECS)
    }
}

impl DnsAnswerCache {
    /// Build a cache with explicit capacity + TTL ceiling.
    /// `capacity` of 0 is silently bumped to 1 (LruCache requires
    /// non-zero); `max_ttl_secs` of 0 disables caching effectively
    /// (every entry expires before its first lookup).
    pub fn new(capacity: usize, max_ttl_secs: u32) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).expect("capacity > 0 enforced");
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            max_ttl: Duration::from_secs(max_ttl_secs.max(1) as u64),
        }
    }

    /// Look up the answer for `(qname, qtype, qclass)`.
    ///
    /// Returns `Some(bytes)` only if:
    /// * The entry exists.
    /// * It has not expired.
    /// * `policy.find_dns_redirect(qname, qtype)` is None (not
    ///   now-redirected).
    ///
    /// On every other shape we return None and let the caller fall
    /// through to the policy + upstream path (where the new policy
    /// state is naturally honored).
    ///
    /// `query_id` is the transaction id of the *current* query and
    /// is patched into bytes 0-1 of the returned response. Without
    /// this, every cache hit would echo the FIRST query's id and
    /// downstream resolvers (which match responses by id) would
    /// reject every hit -- surfaced in the in-VM dns-load bench
    /// during T3 closure as "id mismatch" on 100% of queries.
    pub fn get(
        &self,
        qname: &str,
        qtype: u16,
        qclass: u16,
        query_id: u16,
        policy: &NetworkPolicy,
    ) -> Option<Vec<u8>> {
        let key = CacheKey {
            qname: qname.to_string(),
            qtype,
            qclass,
        };
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap();
        let entry = guard.get(&key)?;
        if entry.expires_at <= now {
            // Lazy expiry: drop the stale entry so the next
            // lookup is a clean miss without re-checking expiry.
            guard.pop(&key);
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
            trace!(qname, qtype, "dns cache: expired entry evicted");
            return None;
        }
        // Coherence: re-check redirect mechanics on every hit. Security-rule
        // enforcement happens before cache lookup in the DNS handler, so this
        // cache layer does not own allow/block decisions.
        if policy.find_dns_redirect(qname, qtype).is_some() {
            guard.pop(&key);
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
            trace!(
                qname,
                qtype,
                "dns cache: entry invalidated by redirect change"
            );
            return None;
        }
        let mut bytes = entry.bytes.clone();
        // Patch the current query's transaction id into bytes 0-1
        // (RFC 1035 sec 4.1.1: the ID field is the first 16 bits of
        // the DNS header, big-endian). The cached answer was stored
        // with the original requesting query's id; subsequent
        // queries to the same name MUST get their own id back or
        // their resolver discards the response.
        if bytes.len() >= 2 {
            let id_be = query_id.to_be_bytes();
            bytes[0] = id_be[0];
            bytes[1] = id_be[1];
        }
        ::metrics::counter!(m::DNS_CACHE_HITS_TOTAL).increment(1);
        trace!(qname, qtype, query_id, "dns cache: hit");
        Some(bytes)
    }

    /// Insert an Allowed response for future hits. The TTL is
    /// derived from the answer wire bytes (minimum across all
    /// answer records, clamped to `[MIN_TTL_SECS, max_ttl]`). On
    /// LRU eviction, the `mitm.dns_cache_evictions_total` counter
    /// fires.
    pub fn insert(&self, qname: &str, qtype: u16, qclass: u16, answer_bytes: &[u8]) {
        let ttl = ttl_from_answer(answer_bytes, self.max_ttl);
        let entry = Entry {
            bytes: answer_bytes.to_vec(),
            expires_at: Instant::now() + ttl,
        };
        let key = CacheKey {
            qname: qname.to_string(),
            qtype,
            qclass,
        };
        let mut guard = self.inner.lock().unwrap();
        let evicted = guard.push(key, entry);
        if evicted.is_some() {
            ::metrics::counter!(m::DNS_CACHE_EVICTIONS_TOTAL).increment(1);
        }
    }

    /// Drop every cached entry. Used when the policy is hot-swapped
    /// in bulk (e.g. corp config reload) -- cheaper than letting
    /// each entry independently re-validate against the new policy
    /// on its next lookup.
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Current entry count (debugging / metrics only).
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Extract the cache TTL from an answer message.
///
/// Logic: take `min(record.ttl)` across every answer record, clamp
/// to `[MIN_TTL_SECS, max_ttl]`. Empty answer section (NoData) gets
/// `MIN_TTL_SECS` so we still cache the negative-shape answer
/// briefly. Decode failure short-circuits to `MIN_TTL_SECS` to
/// avoid hot-looping on a malformed answer that the resolver
/// somehow accepted.
fn ttl_from_answer(answer_bytes: &[u8], max_ttl: Duration) -> Duration {
    let answer_ttl = match Message::from_vec(answer_bytes) {
        Ok(m) if !m.answers.is_empty() => m
            .answers
            .iter()
            .map(|r| r.ttl)
            .min()
            .unwrap_or(MIN_TTL_SECS),
        _ => MIN_TTL_SECS,
    };
    let clamped = answer_ttl.max(MIN_TTL_SECS) as u64;
    Duration::from_secs(clamped).min(max_ttl)
}

#[cfg(test)]
mod tests;
