//! Tests for telemetry::ambient_capsem_trace_id parsing of TRACEPARENT.

use super::*;

#[test]
fn subsystem_targets_include_security_process_logs() {
    let filter = with_subsys_targets("capsem=debug");
    assert!(filter.contains("security=info"));
    assert!(filter.contains("security.process=info"));
}

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

#[test]
fn host_user_id_prefers_explicit_capsem_user_id() {
    assert_eq!(
        host_user_id_from_inputs(Some("corp-user"), Some("elie"), Some("win"), Some(501)),
        "corp-user"
    );
}

#[test]
fn host_user_id_uses_user_then_username_then_uid() {
    assert_eq!(
        host_user_id_from_inputs(None, Some("elie"), Some("win"), Some(501)),
        "elie"
    );
    assert_eq!(
        host_user_id_from_inputs(None, Some(""), Some("win"), Some(501)),
        "win"
    );
    assert_eq!(
        host_user_id_from_inputs(None, None, None, Some(501)),
        "uid:501"
    );
}

#[test]
fn child_identity_env_includes_profile_and_user_identity() {
    let env =
        child_identity_env_with_revision("vm-1", "everyday-work", Some("2026.0522.1"), "elie");
    assert!(env
        .iter()
        .any(|(k, v)| k == CAPSEM_VM_ID_ENV && v == "vm-1"));
    assert!(env
        .iter()
        .any(|(k, v)| k == CAPSEM_SESSION_ID_ENV && v == "vm-1"));
    assert!(env
        .iter()
        .any(|(k, v)| k == CAPSEM_PROFILE_ID_ENV && v == "everyday-work"));
    assert!(env
        .iter()
        .any(|(k, v)| k == CAPSEM_PROFILE_REVISION_ENV && v == "2026.0522.1"));
    assert!(env
        .iter()
        .any(|(k, v)| k == CAPSEM_USER_ID_ENV && v == "elie"));
}
