#![no_main]
//! Fuzz target: `dns_parser::build_servfail` -- same safety contract
//! as build_nxdomain. Different code path (different ResponseCode)
//! but the same decode/re-encode shape, worth a separate target so
//! libFuzzer can converge on each independently.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = capsem_core::net::parsers::dns_parser::build_servfail(data);
});
