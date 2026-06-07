#![no_main]
//! Fuzz target: `dns_parser::parse_query` must NOT panic, hang, or
//! allocate unbounded memory on any byte sequence. Returning Err on
//! malformed input is fine; what we're testing is the safety contract
//! of the wire-format decoder against adversarial wire bytes (the same
//! shape we'd see if a compromised upstream nameserver tried to
//! poison us via the upstream UDP forwarder).
//!
//! Run from `crates/capsem-core/fuzz/`:
//!
//!     cargo +nightly fuzz run parse_query -- -max_total_time=60
//!
//! Plan acceptance: survives 60s clean.
//!
//! Corpus seeds live in `corpus/parse_query/` -- start with the
//! T3.b fixtures (`crates/capsem-network-engine/src/dns_parser/
//! fixtures/*.bin`) for fast structural coverage.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // We don't care whether the result is Ok or Err -- only that the
    // call returns in bounded time without panicking, hanging, or
    // OOMing. libFuzzer treats panics + timeouts + OOMs as crashes.
    let _ = capsem_network_engine::dns_parser::parse_query(data);
});
