//! TTL-honoring LRU answer cache for the DNS proxy (T3.f).
//!
//! The cache shaves the upstream UDP RTT off repeated queries to the
//! same allowed name. Cache shape:
//!
//! * **Key**: the complete wire query except its transaction id, so flags,
//!   question spelling and EDNS options never borrow another query's answer.
//! * **Value**: the wire-format answer bytes + an `expires_at`
//!   `Instant` derived from `min(answer_TTL_seconds, max_cache_ttl)`.
//!   Expiry is enforced lazily on lookup: an expired entry is
//!   removed and counted as a miss.
//! * **Eligibility**: only `Decision::Allowed` answers are cached.
//!   Security blocks run before the cache. Redirect settings are still
//!   re-checked on every query. An upstream NXDOMAIN is cached briefly
//!   (`insert_negative`, bounded by the SOA minimum and
//!   `NEGATIVE_MAX_TTL_SECS`); SERVFAIL is never persisted, since it may
//!   be a transient upstream fault.
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
use hickory_proto::rr::RData;
use lru::LruCache;

use super::coalesce::LookupKey;
use crate::net::parsers::dns_parser::DnsQuery;
use tracing::trace;

use crate::net::mitm_proxy::metrics as m;
use crate::net::policy::NetworkMechanics;

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

/// Ceiling on how long an upstream NXDOMAIN is remembered. RFC 2308 lets a
/// negative answer live for the SOA minimum; a minute is enough to absorb a
/// client retrying a dead name without amplifying a name that reappears.
pub const NEGATIVE_MAX_TTL_SECS: u32 = 60;

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
    inner: Mutex<LruCache<LookupKey, Entry>>,
    max_ttl: Duration,
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
            max_ttl: Duration::from_secs(u64::from(max_ttl_secs)),
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
    pub(super) fn get(&self, query: &DnsQuery, key: &LookupKey, policy: &NetworkMechanics) -> Option<Vec<u8>> {
        let qname = query.qname.as_str();
        let qtype = query.qtype;
        let query_id = query.id;
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap();
        let Some(entry) = guard.get(key) else {
            drop(guard);
            return None;
        };
        if entry.expires_at <= now {
            // Lazy expiry: drop the stale entry so the next
            // lookup is a clean miss without re-checking expiry.
            guard.pop(key);
            drop(guard);
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
            trace!(qname, qtype, "dns cache: expired entry evicted");
            return None;
        }
        // Coherence: re-check redirect mechanics on every hit. Security-rule
        // enforcement happens before cache lookup in the DNS handler, so this
        // cache layer does not own allow/block decisions.
        if policy.find_dns_redirect(qname, qtype).is_some() {
            guard.pop(key);
            drop(guard);
            ::metrics::counter!(m::DNS_CACHE_MISSES_TOTAL).increment(1);
            trace!(qname, qtype, "dns cache: entry invalidated by redirect change");
            return None;
        }
        let bytes = entry.bytes.clone();
        let remaining = entry.expires_at.duration_since(now).as_secs_f64().ceil() as u32;
        drop(guard);
        // Patch the current query's transaction id into bytes 0-1
        // (RFC 1035 sec 4.1.1: the ID field is the first 16 bits of
        // the DNS header, big-endian). The cached answer was stored
        // with the original requesting query's id; subsequent
        // queries to the same name MUST get their own id back or
        // their resolver discards the response.
        let mut message = Message::from_vec(&bytes).ok()?;
        message.metadata.id = query_id;
        for record in message
            .answers
            .iter_mut()
            .chain(message.authorities.iter_mut())
            .chain(message.additionals.iter_mut())
        {
            // OPT's TTL-shaped field carries EDNS flags, not a lifetime.
            if !matches!(record.data, RData::OPT(_)) {
                record.ttl = record.ttl.min(remaining);
            }
        }
        let bytes = message.to_vec().ok()?;
        ::metrics::counter!(m::DNS_CACHE_HITS_TOTAL).increment(1);
        trace!(qname, qtype, query_id, "dns cache: hit");
        Some(bytes)
    }

    /// Insert an Allowed response for future hits. The TTL is
    /// derived from the answer wire bytes (minimum across all
    /// answer records, capped by `max_ttl`). Zero prohibits caching. On
    /// LRU eviction, the `mitm.dns_cache_evictions_total` counter
    /// fires.
    pub(super) fn insert(&self, key: &LookupKey, answer_bytes: &[u8]) {
        let ttl = ttl_from_answer(answer_bytes, self.max_ttl);
        if ttl.is_zero() {
            return;
        }
        let entry = Entry {
            bytes: answer_bytes.to_vec(),
            expires_at: Instant::now() + ttl,
        };
        let key = key.clone();
        let mut guard = self.inner.lock().unwrap();
        let evicted = guard.push(key, entry);
        drop(guard);
        if evicted.is_some() {
            ::metrics::counter!(m::DNS_CACHE_EVICTIONS_TOTAL).increment(1);
        }
    }

    /// Remember an upstream NXDOMAIN for this exact query. The TTL
    /// is the smaller of the authority section's SOA minimum (RFC 2308) and
    /// `NEGATIVE_MAX_TTL_SECS`; zero or absent SOA disables reuse. Only an Allowed
    /// query reaches the upstream, so a denied name can never land here.
    pub(super) fn insert_negative(&self, key: &LookupKey, answer_bytes: &[u8]) {
        let ttl = negative_ttl_from_answer(answer_bytes).min(self.max_ttl);
        if ttl.is_zero() {
            return;
        }
        let entry = Entry {
            bytes: answer_bytes.to_vec(),
            expires_at: Instant::now() + ttl,
        };
        let key = key.clone();
        let evicted = self.inner.lock().unwrap().push(key, entry);
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

/// Extract the cache TTL from a positive answer message.
///
/// Use the shortest record lifetime, never extending it. Empty answers
/// (NODATA) need the same authoritative SOA lifetime as NXDOMAIN.
/// Malformed or truncated answers are not cached.
fn ttl_from_answer(answer_bytes: &[u8], max_ttl: Duration) -> Duration {
    let answer_ttl = match Message::from_vec(answer_bytes) {
        Ok(m) if !m.metadata.truncation && !m.answers.is_empty() => m
            .answers
            .iter()
            .chain(m.authorities.iter())
            .chain(m.additionals.iter())
            .filter(|r| !matches!(r.data, RData::OPT(_)))
            .map(|r| r.ttl)
            .min()
            .unwrap_or(0),
        Ok(m) if !m.metadata.truncation => return negative_ttl_from_answer(answer_bytes).min(max_ttl),
        _ => 0,
    };
    Duration::from_secs(u64::from(answer_ttl)).min(max_ttl)
}

/// The TTL for a negative answer: the SOA minimum from the authority
/// section when the upstream sent one, capped at `NEGATIVE_MAX_TTL_SECS`.
/// Without a SOA (or with undecodable bytes) there is no authoritative
/// negative lifetime; zero must not be promoted to a cacheable lifetime.
fn negative_ttl_from_answer(answer_bytes: &[u8]) -> Duration {
    let soa_minimum = Message::from_vec(answer_bytes).ok().and_then(|message| {
        if message.metadata.truncation {
            return None;
        }
        message
            .authorities
            .iter()
            .filter_map(|record| match &record.data {
                RData::SOA(soa) => Some(record.ttl.min(soa.minimum)),
                _ => None,
            })
            .min()
    });
    let secs = soa_minimum.unwrap_or(0).min(NEGATIVE_MAX_TTL_SECS);
    Duration::from_secs(u64::from(secs))
}

#[cfg(test)]
mod tests;
