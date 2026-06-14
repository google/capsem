//! Tests for telemetry::ambient_capsem_trace_id parsing of TRACEPARENT.

use super::*;

#[test]
fn ambient_trace_id_from_capsem_env_takes_precedence() {
    let id = resolve_ambient_capsem_trace_id(
        Some("deadbeefcafef00d"),
        Some("00-11111111111111112222222222222222-3333333333333333-01"),
    );
    assert_eq!(id.as_deref(), Some("deadbeefcafef00d"));
}

#[test]
fn ambient_trace_id_returns_none_without_env() {
    let id = resolve_ambient_capsem_trace_id(None, None);
    assert_eq!(id, None);
}

#[test]
fn ambient_trace_id_extracts_lower_half_from_traceparent() {
    let id = resolve_ambient_capsem_trace_id(
        None,
        Some("00-11111111111111112222222222222222-3333333333333333-01"),
    );
    assert_eq!(id.as_deref(), Some("2222222222222222"));
}

#[test]
fn debug_telemetry_policy_is_local_only_by_default() {
    let policy = debug_telemetry_policy_from_pairs([
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://collector.example:4317",
        ),
        ("OTEL_TRACES_EXPORTER", "otlp"),
    ]);

    assert!(!policy.local_debug_enabled);
    assert!(!policy.upstream_export_allowed);
    assert_eq!(
        policy.blocked_upstream_env,
        vec!["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_TRACES_EXPORTER"]
    );
}

#[test]
fn debug_telemetry_policy_enables_local_debug_filter_only() {
    let policy = debug_telemetry_policy_from_pairs([(DEBUG_TELEMETRY_ENV, "local")]);

    assert!(policy.local_debug_enabled);
    assert!(!policy.upstream_export_allowed);
    assert!(policy.blocked_upstream_env.is_empty());

    let filter = default_filter_with_debug_telemetry("capsem=info", &policy);
    assert!(filter.contains("capsem=info"));
    assert!(filter.contains("capsem.mitm=debug"));
    assert!(filter.contains("capsem.db=debug"));
}

#[test]
fn upstream_otel_requires_explicit_allow_env() {
    let policy = debug_telemetry_policy_from_pairs([
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://collector.example:4317",
        ),
        (ALLOW_UPSTREAM_OTEL_ENV, "true"),
    ]);

    assert!(policy.upstream_export_allowed);
    assert!(policy.blocked_upstream_env.is_empty());
}

#[test]
fn launch_span_names_match_contract() {
    for name in [
        LAUNCH_SERVICE_SPAN,
        LAUNCH_GATEWAY_SPAN,
        LAUNCH_PROCESS_SPAWN_SPAN,
        LAUNCH_VM_BOOT_SPAN,
        LAUNCH_VSOCK_READY_SPAN,
        LAUNCH_FIRST_NETWORK_READY_SPAN,
    ] {
        assert!(name.starts_with("capsem.launch."));
        assert!(!name.contains("path"));
        assert!(!name.contains("url"));
        assert!(!name.contains("host"));
    }
}
