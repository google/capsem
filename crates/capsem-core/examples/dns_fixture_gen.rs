//! Generate the on-disk DNS wire-format fixtures used by
//! `crates/capsem-core/src/net/parsers/dns_parser/tests.rs`.
//!
//! Run from the repo root:
//!
//!   cargo run -p capsem-core --example dns_fixture_gen
//!
//! Writes every `.bin` file in
//! `crates/capsem-core/src/net/parsers/dns_parser/fixtures/` from a
//! deterministic seed (fixed transaction ids, fixed names, fixed
//! adversarial byte literals). Idempotent: re-running with no source
//! changes produces byte-identical fixtures.
//!
//! The fixtures are version-controlled so `cargo test` doesn't need
//! to regenerate them. Re-run this example after a hickory-proto
//! upgrade or whenever the fixture seed values change in
//! `tests.rs::regenerate_fixtures`. The same generation logic is
//! mirrored as the `regenerate_fixtures` ignored test.

use std::path::PathBuf;

use capsem_core::net::parsers::dns_parser::{build_nxdomain, build_servfail};
use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(n, qtype));
    msg.to_vec().unwrap()
}

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/net/parsers/dns_parser/fixtures");
    std::fs::create_dir_all(&dir).expect("create fixtures dir");

    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        println!("wrote {} ({} bytes)", path.display(), bytes.len());
    };

    // Standard query shapes -- each pinned to a deterministic id.
    write(
        "simple_a_query.bin",
        &build_query_bytes("anthropic.com.", RecordType::A, 0x1234),
    );
    write(
        "aaaa_query.bin",
        &build_query_bytes("anthropic.com.", RecordType::AAAA, 0x4242),
    );
    write(
        "txt_query.bin",
        &build_query_bytes("example.com.", RecordType::TXT, 0x1111),
    );
    write(
        "mx_query.bin",
        &build_query_bytes("example.com.", RecordType::MX, 0x2222),
    );
    write(
        "caa_query.bin",
        &build_query_bytes("example.com.", RecordType::CAA, 0x3333),
    );
    write(
        "https_query.bin",
        &build_query_bytes("example.com.", RecordType::HTTPS, 0x4444),
    );

    // Multi-question (RFC-legal but resolver-rare).
    let mut multi = Message::new(0x5555, MessageType::Query, OpCode::Query);
    multi.metadata.recursion_desired = true;
    multi.add_query(Query::query(
        Name::from_ascii("first.com.").unwrap(),
        RecordType::A,
    ));
    multi.add_query(Query::query(
        Name::from_ascii("second.com.").unwrap(),
        RecordType::A,
    ));
    write("multi_question_query.bin", &multi.to_vec().unwrap());

    // Synthetic responses produced by our own builders.
    let nx_req = build_query_bytes("blocked.example.com.", RecordType::A, 0xCAFE);
    write("nxdomain_response.bin", &build_nxdomain(&nx_req).unwrap());
    let sf_req = build_query_bytes("upstream-down.example.com.", RecordType::A, 0xDEAD);
    write("servfail_response.bin", &build_servfail(&sf_req).unwrap());

    // Hand-crafted adversarial fixtures (raw bytes -- hickory's
    // encoder won't emit these by design).
    write(
        "header_only.bin",
        &[
            0x12, 0x34, // id
            0x01, 0x00, // flags: standard query, RD
            0x00, 0x00, // qdcount = 0
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    );
    write(
        "lying_qdcount.bin",
        &[
            0x12, 0x34, // id
            0x01, 0x00, // flags
            0x00, 0x05, // qdcount = 5 (lie -- no question section follows)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    );

    // Truncated query: header + qdcount=1 + length byte saying 5
    // bytes of label, only 2 present.
    let mut trunc = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    trunc.push(5);
    trunc.extend_from_slice(b"ab");
    write("truncated_query.bin", &trunc);

    // Compression self-loop: name label is a 2-byte pointer
    // (0xC0 0x0C) referencing offset 12 -- itself.
    let mut loop_bytes = vec![
        0x12, 0x34, // id
        0x01, 0x00, // flags
        0x00, 0x01, // qdcount
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    loop_bytes.extend_from_slice(&[0xC0, 0x0C]);
    loop_bytes.extend_from_slice(&[0x00, 0x01]); // qtype A
    loop_bytes.extend_from_slice(&[0x00, 0x01]); // qclass IN
    write("compression_self_loop.bin", &loop_bytes);

    println!("done");
}
