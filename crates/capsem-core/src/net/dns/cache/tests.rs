use super::*;

use std::net::Ipv4Addr;

use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{rdata, Name, RData, Record, RecordType};

use crate::net::policy::{DnsRedirect, NetworkMechanics};

/// Build a synthetic A-record answer for `qname` with `ttl` seconds
/// on the answer record. Used to seed cache entries with known TTLs.
fn build_answer(qname: &str, ttl: u32, ip: [u8; 4]) -> Vec<u8> {
    let mut msg = Message::new(0x1234, MessageType::Response, OpCode::Query);
    msg.metadata.recursion_available = true;
    msg.metadata.response_code = ResponseCode::NoError;
    let n = Name::from_ascii(qname).unwrap();
    msg.add_query(Query::query(n.clone(), RecordType::A));
    msg.add_answer(Record::from_rdata(n, ttl, RData::A(rdata::A(Ipv4Addr::from(ip)))));
    msg.to_vec().unwrap()
}

fn allow_all() -> NetworkMechanics {
    NetworkMechanics::new()
}

#[test]
fn miss_on_empty_cache() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    assert!(get(&cache, "example.com", 1, 1, 0, &policy).is_none());
    assert_eq!(cache.len(), 0);
}

#[test]
fn hit_after_insert_within_ttl() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    insert(&cache, "example.com", 1, 1, &bytes);
    // Pass query_id = 0x1234 -- matches build_answer's hard-coded
    // id so the qid patch is a no-op and we can compare bit-for-bit.
    let got = get(&cache, "example.com", 1, 1, 0x1234, &policy);
    assert_eq!(got.as_deref(), Some(bytes.as_slice()));
}

#[test]
fn miss_when_qtype_differs() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    insert(&cache, "example.com", 1, 1, &bytes);
    // Same qname, different qtype (AAAA) -- must miss.
    assert!(get(&cache, "example.com", 28, 1, 0, &policy).is_none());
}

#[test]
fn miss_when_qclass_differs() {
    let cache = DnsAnswerCache::new(16, 300);
    let policy = allow_all();
    let bytes = build_answer("example.com.", 60, [1, 2, 3, 4]);
    insert(&cache, "example.com", 1, 1, &bytes);
    // CHAOS qclass on the same name+qtype -- must miss.
    assert!(get(&cache, "example.com", 1, 3, 0, &policy).is_none());
}

#[test]
fn invalidated_when_policy_now_redirects() {
    let cache = DnsAnswerCache::new(16, 300);
    let bytes = build_answer("anthropic.com.", 60, [10, 0, 0, 1]);
    insert(&cache, "anthropic.com", 1, 1, &bytes);

    let mut redirect_policy = NetworkMechanics::new();
    redirect_policy.dns_redirects.push(DnsRedirect::new(
        "anthropic.com",
        Some(1),
        vec![std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)],
        60,
    ));
    // Cache hit must not bypass an admin's later redirect rule --
    // the next lookup must miss + invalidate.
    assert!(get(&cache, "anthropic.com", 1, 1, 0, &redirect_policy).is_none());
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
    insert(&cache, "example.com", 1, 1, &bytes);

    // Hit with a different query id -- response bytes 0-1 must
    // reflect THAT id, not 0x1234.
    let got = get(&cache, "example.com", 1, 1, 0xCAFE, &policy).expect("cache hit");
    assert_eq!(got[0], 0xCA, "bytes[0] not patched: {:#04x}", got[0]);
    assert_eq!(got[1], 0xFE, "bytes[1] not patched: {:#04x}", got[1]);
    // Sanity: rest of the response is untouched (next 2 bytes are
    // flags, then qdcount, etc. -- unchanged from the original).
    assert_eq!(&got[2..], &bytes[2..]);

    // Different id again, same key -- another patch.
    let got2 = get(&cache, "example.com", 1, 1, 0xBABE, &policy).expect("cache hit 2");
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
    insert(&cache, "example.com", 1, 1, &bytes);
    let got = get(&cache, "example.com", 1, 1, 0, &policy).unwrap();
    assert_eq!(got[0], 0);
    assert_eq!(got[1], 0);
}

#[test]
fn evicts_when_capacity_exceeded() {
    let cache = DnsAnswerCache::new(2, 300);
    let policy = allow_all();
    insert(&cache, "a.com", 1, 1, &build_answer("a.com.", 60, [1, 1, 1, 1]));
    insert(&cache, "b.com", 1, 1, &build_answer("b.com.", 60, [2, 2, 2, 2]));
    assert_eq!(cache.len(), 2);
    insert(&cache, "c.com", 1, 1, &build_answer("c.com.", 60, [3, 3, 3, 3]));
    assert_eq!(cache.len(), 2); // a.com evicted (LRU)
    assert!(get(&cache, "a.com", 1, 1, 0, &policy).is_none());
    assert!(get(&cache, "b.com", 1, 1, 0, &policy).is_some());
    assert!(get(&cache, "c.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn capacity_one_still_works() {
    let cache = DnsAnswerCache::new(1, 300);
    let policy = allow_all();
    insert(&cache, "a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    insert(&cache, "b.com", 1, 1, &build_answer("b.com.", 60, [5, 6, 7, 8]));
    assert_eq!(cache.len(), 1);
    assert!(get(&cache, "a.com", 1, 1, 0, &policy).is_none());
    assert!(get(&cache, "b.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn capacity_zero_clamped_to_one() {
    // We don't crash on zero -- silent bump to 1.
    let cache = DnsAnswerCache::new(0, 300);
    let policy = allow_all();
    insert(&cache, "a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    assert!(get(&cache, "a.com", 1, 1, 0, &policy).is_some());
}

#[test]
fn lru_order_updates_on_access() {
    let cache = DnsAnswerCache::new(2, 300);
    let policy = allow_all();
    insert(&cache, "a.com", 1, 1, &build_answer("a.com.", 60, [1, 1, 1, 1]));
    insert(&cache, "b.com", 1, 1, &build_answer("b.com.", 60, [2, 2, 2, 2]));
    // Access a -> a becomes most-recently-used; b is now LRU.
    let _ = get(&cache, "a.com", 1, 1, 0, &policy);
    insert(&cache, "c.com", 1, 1, &build_answer("c.com.", 60, [3, 3, 3, 3]));
    // b should be evicted, not a.
    assert!(get(&cache, "a.com", 1, 1, 0, &policy).is_some());
    assert!(get(&cache, "b.com", 1, 1, 0, &policy).is_none());
}

#[test]
fn ttl_from_answer_honors_short_authoritative_lifetimes() {
    let bytes = build_answer("example.com.", 5, [1, 2, 3, 4]);
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::from_secs(5));
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
fn ttl_from_answer_garbage_is_not_cacheable() {
    let ttl = ttl_from_answer(b"\xff\xff", Duration::from_secs(300));
    assert_eq!(ttl, Duration::ZERO);
}

#[test]
fn ttl_from_answer_no_answer_records_requires_an_soa() {
    let mut msg = Message::new(0, MessageType::Response, OpCode::Query);
    msg.metadata.response_code = ResponseCode::NoError;
    let n = Name::from_ascii("example.com.").unwrap();
    msg.add_query(Query::query(n, RecordType::A));
    let bytes = msg.to_vec().unwrap();
    let ttl = ttl_from_answer(&bytes, Duration::from_secs(300));
    assert_eq!(ttl, Duration::ZERO);
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
    insert(&cache, "a.com", 1, 1, &build_answer("a.com.", 60, [1, 2, 3, 4]));
    insert(&cache, "b.com", 1, 1, &build_answer("b.com.", 60, [5, 6, 7, 8]));
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
    for i in 0..=DEFAULT_CAPACITY {
        let name = format!("h{i}.example.com");
        insert(
            &cache,
            &name,
            1,
            1,
            &build_answer(&format!("{name}."), 60, [1, 2, 3, 4]),
        );
    }
    assert_eq!(cache.len(), DEFAULT_CAPACITY);
    // First one should now be evicted.
    assert!(get(&cache, "h0.example.com", 1, 1, 0, &policy).is_none());
}

/// An NXDOMAIN response for `qname`, optionally carrying a SOA in the
/// authority section with `soa_minimum` seconds.
fn build_nxdomain_answer(qname: &str, soa_minimum: Option<u32>) -> Vec<u8> {
    let mut msg = Message::new(0x4444, MessageType::Response, OpCode::Query);
    msg.metadata.recursion_available = true;
    msg.metadata.response_code = ResponseCode::NXDomain;
    let n = Name::from_ascii(qname).unwrap();
    msg.add_query(Query::query(n, RecordType::A));
    if let Some(minimum) = soa_minimum {
        let zone = Name::from_ascii("example.").unwrap();
        let soa = rdata::SOA::new(
            Name::from_ascii("ns.example.").unwrap(),
            Name::from_ascii("hostmaster.example.").unwrap(),
            1,
            3600,
            600,
            86400,
            minimum,
        );
        msg.add_authority(Record::from_rdata(zone, 3600, RData::SOA(soa)));
    }
    msg.to_vec().unwrap()
}

#[test]
fn nxdomain_is_cached_for_at_most_the_negative_ceiling() {
    let cache = DnsAnswerCache::default();
    insert_negative(
        &cache,
        "dead.example.",
        1,
        1,
        &build_nxdomain_answer("dead.example.", Some(86_400)),
    );
    let hit = get(&cache, "dead.example.", 1, 1, 0x0101, &allow_all()).expect("negative answer served");
    assert_eq!(&hit[0..2], &[0x01, 0x01], "the caller's transaction id is patched in");
    assert_eq!(hit[3] & 0x0F, 3, "still NXDOMAIN");
    assert_eq!(
        negative_ttl_from_answer(&build_nxdomain_answer("dead.example.", Some(86_400))),
        Duration::from_secs(u64::from(NEGATIVE_MAX_TTL_SECS)),
        "a day-long SOA minimum is capped"
    );
}

#[test]
fn zero_or_missing_negative_ttl_does_not_create_a_cache_entry() {
    for minimum in [Some(0), None] {
        let cache = DnsAnswerCache::default();
        insert_negative(
            &cache,
            "transient.example.",
            1,
            1,
            &build_nxdomain_answer("transient.example.", minimum),
        );
        assert!(
            get(&cache, "transient.example.", 1, 1, 2, &allow_all()).is_none(),
            "no authoritative cache lifetime: {minimum:?}"
        );
    }
}

#[test]
fn positive_zero_ttl_does_not_create_a_cache_entry() {
    let cache = DnsAnswerCache::default();
    insert(
        &cache,
        "transient.example",
        1,
        1,
        &build_answer("transient.example.", 0, [192, 0, 2, 1]),
    );
    assert!(get(&cache, "transient.example", 1, 1, 1, &allow_all()).is_none());
}

#[test]
fn a_cache_hit_decrements_record_ttls_by_the_time_already_spent_cached() {
    let cache = DnsAnswerCache::default();
    insert(
        &cache,
        "aged.example",
        1,
        1,
        &build_answer("aged.example.", 60, [192, 0, 2, 1]),
    );
    let key = query_key("aged.example", 1, 1, 0).1;
    cache.inner.lock().unwrap().get_mut(&key).unwrap().expires_at = Instant::now() + Duration::from_secs(30);
    let hit = get(&cache, "aged.example", 1, 1, 1, &allow_all()).unwrap();
    let message = Message::from_vec(&hit).unwrap();
    assert!(
        message.answers[0].ttl <= 30,
        "a cache hit must not restart the record lifetime"
    );
}

#[test]
fn negative_ttl_follows_the_soa_minimum_and_requires_a_lifetime() {
    assert_eq!(
        negative_ttl_from_answer(&build_nxdomain_answer("d.example.", Some(5))),
        Duration::from_secs(5)
    );
    assert_eq!(
        negative_ttl_from_answer(&build_nxdomain_answer("d.example.", Some(0))),
        Duration::ZERO,
        "a zero minimum prohibits caching"
    );
    assert_eq!(
        negative_ttl_from_answer(&build_nxdomain_answer("d.example.", None)),
        Duration::ZERO,
        "no SOA: no authoritative cache lifetime"
    );
    assert_eq!(
        negative_ttl_from_answer(&[0xde, 0xad]),
        Duration::ZERO,
        "undecodable bytes cannot populate the cache"
    );
}

#[test]
fn negative_answers_with_aliases_honor_both_the_alias_and_soa_lifetimes() {
    for (alias_ttl, minimum) in [(5, 60), (300, 5)] {
        let mut message = Message::from_vec(&build_nxdomain_answer("alias.example.", Some(minimum))).unwrap();
        message.add_answer(Record::from_rdata(
            Name::from_ascii("alias.example.").unwrap(),
            alias_ttl,
            RData::CNAME(rdata::CNAME(Name::from_ascii("missing.example.").unwrap())),
        ));
        let expected = Duration::from_secs(u64::from(alias_ttl.min(minimum)));
        assert_eq!(negative_ttl_from_answer(&message.to_vec().unwrap()), expected);
        message.metadata.response_code = ResponseCode::NoError;
        assert_eq!(
            ttl_from_answer(&message.to_vec().unwrap(), Duration::from_secs(300)),
            expected,
            "NODATA after an alias is still bounded by its authoritative negative lifetime"
        );
    }
}

#[test]
fn a_negative_entry_yields_to_a_redirect_change_like_any_other() {
    let cache = DnsAnswerCache::default();
    insert_negative(
        &cache,
        "moved.example.",
        1,
        1,
        &build_nxdomain_answer("moved.example.", Some(60)),
    );
    let mut policy = NetworkMechanics::new();
    policy.dns_redirects.push(DnsRedirect::new(
        "moved.example.",
        None,
        vec!["10.0.0.9".parse().unwrap()],
        60,
    ));
    assert!(
        get(&cache, "moved.example.", 1, 1, 1, &policy).is_none(),
        "a redirect added after the negative answer wins"
    );
    assert!(cache.is_empty(), "the stale negative entry is evicted");
}

#[test]
fn eight_threads_insert_and_read_negative_and_positive_answers_without_poisoning() {
    let cache = std::sync::Arc::new(DnsAnswerCache::new(64, 300));
    let handles: Vec<_> = (0..8u32)
        .map(|worker| {
            let cache = std::sync::Arc::clone(&cache);
            std::thread::spawn(move || {
                for i in 0..200u32 {
                    let name = format!("n{}.example.", (worker * 200 + i) % 40);
                    if i % 2 == 0 {
                        insert(&cache, &name, 1, 1, &build_answer(&name, 300, [10, 0, 0, 1]));
                    } else {
                        insert_negative(&cache, &name, 1, 1, &build_nxdomain_answer(&name, Some(30)));
                    }
                    let _ = get(&cache, &name, 1, 1, i as u16, &allow_all());
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("no thread panicked");
    }
    assert!(cache.len() <= 64);
}

// Build the actual wire query so the cache tests exercise the same complete
// identity as the production handler, including type and class.
fn query_key(name: &str, qtype: u16, qclass: u16, id: u16) -> (DnsQuery, LookupKey) {
    let mut message = Message::new(id, MessageType::Query, OpCode::Query);
    let mut question = Query::query(Name::from_ascii(name).unwrap(), RecordType::from(qtype));
    question.set_query_class(hickory_proto::rr::DNSClass::from(qclass));
    message.add_query(question);
    let bytes = message.to_vec().unwrap();
    (
        crate::net::parsers::dns_parser::parse_query(&bytes).unwrap(),
        LookupKey::new(&bytes),
    )
}

fn get(
    cache: &DnsAnswerCache,
    name: &str,
    qtype: u16,
    qclass: u16,
    id: u16,
    policy: &NetworkMechanics,
) -> Option<Vec<u8>> {
    let (query, key) = query_key(name, qtype, qclass, id);
    cache.get(&query, &key, policy)
}

fn insert(cache: &DnsAnswerCache, name: &str, qtype: u16, qclass: u16, bytes: &[u8]) {
    cache.insert(&query_key(name, qtype, qclass, 0).1, bytes);
}

fn insert_negative(cache: &DnsAnswerCache, name: &str, qtype: u16, qclass: u16, bytes: &[u8]) {
    cache.insert_negative(&query_key(name, qtype, qclass, 0).1, bytes);
}
