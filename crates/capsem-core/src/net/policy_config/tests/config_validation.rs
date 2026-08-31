use super::*;

// -----------------------------------------------------------------------
// TOML registry tests
// -----------------------------------------------------------------------

#[test]
fn toml_registry_parses() {
    // The embedded defaults.toml must parse without panicking.
    let defs = setting_definitions();
    assert!(!defs.is_empty(), "defaults.toml must produce at least one setting");
}

#[test]
fn toml_registry_setting_count() {
    // Guard against accidental deletions. Update this if settings are
    // intentionally added or removed.
    let defs = setting_definitions();
    assert!(
        defs.len() >= 20,
        "expected at least 20 settings from defaults.toml, got {}",
        defs.len(),
    );
}

#[test]
fn toml_registry_ids_from_path() {
    // IDs are dot-separated paths derived from the TOML table nesting.
    let defs = setting_definitions();
    for def in &defs {
        assert!(def.id.contains('.'), "setting id '{}' should be a dotted path", def.id,);
    }
}

#[test]
fn toml_registry_category_inherited() {
    // Category is inherited from the nearest ancestor group with a `name`.
    let defs = setting_definitions();
    let github_allow = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        !github_allow.category.is_empty(),
        "repository.providers.github.allow should have a category inherited from its group",
    );
}

#[test]
fn toml_registry_enabled_by_inherited() {
    // enabled_by is inherited from the group and applied to children
    // but NOT to the toggle setting itself.
    let defs = setting_definitions();
    let allow = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        allow.enabled_by.is_none(),
        "the toggle itself should not have enabled_by",
    );
    let api_key = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        api_key.enabled_by.as_deref(),
        Some(SETTING_GITHUB_ALLOW),
        "token should inherit enabled_by from its group",
    );
}

#[test]
fn toml_registry_meta_fields() {
    // Metadata fields (domains, choices, rules, env_vars)
    // are correctly parsed from the `meta` sub-table.
    let defs = setting_definitions();

    // Registry toggles should have domains in metadata
    let github = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        !github.metadata.domains.is_empty(),
        "github toggle should have domain metadata"
    );

    // security.web.http_upstream_ports should be network mechanics, not a decision toggle.
    let ports = defs
        .iter()
        .find(|d| d.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(
        ports.setting_type,
        SettingType::IntList,
        "http_upstream_ports should be an int list"
    );

    assert!(
        defs.iter().all(|d| !d.id.starts_with("ai.")),
        "AI provider controls must not be settings-owned"
    );
}

// -----------------------------------------------------------------------
// Config lint tests
// -----------------------------------------------------------------------

fn make_resolved(
    id: &str,
    stype: SettingType,
    value: SettingValue,
    meta: SettingMetadata,
    enabled_by: Option<&str>,
) -> ResolvedSetting {
    ResolvedSetting {
        id: id.to_string(),
        category: "Test".to_string(),
        name: id.to_string(),
        description: "test".to_string(),
        setting_type: stype,
        default_value: value.clone(),
        effective_value: value,
        source: PolicySource::Default,
        modified: None,
        corp_locked: false,
        enabled_by: enabled_by.map(String::from),
        enabled: true,
        metadata: meta,
        collapsed: false,
        history: Vec::new(),
    }
}

// -- JSON validation (File values) --

fn file_val(path: &str, content: &str) -> SettingValue {
    SettingValue::File {
        path: path.into(),
        content: content.into(),
    }
}

#[test]
fn config_lint_valid_json_passes() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"key":"val"}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_malformed_json_gives_clear_error() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", "{bad json}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("invalid JSON")));
}

#[test]
fn config_lint_json_not_object_warns() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", "42"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("not an object")));
}

#[test]
fn config_lint_empty_json_file_ok() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", ""),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_with_trailing_comma_gives_error() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"a":1,}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.severity == "error"));
}

#[test]
fn config_lint_json_with_unicode_passes() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"name":"cafe\u0301"}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_deeply_nested_passes() {
    let json = r#"{"a":{"b":{"c":{"d":{"e":"deep"}}}}}"#;
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", json),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_huge_payload_passes() {
    let big_val = "x".repeat(1_000_000);
    let json = format!(r#"{{"data":"{}"}}"#, big_val);
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", &json),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_file_path_must_be_absolute() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("relative/path.json", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("absolute")));
}

#[test]
fn config_lint_file_path_no_traversal() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/../etc/passwd", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("..")));
}

#[test]
fn config_lint_file_unusual_path_warns() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/tmp/test.json", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("unusual")));
}

// -- Number validation --

#[test]
fn config_lint_number_in_range_ok() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(4), meta, None);
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_number_below_min_error() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(0), meta, None);
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("below minimum"));
}

#[test]
fn config_lint_number_above_max_error() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved("vm.disk", SettingType::Number, SettingValue::Number(256), meta, None);
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("exceeds maximum"));
}

#[test]
fn config_lint_number_at_boundary_ok() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s1 = make_resolved(
        "vm.min",
        SettingType::Number,
        SettingValue::Number(1),
        meta.clone(),
        None,
    );
    let s2 = make_resolved("vm.max", SettingType::Number, SettingValue::Number(128), meta, None);
    let issues = config_lint(&[s1, s2]);
    assert!(issues.is_empty());
}

// -- Choice validation --

#[test]
fn config_lint_valid_choice_ok() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("deny".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_invalid_choice_error() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("block".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("not a valid choice"));
}

#[test]
fn config_lint_empty_choice_when_choices_defined_error() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
}

#[test]
fn config_lint_case_sensitive_choice() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("Allow".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1, "'Allow' != 'allow' -- case sensitive");
}

// -- API key validation --

#[test]
fn config_lint_apikey_with_whitespace_warns() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant key".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("whitespace")));
}

#[test]
fn config_lint_apikey_with_newline_warns() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant\n".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("whitespace")));
}

#[test]
fn config_lint_apikey_empty_when_enabled_warns() {
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(true),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        SettingMetadata::default(),
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("not set")));
}

#[test]
fn config_lint_apikey_empty_when_disabled_ok() {
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(false),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        SettingMetadata::default(),
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    assert!(issues.is_empty(), "disabled provider with empty key is fine");
}

#[test]
fn config_lint_apikey_normal_value_ok() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant-api03-valid".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

// -- Text validation --

#[test]
fn config_lint_text_with_nul_byte_error() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("hello\0world".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("invalid characters"));
}

#[test]
fn config_lint_text_normal_ok() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("hello".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_text_unicode_ok() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("cafe\u{0301}".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_text_very_long_ok() {
    let long_val = "x".repeat(10_000);
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text(long_val),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

// -- Serialization roundtrip --

#[test]
fn config_lint_all_issues_serialize_deserialize() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
    let issues = config_lint(&[s]);
    let json = serde_json::to_string(&issues).unwrap();
    let roundtrip: Vec<ConfigIssue> = serde_json::from_str(&json).unwrap();
    assert_eq!(issues, roundtrip);
}

#[test]
fn config_lint_issue_messages_are_nonempty() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
    let issues = config_lint(&[s]);
    for issue in &issues {
        assert!(!issue.message.is_empty());
        assert!(!issue.id.is_empty());
    }
}

#[test]
fn config_lint_issue_ids_are_valid_setting_ids() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved(
        "vm.resources.cpu_count",
        SettingType::Number,
        SettingValue::Number(99),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    for issue in &issues {
        assert_eq!(issue.id, "vm.resources.cpu_count");
    }
}

// -- Integration --

#[test]
fn config_lint_default_config_has_no_errors() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let issues = config_lint(&resolved);
    let errors: Vec<_> = issues.iter().filter(|i| i.severity == "error").collect();
    assert!(errors.is_empty(), "default config should have no errors: {errors:?}");
}

#[test]
fn config_lint_returns_multiple_issues() {
    let meta_num = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s1 = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta_num, None);
    let s2 = make_resolved(
        "v.f",
        SettingType::File,
        file_val("/root/test.json", "{bad}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s1, s2]);
    assert!(issues.len() >= 2, "expected multiple issues: {issues:?}");
}

// -- docs_url --

#[test]
fn config_lint_empty_key_has_docs_url() {
    let meta = SettingMetadata {
        docs_url: Some("https://example.com/keys".into()),
        ..Default::default()
    };
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(true),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        meta,
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    let empty_key_issue = issues.iter().find(|i| i.message.contains("not set")).unwrap();
    assert_eq!(empty_key_issue.docs_url.as_deref(), Some("https://example.com/keys"));
}

#[test]
fn config_lint_non_key_issue_no_docs_url() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
    let issues = config_lint(&[s]);
    assert!(!issues.is_empty());
    for issue in &issues {
        assert!(issue.docs_url.is_none(), "non-key issues should not have docs_url");
    }
}

#[test]
fn docs_url_parsed_from_toml() {
    let defs = setting_definitions();
    let github_token = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        github_token.metadata.docs_url.as_deref(),
        Some("https://github.com/settings/tokens")
    );
}
