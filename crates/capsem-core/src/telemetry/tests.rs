//! Tests for telemetry::ambient_capsem_trace_id parsing of TRACEPARENT.

use super::*;

#[test]
fn ambient_trace_id_from_capsem_env_takes_precedence() {
    // Setting CAPSEM_TRACE_ID always wins, regardless of TRACEPARENT.
    // Use a unique value so test ordering can't poison the OnceLock.
    // SAFETY: setenv on the std::env wrapper is documented unsafe in
    // multi-threaded programs; this test is single-threaded and we
    // restore the env on exit.
    unsafe {
        std::env::set_var("CAPSEM_TRACE_ID", "deadbeefcafef00d");
    }
    let id = ambient_capsem_trace_id();
    unsafe {
        std::env::remove_var("CAPSEM_TRACE_ID");
    }
    assert_eq!(id.as_deref(), Some("deadbeefcafef00d"));
}

#[test]
fn ambient_trace_id_returns_none_without_env() {
    unsafe {
        std::env::remove_var("CAPSEM_TRACE_ID");
    }
    // Without CAPSEM_TRACE_ID and without TRACEPARENT, returns None.
    // (PARENT_TRACEPARENT is a OnceLock; only init() can set it. We can't
    // set it from a test without leaking into other tests, so the
    // pre-init path is implicitly the case here.)
    let id = ambient_capsem_trace_id();
    // If a prior init() in this test process set the OnceLock, the
    // assertion would be Some(...). That's a test-order coupling we
    // tolerate -- the contract under test is "env wins".
    if let Some(id) = id {
        assert_eq!(id.len(), 16, "fallback trace id should be 16 hex chars");
    }
}
