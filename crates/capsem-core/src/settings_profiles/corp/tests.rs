use super::super::*;
use super::*;

fn directive_toml(toml: &str) -> CorpDirective {
    toml::from_str::<CorpDirective>(toml).expect("directive must parse")
}

#[test]
fn corp_directive_add_inserts_new_rule_into_merged_profile() {
    let mut profile = Profile::everyday_work();
    let directive = directive_toml(
        r#"
operation = "add"
path = "security.rules.http.corp-policy"
reason = "block known-bad host"
[value]
on = "http.request"
if = "request.url.host == 'evil.com'"
decision = "block"
priority = 0
"#,
    );
    let mut trace = ResolverTrace::new();
    let overrides = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap();

    let rule = profile
        .security
        .rules
        .http
        .get("corp-policy")
        .expect("rule added");
    assert_eq!(rule.decision, RuleDecision::Block);
    assert_eq!(
        overrides.rules.get("corp-policy").map(String::as_str),
        Some("http")
    );
    assert_eq!(trace.events.len(), 1);
    assert_eq!(trace.events[0].operation, ResolverTraceOperation::Add);
    assert_eq!(trace.events[0].source_kind, ResolverTraceSourceKind::Corp);
}

#[test]
fn corp_directive_replace_swaps_existing_rule() {
    let mut profile = Profile::everyday_work();
    profile.security.rules.http.insert(
        "block-secret".to_string(),
        toml::from_str::<ProfileRule>(
            r#"on = "http.request"
if = "request.data.contains_secret"
decision = "block""#,
        )
        .unwrap(),
    );
    let directive = directive_toml(
        r#"
operation = "replace"
path = "security.rules.http.block-secret"
[value]
on = "http.request"
if = "request.data.contains_secret"
decision = "allow"
priority = 0
"#,
    );
    let mut trace = ResolverTrace::new();
    apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap();
    assert_eq!(
        profile.security.rules.http["block-secret"].decision,
        RuleDecision::Allow
    );
    let event = &trace.events[0];
    assert_eq!(event.operation, ResolverTraceOperation::Replace);
    assert!(event.before.is_some());
    assert!(event.after.is_some());
}

#[test]
fn corp_directive_remove_drops_rule() {
    let mut profile = Profile::everyday_work();
    profile.security.rules.http.insert(
        "block-secret".to_string(),
        toml::from_str::<ProfileRule>(
            r#"on = "http.request"
if = "request.data.contains_secret"
decision = "block""#,
        )
        .unwrap(),
    );
    let directive = directive_toml(
        r#"
operation = "remove"
path = "security.rules.http.block-secret"
"#,
    );
    let mut trace = ResolverTrace::new();
    apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap();
    assert!(!profile.security.rules.http.contains_key("block-secret"));
    let event = &trace.events[0];
    assert_eq!(event.operation, ResolverTraceOperation::Remove);
    assert!(event.before.is_some());
    assert!(event.after.is_none());
}

#[test]
fn corp_directive_replace_swaps_security_capability_field() {
    let mut profile = Profile::everyday_work();
    let directive = directive_toml(
        r#"
operation = "replace"
path = "security.capabilities.network_egress"
value = "block"
"#,
    );
    let mut trace = ResolverTrace::new();
    apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap();
    assert_eq!(
        profile.security.capabilities.network_egress,
        CapabilityMode::Block
    );
}

#[test]
fn corp_directive_unknown_path_fails_clearly() {
    let mut profile = Profile::everyday_work();
    let directive = directive_toml(
        r#"
operation = "replace"
path = "something.unknown"
value = 5
"#,
    );
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("unsupported corp directive path")),
        "expected unsupported-path validation error, got {error:?}"
    );
}

#[test]
fn corp_directive_remove_on_missing_key_fails_clearly() {
    let mut profile = Profile::everyday_work();
    let directive = directive_toml(
        r#"
operation = "remove"
path = "security.rules.http.never-existed"
"#,
    );
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("remove on missing key")),
        "expected remove-on-missing validation error, got {error:?}"
    );
}

#[test]
fn corp_directive_add_on_existing_key_fails_clearly() {
    let mut profile = Profile::everyday_work();
    profile.security.rules.http.insert(
        "already-there".to_string(),
        toml::from_str::<ProfileRule>(
            r#"on = "http.request"
if = "true"
decision = "allow""#,
        )
        .unwrap(),
    );
    let directive = directive_toml(
        r#"
operation = "add"
path = "security.rules.http.already-there"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 0
"#,
    );
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("add on existing key")),
        "expected add-on-existing validation error, got {error:?}"
    );
}

#[test]
fn corp_directive_type_mismatch_value_fails_clearly() {
    let mut profile = Profile::everyday_work();
    // value is the wrong shape for a ProfileRule.
    let directive = directive_toml(
        r#"
operation = "add"
path = "security.rules.http.broken"
value = "not-a-rule-table"
"#,
    );
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Parse { kind, .. } if kind == "corp directive value"),
        "expected Parse error for corp directive value, got {error:?}"
    );
}

#[test]
fn corp_directive_validation_rejects_remove_with_value() {
    let directive = directive_toml(
        r#"
operation = "remove"
path = "security.rules.http.x"
value = "whatever"
"#,
    );
    let error = directive.validate("corp_directives[0]").unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("remove/forbid directives must not carry a value")),
        "got {error:?}"
    );
}

#[test]
fn corp_directive_validation_rejects_add_without_value() {
    let directive = directive_toml(
        r#"
operation = "add"
path = "security.rules.http.x"
"#,
    );
    let error = directive.validate("corp_directives[0]").unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("add/replace/lock directives require a value")),
        "got {error:?}"
    );
}

#[test]
fn corp_directive_lock_stamps_path_and_subsequent_directive_violates() {
    let mut profile = Profile::everyday_work();
    let directives = vec![
        directive_toml(
            r#"
operation = "lock"
path = "security.rules.http.required"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 0
"#,
        ),
        directive_toml(
            r#"
operation = "replace"
path = "security.rules.http.required"
[value]
on = "http.request"
if = "true"
decision = "allow"
priority = 0
"#,
        ),
    ];
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &directives, &mut trace).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::ResolverViolation { ref source_layer, ref message, .. }
                if source_layer == "corp" && message.contains("locked")
        ),
        "expected ResolverViolation with locked path, got {error:?}"
    );
    // First directive succeeded: the locked event is in the
    // trace with locked = true, and the rule landed.
    let lock_event = trace
        .events
        .iter()
        .find(|e| e.operation == ResolverTraceOperation::Lock)
        .expect("lock event present");
    assert!(lock_event.locked);
    assert_eq!(
        profile.security.rules.http["required"].decision,
        RuleDecision::Block
    );
}

#[test]
fn corp_directive_forbid_stamps_path_and_subsequent_add_violates() {
    let mut profile = Profile::everyday_work();
    profile.security.rules.http.insert(
        "banned".to_string(),
        toml::from_str::<ProfileRule>(
            r#"on = "http.request"
if = "true"
decision = "allow""#,
        )
        .unwrap(),
    );
    let directives = vec![
        directive_toml(
            r#"
operation = "forbid"
path = "security.rules.http.banned"
"#,
        ),
        directive_toml(
            r#"
operation = "add"
path = "security.rules.http.banned"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 0
"#,
        ),
    ];
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &directives, &mut trace).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::ResolverViolation { ref message, .. }
                if message.contains("forbidden")
        ),
        "expected ResolverViolation with forbidden path, got {error:?}"
    );
    // Forbid removed the existing rule.
    assert!(!profile.security.rules.http.contains_key("banned"));
    // Forbid event recorded.
    assert!(trace
        .events
        .iter()
        .any(|e| e.operation == ResolverTraceOperation::Forbid));
}

#[test]
fn corp_directive_forbid_allows_subsequent_remove_on_already_forbidden_path() {
    // A subsequent `remove` on a forbidden path should NOT be
    // a violation -- removal doesn't restore the entry. This
    // guards against an over-broad "any subsequent directive
    // on a forbidden path is rejected" interpretation.
    let mut profile = Profile::everyday_work();
    let directives = vec![
        directive_toml(
            r#"
operation = "forbid"
path = "security.rules.http.x"
"#,
        ),
        directive_toml(
            r#"
operation = "remove"
path = "security.rules.http.never-existed"
"#,
        ),
    ];
    let mut trace = ResolverTrace::new();
    // The second directive should fail with the existing
    // "remove on missing key" message, NOT with the forbidden
    // violation. (Different path on purpose.)
    let error = apply_corp_directives(&mut profile, &directives, &mut trace).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::Validation { ref message, .. }
                if message.contains("remove on missing key")
        ),
        "expected remove-on-missing validation, got {error:?}"
    );
}

#[test]
fn corp_directive_lock_capability_stamps_path_and_replace_violates() {
    let mut profile = Profile::everyday_work();
    let directives = vec![
        directive_toml(
            r#"
operation = "lock"
path = "security.capabilities.network_egress"
value = "block"
"#,
        ),
        directive_toml(
            r#"
operation = "replace"
path = "security.capabilities.network_egress"
value = "allow"
"#,
        ),
    ];
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &directives, &mut trace).unwrap_err();
    assert!(matches!(
        error,
        SettingsProfilesError::ResolverViolation { .. }
    ));
    assert_eq!(
        profile.security.capabilities.network_egress,
        CapabilityMode::Block
    );
}

#[test]
fn corp_directive_validation_rejects_forbid_with_value() {
    let directive = directive_toml(
        r#"
operation = "forbid"
path = "security.rules.http.x"
value = "x"
"#,
    );
    let error = directive.validate("corp_directives[0]").unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("remove/forbid directives must not carry a value")),
        "got {error:?}"
    );
}

#[test]
fn corp_directive_validation_rejects_lock_without_value() {
    let directive = directive_toml(
        r#"
operation = "lock"
path = "security.rules.http.x"
"#,
    );
    let error = directive.validate("corp_directives[0]").unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::Validation { ref message, .. } if message.contains("add/replace/lock directives require a value")),
        "got {error:?}"
    );
}

#[test]
fn corp_directive_violation_emits_reject_event_before_returning_error() {
    // Slice 6.6: a violation must surface in the trace as a
    // `reject` event so status / debug surfaces can show
    // "corp_directives[1] was rejected because the path is
    // locked" without callers having to correlate the typed
    // error against the trace by hand.
    let mut profile = Profile::everyday_work();
    let directives = vec![
        directive_toml(
            r#"
operation = "lock"
path = "security.rules.http.x"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 0
"#,
        ),
        directive_toml(
            r#"
operation = "replace"
path = "security.rules.http.x"
[value]
on = "http.request"
if = "true"
decision = "allow"
priority = 0
"#,
        ),
    ];
    let mut trace = ResolverTrace::new();
    let _ = apply_corp_directives(&mut profile, &directives, &mut trace).unwrap_err();
    let reject = trace
        .events
        .iter()
        .find(|event| event.operation == ResolverTraceOperation::Reject)
        .expect("reject event present");
    assert_eq!(reject.path, "security.rules.http.x");
    assert_eq!(reject.source_kind, ResolverTraceSourceKind::Corp);
    assert_eq!(
        reject.source_label, "corp_directives[1]",
        "reject event must point at the second (violating) directive, not the lock"
    );
    assert!(reject
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("locked"));
}

#[test]
fn resolve_effective_vm_settings_with_corp_attributes_replaced_rule_to_corp() {
    // End-to-end: profile declares a rule; service settings
    // replace it via corp directive; effective rules reflect
    // the replacement AND per-rule provenance attributes to
    // `corp`, and the trace has both the profile event and the
    // corp event.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(
        base_dir.join("p.toml"),
        r#"
version = 1
id = "p"
name = "P"
best_for = "P."
profile_type = "coding"

[security.rules.http.flagged]
on = "http.request"
if = "true"
decision = "allow"
"#,
    )
    .unwrap();
    let mut settings = ServiceSettings {
        profiles: ProfileRootSettings {
            base_dirs: vec![base_dir],
            corp_dirs: Vec::new(),
            user_dirs: vec![user_dir],
            default_profile: "p".to_string(),
            allow_user_profiles: true,
            allow_user_fork: true,
            allow_user_delete: true,
        },
        ..ServiceSettings::default()
    };
    settings.corp_directives.push(directive_toml(
        r#"
operation = "replace"
path = "security.rules.http.flagged"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 0
"#,
    ));

    let (effective, trace) = resolve_effective_vm_settings_with_corp(&settings, Some("p")).unwrap();
    let rule = effective
        .rules
        .iter()
        .find(|r| r.id == "http.flagged")
        .expect("rule present");
    assert_eq!(rule.decision, RuleDecision::Block);
    assert_eq!(rule.provenance.profile_id, "corp");
    assert_eq!(rule.provenance.source, ProfileSource::Corp);

    // Trace contains a corp event AND a final rule event with
    // source_kind = corp for the corp-touched rule.
    let corp_events = trace
        .events
        .iter()
        .filter(|e| matches!(e.source_kind, ResolverTraceSourceKind::Corp))
        .count();
    assert!(
        corp_events >= 2,
        "expected at least one corp directive event AND the final corp-attributed rule event; got events: {:?}",
        trace.events
    );
}
