#![no_main]
//! Fuzz target: `dns_parser::build_nxdomain` must NOT panic on any
//! byte sequence. The function decodes the input as a query, copies
//! the questions + transaction id into a synthetic NXDOMAIN response,
//! and re-encodes; both decode and re-encode must be safe under
//! adversarial input because the policy-block path runs this on
//! whatever the guest agent sent over vsock.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = capsem_core::net::parsers::dns_parser::build_nxdomain(data);
});
