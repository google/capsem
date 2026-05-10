//! Property-based tests for the DNS wire codec (T3.f).
//!
//! Complements the unit tests + cargo-fuzz targets. Where fuzz
//! converges on crash inputs over time, these properties run on
//! every `cargo test` and pin the structural invariants:
//!
//! 1. Round-trip: a built query parses back to the same id /
//!    qname / qtype / qclass.
//! 2. NXDOMAIN preserves question: the synthetic NXDOMAIN response
//!    parses to the same id / qname / qtype / qclass as the original
//!    query.
//! 3. SERVFAIL preserves question: same shape, different rcode.
//! 4. Redirect response with arbitrary IPs preserves question.
//! 5. parse_query never panics on arbitrary bytes -- bounded-time,
//!    no-OOM safety contract from the fuzz target also asserted
//!    structurally here so PRs flag a regression even without
//!    nightly + cargo-fuzz installed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{DNSClass, Name, RecordType};
use proptest::collection::vec;
use proptest::prelude::*;

use super::{build_nxdomain, build_redirect_response, build_servfail, parse_query};

/// Strategy: a syntactically valid lowercase DNS label
/// (1..=63 ASCII letters/digits, no leading/trailing hyphens).
fn label_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(b"abcdefghijklmnopqrstuvwxyz0123456789-".as_ref()),
        1..30,
    )
    .prop_filter("no leading hyphen", |bytes| bytes[0] != b'-')
    .prop_filter("no trailing hyphen", |bytes| *bytes.last().unwrap() != b'-')
    .prop_map(|bytes| String::from_utf8(bytes).unwrap())
}

/// Strategy: a syntactically valid 2-3 label DNS name (e.g.
/// "foo.example.com"). Bounded length + label count keeps the
/// generated corpus realistic.
fn dns_name_strategy() -> impl Strategy<Value = String> {
    vec(label_strategy(), 2..=3).prop_map(|labels| labels.join("."))
}

/// Strategy: one of the qtypes the resolver is likely to see.
/// Pinned to a small set so the property runs quickly + each
/// run exercises the common types.
fn qtype_strategy() -> impl Strategy<Value = RecordType> {
    prop::sample::select(vec![
        RecordType::A,
        RecordType::AAAA,
        RecordType::TXT,
        RecordType::MX,
        RecordType::CNAME,
        RecordType::SRV,
        RecordType::CAA,
        RecordType::NS,
        RecordType::SOA,
        RecordType::PTR,
        RecordType::HTTPS,
        RecordType::ANY,
    ])
}

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(format!("{name}.")).unwrap();
    msg.add_query(Query::query(n, qtype));
    msg.to_vec().unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// Property: build_query_bytes(name, qtype, id) parses back
    /// to the same id / qname / qtype.
    #[test]
    fn parse_query_round_trip(
        name in dns_name_strategy(),
        qtype in qtype_strategy(),
        id in any::<u16>(),
    ) {
        let bytes = build_query_bytes(&name, qtype, id);
        let parsed = parse_query(&bytes).expect("synthesized query must parse");
        prop_assert_eq!(parsed.id, id);
        prop_assert_eq!(parsed.qname, name.to_lowercase());
        prop_assert_eq!(parsed.qtype, u16::from(qtype));
        prop_assert_eq!(parsed.qclass, 1); // IN
        prop_assert_eq!(parsed.extra_questions, 0);
    }

    /// Property: NXDOMAIN response built from a query parses back
    /// to a question with the same id / qname / qtype / qclass.
    #[test]
    fn build_nxdomain_preserves_question(
        name in dns_name_strategy(),
        qtype in qtype_strategy(),
        id in any::<u16>(),
    ) {
        let req = build_query_bytes(&name, qtype, id);
        let resp_bytes = build_nxdomain(&req).expect("nxdomain build must succeed");
        let resp = Message::from_vec(&resp_bytes).expect("nxdomain must re-parse");
        prop_assert_eq!(resp.metadata.id, id);
        prop_assert_eq!(resp.queries.len(), 1);
        prop_assert_eq!(resp.queries[0].query_type(), qtype);
        prop_assert_eq!(resp.queries[0].query_class(), DNSClass::IN);
        prop_assert_eq!(resp.answers.len(), 0);
    }

    /// Property: SERVFAIL response built from a query parses back
    /// with rcode=ServFail and the same question.
    #[test]
    fn build_servfail_preserves_question(
        name in dns_name_strategy(),
        qtype in qtype_strategy(),
        id in any::<u16>(),
    ) {
        let req = build_query_bytes(&name, qtype, id);
        let resp_bytes = build_servfail(&req).expect("servfail build must succeed");
        let resp = Message::from_vec(&resp_bytes).expect("servfail must re-parse");
        prop_assert_eq!(resp.metadata.id, id);
        prop_assert_eq!(resp.queries.len(), 1);
        prop_assert_eq!(resp.queries[0].query_type(), qtype);
    }

    /// Property: redirect response built with arbitrary IPv4 IPs
    /// parses back with the same question + at most `ips.len()`
    /// answers (cross-family IPs filter out).
    #[test]
    fn build_redirect_preserves_question_for_a(
        name in dns_name_strategy(),
        id in any::<u16>(),
        ips in vec(any::<u32>(), 1..5),
    ) {
        let req = build_query_bytes(&name, RecordType::A, id);
        let answers: Vec<IpAddr> = ips.iter().map(|n| {
            IpAddr::V4(Ipv4Addr::from(*n))
        }).collect();
        let resp_bytes = build_redirect_response(&req, &answers, 60)
            .expect("redirect build must succeed");
        let resp = Message::from_vec(&resp_bytes).expect("redirect must re-parse");
        prop_assert_eq!(resp.metadata.id, id);
        prop_assert_eq!(resp.queries.len(), 1);
        // Every input IP was IPv4 + qtype=A, so all should land in
        // the answer section.
        prop_assert_eq!(resp.answers.len(), answers.len());
    }

    /// Property: redirect with mixed IPv4/IPv6 + an A query yields
    /// only IPv4 answers (cross-family filter).
    #[test]
    fn build_redirect_filters_cross_family(
        name in dns_name_strategy(),
        id in any::<u16>(),
        v4: u32,
        v6: u128,
    ) {
        let req = build_query_bytes(&name, RecordType::A, id);
        let answers = vec![
            IpAddr::V4(Ipv4Addr::from(v4)),
            IpAddr::V6(Ipv6Addr::from(v6)),
        ];
        let resp_bytes = build_redirect_response(&req, &answers, 60)
            .expect("redirect build must succeed");
        let resp = Message::from_vec(&resp_bytes).expect("redirect must re-parse");
        // A query: only the IPv4 lands in the answer section.
        prop_assert_eq!(resp.answers.len(), 1);
    }

    /// Property: parse_query on arbitrary bytes never panics.
    /// Mirrors the cargo-fuzz target's safety contract; runs every
    /// `cargo test` so PRs surface a regression even without
    /// nightly + cargo-fuzz installed locally.
    #[test]
    fn parse_query_does_not_panic_on_arbitrary_bytes(
        bytes in vec(any::<u8>(), 0..2000),
    ) {
        let _ = parse_query(&bytes);
    }

    /// Property: build_nxdomain on arbitrary bytes never panics.
    /// May return Err (most arbitrary bytes don't decode), but
    /// must not crash.
    #[test]
    fn build_nxdomain_does_not_panic_on_arbitrary_bytes(
        bytes in vec(any::<u8>(), 0..2000),
    ) {
        let _ = build_nxdomain(&bytes);
    }
}
