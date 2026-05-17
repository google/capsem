use super::super::*;
use super::*;

#[test]
fn resolver_trace_event_serializes_required_fields() {
    let event = ResolverTraceEvent {
        step: 7,
        path: "security.rules.http.x".to_string(),
        operation: ResolverTraceOperation::Set,
        source_kind: ResolverTraceSourceKind::Profile,
        source_profile_id: Some("strict".to_string()),
        source_label: "profile rule".to_string(),
        before: None,
        after: Some(serde_json::json!({"decision": "block"})),
        locked: true,
        reason: Some("override parent".to_string()),
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["step"], 7);
    assert_eq!(json["path"], "security.rules.http.x");
    assert_eq!(json["operation"], "set");
    assert_eq!(json["source_kind"], "profile");
    assert_eq!(json["source_profile_id"], "strict");
    assert_eq!(json["locked"], true);
    assert_eq!(json["after"]["decision"], "block");
}

#[test]
fn resolver_trace_append_numbers_steps_monotonically_from_zero() {
    let mut trace = ResolverTrace::new();
    for path in ["a", "b", "c"] {
        trace.append(ResolverTraceEvent {
            step: 999, // intentionally wrong; append must overwrite
            path: path.to_string(),
            operation: ResolverTraceOperation::Set,
            source_kind: ResolverTraceSourceKind::Default,
            source_profile_id: None,
            source_label: "test".to_string(),
            before: None,
            after: None,
            locked: false,
            reason: None,
        });
    }
    let steps: Vec<u32> = trace.events.iter().map(|event| event.step).collect();
    assert_eq!(steps, vec![0, 1, 2]);
}

#[test]
fn resolver_trace_round_trip_through_disk() {
    let temp = tempfile::tempdir().unwrap();
    let mut trace = ResolverTrace::new();
    trace.append(ResolverTraceEvent {
        step: 0,
        path: "*".to_string(),
        operation: ResolverTraceOperation::Set,
        source_kind: ResolverTraceSourceKind::Default,
        source_profile_id: None,
        source_label: "schema defaults".to_string(),
        before: None,
        after: None,
        locked: false,
        reason: None,
    });
    let written = write_vm_effective_trace(temp.path(), &trace).unwrap();
    assert!(written.ends_with(VM_EFFECTIVE_TRACE_FILENAME));
    let loaded = load_vm_effective_trace(temp.path()).unwrap();
    assert_eq!(loaded, trace);
}

#[test]
fn load_vm_effective_trace_fails_clearly_on_corrupt_json() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(VM_EFFECTIVE_TRACE_FILENAME),
        "{ not valid json",
    )
    .unwrap();
    let error = load_vm_effective_trace(temp.path()).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Parse { kind, .. } if kind == "vm-effective trace"),
        "expected Parse error, got {error:?}"
    );
}

#[test]
fn load_vm_effective_trace_fails_clearly_on_missing_file() {
    let temp = tempfile::tempdir().unwrap();
    let error = load_vm_effective_trace(temp.path()).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::ReadFile { .. }),
        "expected ReadFile error, got {error:?}"
    );
}

#[test]
fn resolve_effective_vm_settings_with_trace_emits_default_and_ancestor_events() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(
        base_dir.join("parent.toml"),
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"
"#,
    )
    .unwrap();
    std::fs::write(
        base_dir.join("child.toml"),
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"
"#,
    )
    .unwrap();

    let roots = ProfileRootSettings {
        base_dirs: vec![base_dir],
        corp_dirs: Vec::new(),
        user_dirs: vec![user_dir],
        default_profile: EVERYDAY_WORK_PROFILE_ID.to_string(),
        allow_user_profiles: true,
        allow_user_fork: true,
        allow_user_delete: true,
    };
    let (_effective, trace) =
        resolve_effective_vm_settings_with_trace(&roots, Some("child")).unwrap();

    // First event is the schema-default baseline.
    let head = trace.events.first().expect("trace must have events");
    assert_eq!(head.step, 0);
    assert_eq!(head.source_kind, ResolverTraceSourceKind::Default);
    assert_eq!(head.path, "*");

    // Followed by one profile event per ancestor (parent, then child).
    let profile_events: Vec<&ResolverTraceEvent> = trace
        .events
        .iter()
        .filter(|event| matches!(event.source_kind, ResolverTraceSourceKind::Profile))
        .collect();
    let profile_paths: Vec<&str> = profile_events
        .iter()
        .filter(|event| event.path.starts_with("profiles."))
        .map(|event| event.path.as_str())
        .collect();
    assert_eq!(profile_paths, vec!["profiles.parent", "profiles.child"]);
}

#[test]
fn resolver_trace_summary_captures_counts_and_tail() {
    let mut trace = ResolverTrace::new();
    for path in ["a", "b", "c", "d", "e", "f"] {
        trace.append(ResolverTraceEvent {
            step: 0,
            path: path.to_string(),
            operation: ResolverTraceOperation::Set,
            source_kind: ResolverTraceSourceKind::Profile,
            source_profile_id: Some("test".to_string()),
            source_label: "test".to_string(),
            before: None,
            after: None,
            locked: false,
            reason: None,
        });
    }
    trace.append(ResolverTraceEvent {
        step: 0,
        path: "locked-path".to_string(),
        operation: ResolverTraceOperation::Lock,
        source_kind: ResolverTraceSourceKind::Corp,
        source_profile_id: None,
        source_label: "corp_directives[0]".to_string(),
        before: None,
        after: None,
        locked: true,
        reason: None,
    });
    let summary = trace.summary(3);
    assert_eq!(summary.event_count, 7);
    assert_eq!(summary.corp_event_count, 1);
    assert_eq!(summary.locked_paths, vec!["locked-path"]);
    assert_eq!(summary.last_events.len(), 3);
    let last_paths: Vec<&str> = summary
        .last_events
        .iter()
        .map(|event| event.path.as_str())
        .collect();
    assert_eq!(last_paths, vec!["e", "f", "locked-path"]);
}

#[test]
fn resolver_trace_summary_records_rejected_paths_from_violation_events() {
    let mut trace = ResolverTrace::new();
    trace.append(ResolverTraceEvent {
        step: 0,
        path: "security.rules.http.x".to_string(),
        operation: ResolverTraceOperation::Reject,
        source_kind: ResolverTraceSourceKind::Corp,
        source_profile_id: None,
        source_label: "corp_directives[5]".to_string(),
        before: None,
        after: None,
        locked: false,
        reason: Some("path is locked".to_string()),
    });
    let summary = trace.summary(8);
    assert_eq!(summary.rejected_paths, vec!["security.rules.http.x"]);
}

#[test]
fn resolver_trace_is_deterministic_for_identical_input() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(
        base_dir.join("only.toml"),
        r#"
version = 1
id = "only"
name = "Only"
best_for = "Only."
profile_type = "coding"

[security.rules.http.a]
on = "http.request"
if = "true"
decision = "allow"

[security.rules.http.b]
on = "http.request"
if = "true"
decision = "block"
"#,
    )
    .unwrap();
    let roots = ProfileRootSettings {
        base_dirs: vec![base_dir],
        corp_dirs: Vec::new(),
        user_dirs: vec![user_dir],
        default_profile: "only".to_string(),
        allow_user_profiles: true,
        allow_user_fork: true,
        allow_user_delete: true,
    };
    let (_e1, t1) = resolve_effective_vm_settings_with_trace(&roots, Some("only")).unwrap();
    let (_e2, t2) = resolve_effective_vm_settings_with_trace(&roots, Some("only")).unwrap();
    assert_eq!(t1, t2, "trace must be deterministic across two runs");
}
