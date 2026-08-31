use super::*;

fn make_resolved(id: &str, typ: SettingType, value: SettingValue) -> ResolvedSetting {
    ResolvedSetting {
        id: id.to_string(),
        category: "test".into(),
        name: id.to_string(),
        description: "".into(),
        setting_type: typ,
        default_value: value.clone(),
        effective_value: value,
        source: PolicySource::Default,
        modified: None,
        corp_locked: false,
        enabled_by: None,
        enabled: true,
        metadata: SettingMetadata::default(),
        collapsed: false,
        history: vec![],
    }
}

#[test]
fn lint_empty_settings_no_issues() {
    let issues = config_lint(&[]);
    assert!(issues.is_empty());
}

#[test]
fn lint_nul_byte_in_text() {
    let s = make_resolved(
        "test.key",
        SettingType::Text,
        SettingValue::Text("hello\0world".into()),
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("invalid characters")));
}

#[test]
fn lint_number_below_min() {
    let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(0));
    s.metadata.min = Some(1);
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.message.contains("below minimum")));
}

#[test]
fn lint_number_above_max() {
    let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(100));
    s.metadata.max = Some(50);
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.message.contains("exceeds maximum")));
}

#[test]
fn lint_number_in_range_no_issue() {
    let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(5));
    s.metadata.min = Some(1);
    s.metadata.max = Some(10);
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn lint_invalid_choice() {
    let mut s = make_resolved(
        "test.choice",
        SettingType::Text,
        SettingValue::Text("bad".into()),
    );
    s.metadata.choices = vec!["good".into(), "ok".into()];
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.message.contains("not a valid choice")));
}

#[test]
fn lint_valid_choice_no_issue() {
    let mut s = make_resolved(
        "test.choice",
        SettingType::Text,
        SettingValue::Text("good".into()),
    );
    s.metadata.choices = vec!["good".into(), "ok".into()];
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn lint_file_path_traversal() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        SettingValue::File {
            path: "/root/../etc/shadow".into(),
            content: "".into(),
        },
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.message.contains("must not contain '..'")));
}

#[test]
fn lint_file_path_not_absolute() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        SettingValue::File {
            path: "relative/path.txt".into(),
            content: "".into(),
        },
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.message.contains("must be absolute")));
}

#[test]
fn lint_file_invalid_json_content() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        SettingValue::File {
            path: "/root/.config/settings.json".into(),
            content: "not json {{{".into(),
        },
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.message.contains("invalid JSON")));
}

#[test]
fn lint_api_key_with_whitespace() {
    let s = make_resolved(
        "ai.test.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-abc 123\n".into()),
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.message.contains("whitespace")));
}

#[test]
fn lint_url_not_http() {
    let s = make_resolved(
        "test.url",
        SettingType::Url,
        SettingValue::Text("ftp://example.com".into()),
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.message.contains("not a valid URL")));
}

#[test]
fn lint_url_valid_https() {
    let s = make_resolved(
        "test.url",
        SettingType::Url,
        SettingValue::Text("https://example.com".into()),
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}
