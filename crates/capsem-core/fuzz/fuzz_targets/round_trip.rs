#![no_main]
//! Fuzz target: round-trip invariant -- if `parse_query` succeeds,
//! `build_nxdomain` on the same bytes must also succeed and the
//! resulting response must decode back to a Message whose first
//! question's qname / qtype / qclass match the parsed input.
//!
//! Catches cases where parse_query is permissive but
//! build_synthetic_response chokes on the same bytes -- a divergence
//! that would let a malformed query escape NXDOMAIN gating in
//! production.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    use capsem_core::net::parsers::dns_parser::{build_nxdomain, parse_query};

    if let Ok(parsed) = parse_query(data) {
        // build_nxdomain decodes the same input and re-encodes it as
        // a response. Must succeed when parse_query succeeded -- any
        // divergence here is a real bug, so libFuzzer should treat it
        // as a crash (assert!).
        let resp_bytes = match build_nxdomain(data) {
            Ok(b) => b,
            Err(e) => panic!(
                "parse_query succeeded but build_nxdomain failed: parsed={parsed:?} err={e:#}"
            ),
        };

        // The response must re-parse back into the same query shape
        // (id, qname, qtype, qclass).
        let reparsed = parse_query(&resp_bytes).unwrap_or_else(|e| {
            panic!("synthesized NXDOMAIN failed to re-parse: {e:#}")
        });
        assert_eq!(parsed.id, reparsed.id, "id drift across NXDOMAIN round-trip");
        assert_eq!(
            parsed.qname, reparsed.qname,
            "qname drift across NXDOMAIN round-trip"
        );
        assert_eq!(
            parsed.qtype, reparsed.qtype,
            "qtype drift across NXDOMAIN round-trip"
        );
        assert_eq!(
            parsed.qclass, reparsed.qclass,
            "qclass drift across NXDOMAIN round-trip"
        );
    }
});
