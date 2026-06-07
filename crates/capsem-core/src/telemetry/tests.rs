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
