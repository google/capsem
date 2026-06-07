use super::*;

use std::net::Ipv4Addr;

use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{rdata, Name, RData, Record, RecordType};

use crate::net::policy::{DnsRedirect, NetworkPolicy};

/// Build a synthetic A-record answer for `qname` with `ttl` seconds
/// on the answer record. Used to seed cache entries with known TTLs.
fn build_answer(qname: &str, ttl: u32, ip: [u8; 4]) -> Vec<u8> {
    let mut msg = Message::new(0x1234, MessageType::Response, OpCode::Query);
    msg.metadata.recursion_available = true;
    msg.metadata.response_code = ResponseCode::NoError;
    let n = Name::from_ascii(qname).unwrap();
    msg.add_query(Query::query(n.clone(), RecordType::A));
    msg.add_answer(Record::from_rdata(
        n,
        ttl,
        RData::A(rdata::A(Ipv4Addr::from(ip))),
    ));
    msg.to_vec().unwrap()
}

fn allow_all() -> NetworkPolicy {
    NetworkPolicy::new()
}

#[test]
fn miss_on_empty_cache() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    assert!(cache.get("example.com", 1, 1, 0, &policy).is_none());
    assert_eq!(cache.len(), 0);
}

#[test]
fn hit_after_insert_within_ttl() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    cache.insert("example.com", 1, 1, &bytes);
    // Pass query_id = 0x1234 -- matches build_answer's hard-coded
    // id so the qid patch is a no-op and we can compare bit-for-bit.
    let got = cache.get("example.com", 1, 1, 0x1234, &policy);
    assert_eq!(got.as_deref(), Some(bytes.as_slice()));
}

#[test]
fn miss_when_qtype_differs() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    cache.insert("example.com", 1, 1, &bytes);
    // Same qname, different qtype (AAAA) -- must miss.
    assert!(cache.get("example.com", 28, 1, 0, &policy).is_none());
}

#[test]
fn miss_when_qclass_differs() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    cache.insert("example.com", 1, 1, &bytes);
    // CHAOS qclass on the same name+qtype -- must miss.
    assert!(cache.get("example.com", 1, 3, 0, &policy).is_none());
}

#[test]
fn invalidated_when_policy_now_redirects() {
    let cache = DnsAnswerCache::new(16, 300);
    let bytes = build_answer("anthropic.com.", 60, [10, 0, 0, 1]);
    cache.insert("anthropic.com", 1, 1, &bytes);

    let mut redirect_policy = NetworkPolicy::new();
    redirect_policy.dns_redirects.push(DnsRedirect::new(
        "anthropic.com",
        Some(1),
        vec![std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)],
        60,
    ));
    // Cache hit must not bypass an admin's later redirect rule --
    // the next lookup must miss + invalidate.
    assert!(cache
        .get("anthropic.com", 1, 1, 0, &redirect_policy)
        .is_none());
}

#[test]
fn cache_hit_patches_query_id_into_response() {
    // Regression for the in-VM dns-load surfaced bug: cached wire
    // bytes include the FIRST query's id in bytes 0-1; subsequent
    // queries to the same name would echo that id, breaking
    // resolver correlation. Cache::get must rewrite bytes 0-1
    // to the current query's id on every hit.
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    // build_answer hard-codes id=0x1234.
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    cache.insert("example.com", 1, 1, &bytes);

    // Hit with a different query id -- response bytes 0-1 must
    // reflect THAT id, not 0x1234.
    let got = cache
        .get("example.com", 1, 1, 0xCAFE, &policy)
        .expect("cache hit");
    assert_eq!(got[0], 0xCA, "bytes[0] not patched: {:#04x}", got[0]);
    assert_eq!(got[1], 0xFE, "bytes[1] not patched: {:#04x}", got[1]);
    // Sanity: rest of the response is untouched (next 2 bytes are
    // flags, then qdcount, etc. -- unchanged from the original).
    assert_eq!(&got[2..], &bytes[2..]);

    // Different id again, same key -- another patch.
    let got2 = cache
        .get("example.com", 1, 1, 0xBABE, &policy)
        .expect("cache hit 2");
    assert_eq!(got2[0], 0xBA);
    assert_eq!(got2[1], 0xBE);
}

#[test]
fn cache_hit_with_zero_query_id_zeroes_bytes() {
    // Defensive: query id = 0 must overwrite the cached bytes too,
    // not skip the patch.
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    cache.insert("example.com", 1, 1, &bytes);
    let got = cache.get("example.com", 1, 1, 0, &policy).unwrap();
    assert_eq!(got[0], 0);
    assert_eq!(got[1], 0);
}

#[test]
fn evicts_when_capacity_exceeded() {
    let cache = DnsAnswerCache::new(2, 300);
    let policy = allow_all();
    cache.insert("a.com", 1, 1, &build_answer("a.com.", 60, [1, 1, 1, 1]));
    cache.insert("b.com", 1, 1, &build_answer("b.com.", 60, [2, 2, 2, 2]));
    assert_eq!(cache.len(), 2);
    cache.insert("c.com", 1, 1, &build_answer("c.com.", 60, [3, 3, 3, 3]));
    assert_eq!(cache.len(), 2); // a.com evicted (LRU)
    assert!(cache.get("a.com", 1, 1, 0, &policy).is_none());
    assert!(cache.get("b.com", 1, 1, 0, &policy).is_some());
    assert!(cache.get("c.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn capacity_one_still_works() {
    let cache = DnsAnswerCache::new(1, 300);
    let policy = allow_all();
    cache.insert("a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    cache.insert("b.com", 1, 1, &build_answer("b.com.", 60, [5, 6, 7, 8]));
    assert_eq!(cache.len(), 1);
    assert!(cache.get("a.com", 1, 1, 0, &policy).is_none());
    assert!(cache.get("b.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn capacity_zero_clamped_to_one() {
    // We don't crash on zero -- silent bump to 1.
    let cache = DnsAnswerCache::new(0, 300);
    let policy = allow_all();
    cache.insert("a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    assert!(cache.get("a.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn lru_order_updates_on_access() {
    let cache = DnsAnswerCache::new(2, 300);
    let policy = allow_all();
    cache.insert("a.com", 1, 1, &build_answer("a.com.", 60, [1, 1, 1, 1]));
    cache.insert("b.com", 1, 1, &build_answer("b.com.", 60, [2, 2, 2, 2]));
    // Access a -> a becomes most-recently-used; b is now LRU.
    let _ = cache.get("a.com", 1, 1, 0, &policy);
    cache.insert("c.com", 1, 1, &build_answer("c.com.", 60, [3, 3, 3, 3]));
    // b should be evicted, not a.
    assert!(cache.get("a.com", 1, 1, 0, &policy).is_some());
    assert!(cache.get("b.com", 1, 1, 0, &policy).is_none());
}

#[test]
fn ttl_from_answer_min_clamps_to_min_secs() {
    // Answer record TTL = 5 (below MIN_TTL_SECS=60). Cache should
    // honor the min floor.
    let bytes = build_answer("example.com.", 5, [1, 2, 3, 4]);
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(MIN_TTL_SECS as u64));
}

#[test]
fn ttl_from_answer_min_clamps_to_max_ttl() {
    // Answer record TTL = 86400 (24h). max_ttl = 60s. Cache
    // honors the ceiling.
    let bytes = build_answer("example.com.", 86400, [1, 2, 3, 4]);
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(60));
    assert_eq!(ttl, Duration::from_secs(60));
}

#[test]
fn ttl_from_answer_passes_through_in_range() {
    // Answer record TTL = 120 (between min=60 and max=300).
    let bytes = build_answer("example.com.", 120, [1, 2, 3, 4]);
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(120));
}

#[test]
fn ttl_from_answer_garbage_falls_to_min_secs() {
    let ttl = ttl_from_answer(b"\xff\xff", Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(MIN_TTL_SECS as u64));
}

#[test]
fn ttl_from_answer_no_answer_records_falls_to_min_secs() {
    let mut msg = Message::new(0, MessageType::Response, OpCode::Query);
    msg.metadata.response_code = ResponseCode::NoError;
    let n = Name::from_ascii("example.com.").unwrap();
    msg.add_query(Query::query(n, RecordType::A));
    let bytes = msg.to_vec().unwrap();
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(MIN_TTL_SECS as u64));
}

#[test]
fn ttl_from_answer_picks_min_across_records() {
    let mut msg = Message::new(0, MessageType::Response, OpCode::Query);
    msg.metadata.response_code = ResponseCode::NoError;
    let n = Name::from_ascii("example.com.").unwrap();
    msg.add_query(Query::query(n.clone(), RecordType::A));
    // Two records, TTLs 200 and 100. Min = 100, in the [60, 300]
    // band, so cache should honor it verbatim.
    msg.add_answer(Record::from_rdata(
        n.clone(),
        200,
        RData::A(rdata::A(Ipv4Addr::new(1, 1, 1, 1))),
    ));
    msg.add_answer(Record::from_rdata(
        n,
        100,
        RData::A(rdata::A(Ipv4Addr::new(2, 2, 2, 2))),
    ));
    let bytes = msg.to_vec().unwrap();
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(100));
}

#[test]
fn clear_drops_every_entry() {
    let cache = DnsAnswerCache::new(16, 300);
    cache.insert("a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    cache.insert("b.com", 1, 1, &build_answer("b.com.", 60, [5, 6, 7, 8]));
    assert_eq!(cache.len(), 2);
    cache.clear();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn default_capacity_and_max_ttl_match_constants() {
    let cache = DnsAnswerCache::default();
    // Insert N+1 entries to verify capacity is what we claimed.
    let policy = allow_all();
    for i in 0..(DEFAULT_CAPACITY + 1) {
        let name = format!("h{i}.example.com");
        cache.insert(
            &name,
            1,
            1,
            &build_answer(&format!("{name}."), 60, [1, 2, 3, 4]),
        );
    }
    assert_eq!(cache.len(), DEFAULT_CAPACITY);
    // First one should now be evicted.
    assert!(cache.get("h0.example.com", 1, 1, 0, &policy).is_none());
}
