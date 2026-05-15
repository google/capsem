//! Tests for telemetry::ambient_capsem_trace_id parsing of TRACEPARENT.

use super::*;

#[test]
fn ambient_trace_id_from_capsem_env_takes_precedence() {
    let id = ambient_capsem_trace_id_from_inputs(
        Some("deadbeefcafef00d"),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
    );
    assert_eq!(id.as_deref(), Some("deadbeefcafef00d"));
}

#[test]
fn ambient_trace_id_returns_none_without_env() {
    let id = ambient_capsem_trace_id_from_inputs(None, None);
    assert_eq!(id, None);
}

#[test]
fn ambient_trace_id_falls_back_to_parent_traceparent() {
    let id = ambient_capsem_trace_id_from_inputs(
        None,
        Some("00-11112222333344445555666677778888-0123456789abcdef-01"),
    );
    assert_eq!(id.as_deref(), Some("5555666677778888"));
}

#[test]
fn ambient_trace_id_ignores_empty_env_and_uses_parent() {
    let id = ambient_capsem_trace_id_from_inputs(
        Some(""),
        Some("00-1234567890abcdef1234567890abcdef-fedcba0987654321-01"),
    );
    assert_eq!(id.as_deref(), Some("1234567890abcdef"));
}

#[test]
fn ambient_trace_id_rejects_short_parent_trace_id() {
    let id = ambient_capsem_trace_id_from_inputs(None, Some("00-deadbeef-bbbbbbbbbbbbbbbb-01"));
    assert_eq!(id, None);
}
