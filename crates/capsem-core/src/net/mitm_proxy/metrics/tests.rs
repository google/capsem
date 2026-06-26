//! Smoke tests for the metrics declarations + emission helpers in
//! `mitm_proxy::metrics`. Real per-request emission is exercised
//! through the existing `mitm_proxy::tests` integration paths -- here
//! we just verify the names + describe_all both compile and produce
//! the documented set without panicking.

use super::*;

#[test]
fn all_names_distinct() {
    let names = [
        CONNECTIONS_TOTAL,
        REQUESTS_TOTAL,
        POLICY_DECISIONS_TOTAL,
        DNS_QUERIES_TOTAL,
        MCP_METHODS_TOTAL,
        MCP_DISCONNECTS_TOTAL,
        PARSER_EVENTS_TOTAL,
        HOOK_INVOCATIONS_TOTAL,
        TELEMETRY_DROPPED_TOTAL,
        TLS_HANDSHAKE_MS,
        UPSTREAM_DIAL_MS,
        HOOK_DURATION_MS,
        TELEMETRY_RESPONSE_END_DURATION_MS,
        TELEMETRY_STAGE_DURATION_MS,
        REQUEST_BODY_BYTES,
        RESPONSE_BODY_BYTES,
        ACTIVE_CONNECTIONS,
        UPSTREAM_POOL_SIZE,
        RUNTIME_BUSY_RATIO,
    ];
    let mut sorted = names.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        names.len(),
        sorted.len(),
        "metric names must be unique; duplicate found",
    );
}

#[test]
fn describe_all_does_not_panic() {
    // No recorder installed -> describe_* are no-ops at the facade
    // level, but still need to be valid metadata.
    describe_all();
    // Idempotent -- calling twice is fine.
    describe_all();
}
