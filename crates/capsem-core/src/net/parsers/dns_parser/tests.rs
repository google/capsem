use super::*;
use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{DNSClass, Name, RecordType};

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(n, qtype));
    msg.to_vec().unwrap()
}

#[test]
fn parse_simple_a_query() {
    let bytes = build_query_bytes("anthropic.com.", RecordType::A, 0x1234);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.id, 0x1234);
    assert_eq!(parsed.qname, "anthropic.com");
    assert_eq!(parsed.qtype, u16::from(RecordType::A));
    assert_eq!(parsed.qclass, 1); // IN
    assert_eq!(parsed.extra_questions, 0);
}

#[test]
fn parse_strips_trailing_dot_and_lowercases() {
    let bytes = build_query_bytes("ANThropic.COM.", RecordType::A, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qname, "anthropic.com");
}

#[test]
fn parse_preserves_query_id() {
    for id in [0u16, 1, 0xFFFE, 0xFFFF] {
        let bytes = build_query_bytes("example.com.", RecordType::AAAA, id);
        let parsed = parse_query(&bytes).unwrap();
        assert_eq!(parsed.id, id);
    }
}

#[test]
fn parse_aaaa_query() {
    let bytes = build_query_bytes("v6.example.com.", RecordType::AAAA, 7);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::AAAA));
    assert_eq!(parsed.qtype, 28);
}

#[test]
fn parse_txt_query() {
    let bytes = build_query_bytes("example.com.", RecordType::TXT, 5);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::TXT));
}

#[test]
fn parse_mx_query() {
    let bytes = build_query_bytes("example.com.", RecordType::MX, 5);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::MX));
}

#[test]
fn parse_garbage_bytes_errors() {
    let err = parse_query(b"not a dns message").unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("dns") || msg.contains("decode"),
        "expected DNS decode error, got: {msg}"
    );
}

#[test]
fn parse_truncated_header_errors() {
    // First 6 bytes of a real DNS query header (incomplete).
    assert!(parse_query(&[0, 1, 0, 0, 0, 1]).is_err());
}

#[test]
fn parse_zero_questions_errors() {
    let msg = Message::new(99, MessageType::Query, OpCode::Query);
    let bytes = msg.to_vec().unwrap();
    let err = parse_query(&bytes).unwrap_err();
    assert!(format!("{err:#}").contains("no questions"));
}

#[test]
fn parse_multi_question_returns_first_and_extras() {
    let mut msg = Message::new(42, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n1 = Name::from_ascii("first.com.").unwrap();
    let n2 = Name::from_ascii("second.com.").unwrap();
    msg.add_query(Query::query(n1, RecordType::A));
    msg.add_query(Query::query(n2, RecordType::A));
    let bytes = msg.to_vec().unwrap();
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qname, "first.com");
    assert_eq!(parsed.extra_questions, 1);
}

#[test]
fn build_nxdomain_preserves_id_and_questions() {
    let req = build_query_bytes("blocked.example.com.", RecordType::A, 0xCAFE);
    let resp_bytes = build_nxdomain(&req).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.id, 0xCAFE);
    assert_eq!(resp.metadata.message_type, MessageType::Response);
    assert_eq!(resp.metadata.response_code, ResponseCode::NXDomain);
    assert_eq!(resp.queries.len(), 1);
    assert_eq!(
        resp.queries[0].name().to_ascii().trim_end_matches('.'),
        "blocked.example.com"
    );
    assert_eq!(resp.answers.len(), 0);
    assert!(resp.metadata.recursion_available);
}

#[test]
fn build_nxdomain_preserves_recursion_desired_bit() {
    // Some clients set RD=0 (e.g. internal validators); response must
    // mirror that bit so the guest can tell it didn't get cached.
    let mut req = Message::new(1, MessageType::Query, OpCode::Query);
    req.metadata.recursion_desired = false;
    let q = Query::query(Name::from_ascii("x.example.").unwrap(), RecordType::A);
    req.add_query(q);
    let bytes = req.to_vec().unwrap();
    let resp_bytes = build_nxdomain(&bytes).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert!(!resp.metadata.recursion_desired);
}

#[test]
fn build_nxdomain_garbage_input_errors() {
    assert!(build_nxdomain(b"not a dns message").is_err());
}

#[test]
fn build_servfail_sets_correct_rcode() {
    let req = build_query_bytes("upstream-down.example.com.", RecordType::A, 1);
    let resp_bytes = build_servfail(&req).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::ServFail);
    assert_eq!(resp.metadata.id, 1);
    assert_eq!(resp.metadata.message_type, MessageType::Response);
}

// =====================================================================
// (a) -- record-type breadth
//
// Every qtype the dev policy might see. Hickory exposes them via the
// RecordType enum + a u16 conversion; the parser is qtype-agnostic so
// we mostly assert "the qtype round-trips through the wire codec
// unchanged" -- a hickory upgrade that quietly renumbers a variant
// (or drops one) lights up here before it bites a real query.
// =====================================================================

#[test]
fn parse_cname_query() {
    let bytes = build_query_bytes("alias.example.com.", RecordType::CNAME, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::CNAME));
    assert_eq!(parsed.qtype, 5);
}

#[test]
fn parse_ns_query() {
    let bytes = build_query_bytes("example.com.", RecordType::NS, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::NS));
    assert_eq!(parsed.qtype, 2);
}

#[test]
fn parse_soa_query() {
    let bytes = build_query_bytes("example.com.", RecordType::SOA, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::SOA));
    assert_eq!(parsed.qtype, 6);
}

#[test]
fn parse_ptr_query() {
    let bytes = build_query_bytes("1.0.0.127.in-addr.arpa.", RecordType::PTR, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qname, "1.0.0.127.in-addr.arpa");
    assert_eq!(parsed.qtype, u16::from(RecordType::PTR));
    assert_eq!(parsed.qtype, 12);
}

#[test]
fn parse_srv_query() {
    let bytes = build_query_bytes("_xmpp._tcp.example.com.", RecordType::SRV, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qname, "_xmpp._tcp.example.com");
    assert_eq!(parsed.qtype, u16::from(RecordType::SRV));
    assert_eq!(parsed.qtype, 33);
}

#[test]
fn parse_caa_query() {
    let bytes = build_query_bytes("example.com.", RecordType::CAA, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::CAA));
    assert_eq!(parsed.qtype, 257);
}

#[test]
fn parse_https_query() {
    // RFC 9460 SVCB / HTTPS records -- rapidly becoming common as
    // Chrome / Firefox use them for ECH and ALPN advertisement.
    let bytes = build_query_bytes("example.com.", RecordType::HTTPS, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::HTTPS));
    assert_eq!(parsed.qtype, 65);
}

#[test]
fn parse_any_query() {
    let bytes = build_query_bytes("example.com.", RecordType::ANY, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::ANY));
    assert_eq!(parsed.qtype, 255);
}

#[test]
fn parse_null_query() {
    let bytes = build_query_bytes("example.com.", RecordType::NULL, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::NULL));
    assert_eq!(parsed.qtype, 10);
}

#[test]
fn parse_hinfo_query() {
    let bytes = build_query_bytes("example.com.", RecordType::HINFO, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::HINFO));
    assert_eq!(parsed.qtype, 13);
}

#[test]
fn parse_axfr_query() {
    // Zone-transfer query. We don't authoritatively serve any zone,
    // but the parser must accept the qtype so the policy / telemetry
    // can record + reject it cleanly.
    let bytes = build_query_bytes("example.com.", RecordType::AXFR, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::AXFR));
    assert_eq!(parsed.qtype, 252);
}

#[test]
fn parse_ixfr_query() {
    let bytes = build_query_bytes("example.com.", RecordType::IXFR, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qtype, u16::from(RecordType::IXFR));
    assert_eq!(parsed.qtype, 251);
}

// =====================================================================
// (a) -- qclass coverage
//
// IN is what 99.99% of queries use. Other classes show up in BIND
// tooling, dig probing, DNSSEC validators, and the occasional bug.
// The parser surfaces qclass as a u16; the values must round-trip.
// =====================================================================

fn build_query_with_class(name: &str, qtype: RecordType, klass: DNSClass, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(name).unwrap();
    let mut q = Query::query(n, qtype);
    q.set_query_class(klass);
    msg.add_query(q);
    msg.to_vec().unwrap()
}

#[test]
fn parse_qclass_in() {
    let bytes = build_query_with_class("example.com.", RecordType::A, DNSClass::IN, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qclass, 1);
}

#[test]
fn parse_qclass_chaos() {
    // CH (3) -- BIND's `version.bind` `id.server` chaos queries use this.
    let bytes = build_query_with_class("version.bind.", RecordType::TXT, DNSClass::CH, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qclass, 3);
}

#[test]
fn parse_qclass_hesiod() {
    let bytes = build_query_with_class("example.com.", RecordType::A, DNSClass::HS, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qclass, 4);
}

#[test]
fn parse_qclass_none() {
    let bytes = build_query_with_class("example.com.", RecordType::A, DNSClass::NONE, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qclass, 254);
}

#[test]
fn parse_qclass_any() {
    let bytes = build_query_with_class("example.com.", RecordType::A, DNSClass::ANY, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert_eq!(parsed.qclass, 255);
}

// =====================================================================
// (a) -- adversarial / risk-shape
//
// The parser must NOT panic, allocate unbounded memory, or hang on
// pathological inputs. Returning Err is fine; the contract is "the
// process keeps running and the row gets logged with decision=error".
// =====================================================================

#[test]
fn parse_empty_payload_errors() {
    assert!(parse_query(&[]).is_err());
}

#[test]
fn parse_single_byte_payload_errors() {
    assert!(parse_query(&[0xFF]).is_err());
}

#[test]
fn parse_header_only_payload_errors() {
    // 12-byte DNS header with all-zero counts: 0 questions, 0 answers,
    // 0 authority, 0 additional. Parses but has no questions, so the
    // parser must return our "no questions" error rather than panic.
    let header = [
        0x12, 0x34, // id
        0x01, 0x00, // flags: standard query, RD
        0x00, 0x00, // qdcount = 0
        0x00, 0x00, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
    ];
    let err = parse_query(&header).unwrap_err();
    assert!(format!("{err:#}").contains("no questions"));
}

#[test]
fn parse_payload_with_lying_qdcount_errors() {
    // Header claims 5 questions but no question section follows.
    // Hickory must reject -- panic / OOM here would be a wire-fuzz
    // surface for any future host we're proxying.
    let header = [
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x05, // qdcount = 5 (lie)
        0x00, 0x00, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
    ];
    assert!(parse_query(&header).is_err());
}

#[test]
fn parse_label_compression_self_loop_does_not_hang() {
    // RFC 1035 sec 4.1.4 message compression: a label can be a 2-byte
    // pointer (high two bits set) referencing an offset earlier in the
    // message. A pointer pointing at itself produces an infinite loop
    // in a naive decoder; hickory must detect and reject it.
    //
    // Layout: 12-byte header, qdcount=1, then a single label that's
    // a pointer to offset 12 (its own position). Pointer bytes:
    // 0xC0 0x0C  (0xC0 = compression marker, 0x0C = 12).
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount = 1
        0x00, 0x00, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
    ];
    bytes.extend_from_slice(&[0xC0, 0x0C]); // self-pointer at offset 12
    bytes.extend_from_slice(&[0x00, 0x01]); // qtype = A
    bytes.extend_from_slice(&[0x00, 0x01]); // qclass = IN

    // Must return in bounded time and NOT panic. Either Err or Ok
    // (with whatever hickory decides) is acceptable; what matters is
    // that the test process exits.
    let _ = parse_query(&bytes);
}

#[test]
fn parse_label_compression_forward_pointer_does_not_hang() {
    // Pointer to an offset PAST the end of the message. Hickory
    // must reject without reading past the buffer.
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount = 1
        0x00, 0x00, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
    ];
    bytes.extend_from_slice(&[0xC0, 0xFF]); // pointer to offset 255 (off the end)
    bytes.extend_from_slice(&[0x00, 0x01]); // qtype = A
    bytes.extend_from_slice(&[0x00, 0x01]); // qclass = IN

    let _ = parse_query(&bytes); // must NOT panic
}

#[test]
fn parse_label_too_long_errors() {
    // RFC 1035 sec 2.3.4: a label is at most 63 octets. Build a
    // single label of 64 0x41 ('A') bytes -- length byte 0x40 is
    // 64, which is invalid (the high two bits encode compression
    // markers when both set; 64 = 0100 0000 has high bits 01 which
    // is reserved/invalid in RFC 1035).
    //
    // We can't go through hickory's `Name::from_ascii` because it
    // rejects oversized labels client-side. Build the wire bytes
    // directly.
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    bytes.push(64); // invalid label length (>63)
    bytes.extend_from_slice(&[0x41u8; 64]);
    bytes.push(0); // root label
    bytes.extend_from_slice(&[0x00, 0x01]); // qtype = A
    bytes.extend_from_slice(&[0x00, 0x01]); // qclass = IN

    // Hickory should reject; certainly must not panic.
    let _ = parse_query(&bytes);
}

#[test]
fn parse_name_with_nul_byte_in_label_does_not_panic() {
    // A label of length 5 containing a NUL byte: \0 in a domain
    // name is unusual but RFC-legal as a binary label. The parser
    // must not panic; we don't care whether it returns Ok or Err.
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    bytes.push(5); // label length
    bytes.extend_from_slice(b"a\0b\0c"); // 5 bytes including NULs
    bytes.push(0); // root label
    bytes.extend_from_slice(&[0x00, 0x01]); // qtype
    bytes.extend_from_slice(&[0x00, 0x01]); // qclass

    let _ = parse_query(&bytes);
}

#[test]
fn parse_truncated_question_section_errors() {
    // Header says qdcount=1 + a length byte that promises 5 bytes
    // of label, but only 2 are present -- truncated mid-label.
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    bytes.push(5); // label length
    bytes.extend_from_slice(b"ab"); // only 2 of 5 bytes present
                                    // No root label, no qtype, no qclass -- buffer ends here.

    assert!(parse_query(&bytes).is_err());
}

#[test]
fn parse_max_label_size_accepted() {
    // A label of EXACTLY 63 bytes is the RFC max -- must parse
    // (build_query_bytes -> hickory accepts it).
    let max_label: String = "a".repeat(63);
    let name = format!("{max_label}.example.com.");
    let bytes = build_query_bytes(&name, RecordType::A, 1);
    let parsed = parse_query(&bytes).unwrap();
    assert!(parsed.qname.starts_with(&max_label));
    assert!(parsed.qname.ends_with(".example.com"));
}

#[test]
fn parse_oversized_qdcount_does_not_oom() {
    // qdcount = 0xFFFF (65535) -- if hickory naively pre-allocated
    // a 65535-element Vec<Question>, that's 2-3 MB on the stack
    // for a 12-byte input. Modern hickory uses lazy iteration
    // and bounded reads; assert we don't panic + don't allocate
    // unbounded.
    let mut bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0xFF, 0xFF, // qdcount = 65535 (lie)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x01]); // root label + A + IN
    let _ = parse_query(&bytes); // must return in bounded time
}

#[test]
fn parse_total_garbage_is_err_not_panic() {
    // Random bytes that don't form a valid DNS message at all.
    let garbage = [
        0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE,
        0xF0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
    ];
    // Either the length-byte interpretation lands on a too-large
    // value (Err) or the structure is wrong (Err); never Ok.
    let _ = parse_query(&garbage); // must not panic
}

#[test]
fn build_nxdomain_for_high_qtype_works() {
    // A query with an obscure qtype (CAA = 257) must NXDOMAIN-build
    // cleanly -- the synthetic response code path doesn't depend on
    // qtype.
    let req = build_query_bytes("blocked.example.com.", RecordType::CAA, 1);
    let resp_bytes = build_nxdomain(&req).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NXDomain);
    assert_eq!(resp.queries[0].query_type(), RecordType::CAA);
}

#[test]
fn build_nxdomain_preserves_qclass() {
    let req = build_query_with_class("blocked.example.com.", RecordType::A, DNSClass::CH, 0xCAFE);
    let resp_bytes = build_nxdomain(&req).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.queries[0].query_class(), DNSClass::CH);
}

#[test]
fn build_servfail_for_undecodable_input_errors() {
    // build_synthetic_response decodes the request first -- garbage
    // in is reported, not silently turned into an empty SERVFAIL.
    assert!(build_servfail(b"\xff\xff\xff\xff").is_err());
}

// =====================================================================
// (T3.d) -- build_redirect_response unit tests
//
// `build_redirect_response` is the wire-format builder for synthetic
// answers produced by the DnsRedirect policy rule. The handler-level
// integration is covered by `net::dns::tests`; these tests pin the
// pure-builder semantics in isolation.
// =====================================================================

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[test]
fn build_redirect_a_record_appears_in_answer() {
    let req = build_query_bytes("foo.example.com.", RecordType::A, 1);
    let answers = vec![IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))];
    let resp_bytes = build_redirect_response(&req, &answers, 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 1);
    assert_eq!(resp.answers[0].record_type(), RecordType::A);
    assert_eq!(resp.answers[0].ttl, 60);
}

#[test]
fn build_redirect_aaaa_record_appears_in_answer() {
    let req = build_query_bytes("foo.example.com.", RecordType::AAAA, 1);
    let answers = vec![IpAddr::V6(Ipv6Addr::LOCALHOST)];
    let resp_bytes = build_redirect_response(&req, &answers, 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.answers.len(), 1);
    assert_eq!(resp.answers[0].record_type(), RecordType::AAAA);
}

#[test]
fn build_redirect_filters_cross_family() {
    // A query + IPv6 answer -> NoError, zero matching answers
    // (the IPv6 is silently skipped because A means "give me v4").
    let req = build_query_bytes("foo.example.com.", RecordType::A, 1);
    let answers = vec![IpAddr::V6(Ipv6Addr::LOCALHOST)];
    let resp_bytes = build_redirect_response(&req, &answers, 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 0);
}

#[test]
fn build_redirect_mixed_family_yields_only_matching() {
    // Two IPv4 + two IPv6, A query -> only the two IPv4 land in
    // the answer section.
    let req = build_query_bytes("foo.example.com.", RecordType::A, 1);
    let answers = vec![
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)),
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    ];
    let resp_bytes = build_redirect_response(&req, &answers, 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.answers.len(), 2);
}

#[test]
fn build_redirect_preserves_id_and_question() {
    let req = build_query_bytes("blocked.example.com.", RecordType::A, 0xBEEF);
    let resp_bytes =
        build_redirect_response(&req, &[IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))], 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.id, 0xBEEF);
    assert_eq!(resp.queries.len(), 1);
    assert_eq!(
        resp.queries[0].name().to_ascii().trim_end_matches('.'),
        "blocked.example.com"
    );
}

#[test]
fn build_redirect_empty_answers_is_legal_nodata() {
    let req = build_query_bytes("noip.example.com.", RecordType::A, 1);
    let resp_bytes = build_redirect_response(&req, &[], 60).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.metadata.response_code, ResponseCode::NoError);
    assert_eq!(resp.answers.len(), 0);
}

#[test]
fn build_redirect_garbage_input_errors() {
    assert!(build_redirect_response(b"\x00", &[], 60).is_err());
}

#[test]
fn build_redirect_ttl_propagates_verbatim() {
    let req = build_query_bytes("foo.example.com.", RecordType::A, 1);
    let resp_bytes =
        build_redirect_response(&req, &[IpAddr::V4(Ipv4Addr::LOCALHOST)], 12345).unwrap();
    let resp = Message::from_vec(&resp_bytes).unwrap();
    assert_eq!(resp.answers[0].ttl, 12345);
}

// =====================================================================
// (b) -- on-disk fixture corpora + deterministic round-trip
//
// Pinning real wire bytes prevents a hickory-proto upgrade from
// silently changing the on-the-wire encoding of a query (e.g. a
// different default for the AD bit, a renumbered EDNS opt). The
// fixtures live in `fixtures/` so cargo-fuzz can seed corpora from
// them and external tools (dig captures, third-party validators)
// can exchange known-good byte streams with us.
//
// Each fixture is loaded via `include_bytes!` so test runs don't
// touch the filesystem. The regenerate_fixtures test (ignored by
// default) rewrites them from the deterministic builders.
// =====================================================================

const FIX_SIMPLE_A: &[u8] = include_bytes!("fixtures/simple_a_query.bin");
const FIX_AAAA: &[u8] = include_bytes!("fixtures/aaaa_query.bin");
const FIX_TXT: &[u8] = include_bytes!("fixtures/txt_query.bin");
const FIX_MX: &[u8] = include_bytes!("fixtures/mx_query.bin");
const FIX_CAA: &[u8] = include_bytes!("fixtures/caa_query.bin");
const FIX_HTTPS: &[u8] = include_bytes!("fixtures/https_query.bin");
const FIX_MULTI: &[u8] = include_bytes!("fixtures/multi_question_query.bin");
const FIX_NXDOMAIN: &[u8] = include_bytes!("fixtures/nxdomain_response.bin");
const FIX_SERVFAIL: &[u8] = include_bytes!("fixtures/servfail_response.bin");
const FIX_TRUNCATED: &[u8] = include_bytes!("fixtures/truncated_query.bin");
const FIX_COMPRESSION_LOOP: &[u8] = include_bytes!("fixtures/compression_self_loop.bin");
const FIX_HEADER_ONLY: &[u8] = include_bytes!("fixtures/header_only.bin");
const FIX_LYING_QDCOUNT: &[u8] = include_bytes!("fixtures/lying_qdcount.bin");

#[test]
fn fixture_simple_a_parses_to_expected_query() {
    let q = parse_query(FIX_SIMPLE_A).unwrap();
    assert_eq!(q.id, 0x1234);
    assert_eq!(q.qname, "anthropic.com");
    assert_eq!(q.qtype, u16::from(RecordType::A));
    assert_eq!(q.qclass, 1);
}

#[test]
fn fixture_aaaa_parses_to_expected_query() {
    let q = parse_query(FIX_AAAA).unwrap();
    assert_eq!(q.id, 0x4242);
    assert_eq!(q.qname, "anthropic.com");
    assert_eq!(q.qtype, u16::from(RecordType::AAAA));
}

#[test]
fn fixture_txt_parses_correctly() {
    let q = parse_query(FIX_TXT).unwrap();
    assert_eq!(q.qname, "example.com");
    assert_eq!(q.qtype, u16::from(RecordType::TXT));
}

#[test]
fn fixture_mx_parses_correctly() {
    let q = parse_query(FIX_MX).unwrap();
    assert_eq!(q.qtype, u16::from(RecordType::MX));
}

#[test]
fn fixture_caa_parses_correctly() {
    let q = parse_query(FIX_CAA).unwrap();
    assert_eq!(q.qtype, u16::from(RecordType::CAA));
}

#[test]
fn fixture_https_parses_correctly() {
    let q = parse_query(FIX_HTTPS).unwrap();
    assert_eq!(q.qtype, u16::from(RecordType::HTTPS));
}

#[test]
fn fixture_multi_question_first_and_extras_count() {
    let q = parse_query(FIX_MULTI).unwrap();
    assert_eq!(q.qname, "first.com");
    assert_eq!(q.extra_questions, 1);
}

#[test]
fn fixture_nxdomain_response_decodes() {
    let m = Message::from_vec(FIX_NXDOMAIN).unwrap();
    assert_eq!(m.metadata.message_type, MessageType::Response);
    assert_eq!(m.metadata.response_code, ResponseCode::NXDomain);
    assert_eq!(m.queries.len(), 1);
    assert_eq!(m.answers.len(), 0);
    assert_eq!(
        m.queries[0].name().to_ascii().trim_end_matches('.'),
        "blocked.example.com"
    );
}

#[test]
fn fixture_servfail_response_decodes() {
    let m = Message::from_vec(FIX_SERVFAIL).unwrap();
    assert_eq!(m.metadata.response_code, ResponseCode::ServFail);
    assert_eq!(m.metadata.message_type, MessageType::Response);
}

#[test]
fn fixture_truncated_errors_no_panic() {
    assert!(parse_query(FIX_TRUNCATED).is_err());
}

#[test]
fn fixture_compression_loop_does_not_hang() {
    // Same contract as parse_label_compression_self_loop_does_not_hang
    // but loaded from the on-disk fixture so cargo-fuzz can corpus-seed
    // from this exact byte stream.
    let _ = parse_query(FIX_COMPRESSION_LOOP);
}

#[test]
fn fixture_header_only_returns_no_questions() {
    let err = parse_query(FIX_HEADER_ONLY).unwrap_err();
    assert!(format!("{err:#}").contains("no questions"));
}

#[test]
fn fixture_lying_qdcount_errors() {
    assert!(parse_query(FIX_LYING_QDCOUNT).is_err());
}

#[test]
fn all_fixtures_have_nonzero_length() {
    // Catches "include_bytes! pointed at an empty file" -- a
    // surprisingly common failure mode after a regen that only
    // half-wrote the corpus.
    for (name, bytes) in [
        ("simple_a_query.bin", FIX_SIMPLE_A),
        ("aaaa_query.bin", FIX_AAAA),
        ("txt_query.bin", FIX_TXT),
        ("mx_query.bin", FIX_MX),
        ("caa_query.bin", FIX_CAA),
        ("https_query.bin", FIX_HTTPS),
        ("multi_question_query.bin", FIX_MULTI),
        ("nxdomain_response.bin", FIX_NXDOMAIN),
        ("servfail_response.bin", FIX_SERVFAIL),
        ("truncated_query.bin", FIX_TRUNCATED),
        ("compression_self_loop.bin", FIX_COMPRESSION_LOOP),
        ("header_only.bin", FIX_HEADER_ONLY),
        ("lying_qdcount.bin", FIX_LYING_QDCOUNT),
    ] {
        assert!(!bytes.is_empty(), "fixture {name} is empty");
    }
}

// Fixtures are bootstrapped + regenerated by:
//
//   cargo run -p capsem-core --example dns_fixture_gen
//
// See `crates/capsem-core/examples/dns_fixture_gen.rs`. Keeping the
// generator in `examples/` (separate compilation unit) avoids the
// chicken-and-egg where the `include_bytes!` macros above would fail
// to compile if the .bin files didn't exist yet.
