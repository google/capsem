use super::*;

// -----------------------------------------------------------------------
// Q: MergedPolicies basic construction (6)
// -----------------------------------------------------------------------

fn file_with_mcp(entries: Vec<(&str, SettingValue)>, mcp: crate::mcp::policy::McpProfileConfig) -> SettingsFile {
    let mut f = file_with(entries);
    f.mcp = Some(mcp);
    f
}

#[test]
fn merged_defaults_only() {
    let m = MergedPolicies::from_files(&empty_file(), &empty_file()).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
    assert!(has_security_rule(&m, "profiles.rules.default_dns"));
}

#[test]
fn merged_user_enables_provider() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_user_enables_search() {
    let user = file_with(vec![(
        "security.services.search.google.allow",
        SettingValue::Bool(true),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_all_policies_populated() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    assert!(!m.security_rules.rules().is_empty());
    // Guest config still carries non-secret built-in shell env defaults.
    assert!(m.guest.env.is_some());
    // VM settings have defaults
    assert!(m.vm.cpu_count.is_some());
}

// -----------------------------------------------------------------------
// S: Corp override persistence (11)
// -----------------------------------------------------------------------

#[test]
fn corp_forces_provider_on() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
    let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn corp_forces_provider_off() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
    let m = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn corp_sets_api_key() {
    let user = file_with(vec![(
        "ai.openai.api_key",
        SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into()),
    )]);
    let corp = file_with(vec![(
        "ai.openai.api_key",
        SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into()),
    )]);
    let m = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    let env = m.guest.env.unwrap_or_default();
    assert!(!env.contains_key("OPENAI_API_KEY"));
}

#[test]
fn corp_sets_network_mechanics_ports() {
    let user = empty_file();
    let corp = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80]),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let ports = resolved
        .iter()
        .find(|setting| setting.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(ports.effective_value, SettingValue::IntList(vec![80]));
    assert_eq!(ports.source, PolicySource::Corp);
}

#[test]
fn retired_web_decision_settings_are_not_resolved() {
    let user = file_with(vec![
        ("security.web.allow_read", SettingValue::Bool(true)),
        ("security.web.allow_write", SettingValue::Bool(true)),
        (
            "security.web.custom_allow",
            SettingValue::Text("internal.corp.com".into()),
        ),
        ("security.web.custom_block", SettingValue::Text("evil.com".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    for retired_id in [
        "security.web.allow_read",
        "security.web.allow_write",
        "security.web.custom_allow",
        "security.web.custom_block",
    ] {
        assert!(
            resolved.iter().all(|setting| setting.id != retired_id),
            "{retired_id} must not be a resolved setting"
        );
    }
}

// -----------------------------------------------------------------------
// T: Invalid / missing / corrupt inputs (13)
// -----------------------------------------------------------------------

#[test]
fn merged_from_missing_user_toml() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("missing_settings.toml");
    let user = load_settings_file(&nonexistent).unwrap_or_default();
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // Should produce valid defaults without panicking
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_missing_corp_toml() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("missing_corp.toml");
    let corp = load_settings_file(&nonexistent).unwrap_or_default();
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_from_both_missing() {
    let dir = tempfile::tempdir().unwrap();
    let u = load_settings_file(&dir.path().join("u.toml")).unwrap_or_default();
    let c = load_settings_file(&dir.path().join("c.toml")).unwrap_or_default();
    let m = MergedPolicies::from_files(&u, &c).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_invalid_user_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "not valid {{{{ toml").unwrap();
    let result = load_settings_file(&path);
    assert!(result.is_err());
    // Fallback to default still works
    let user = result.unwrap_or_default();
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_invalid_corp_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad_corp.toml");
    std::fs::write(&path, "garbage!!!!").unwrap();
    let result = load_settings_file(&path);
    assert!(result.is_err());
    let corp = result.unwrap_or_default();
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_ignores_unknown_setting_ids() {
    let user = file_with(vec![
        ("nonexistent.setting.foo", SettingValue::Bool(true)),
        ("ai.anthropic.allow", SettingValue::Bool(true)),
    ]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // Should not crash, anthropic should still work
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_wrong_type_for_bool_setting() {
    // SettingValue::Text for a Bool-type setting -- resolve will use default
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Text("yes".into()))]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // Provider detection/default rules are independent from legacy allow
    // toggles; malformed toggle values do not create network decisions.
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_wrong_type_for_number_setting() {
    let user = file_with(vec![("vm.resources.cpu_count", SettingValue::Text("four".into()))]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // as_number() returns None -> falls back to default (4)
    assert_eq!(m.vm.cpu_count, Some(4));
}

#[test]
fn merged_retired_custom_allow_setting_is_ignored() {
    let user = file_with(vec![("security.web.custom_allow", SettingValue::Text("".into()))]);
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // Should not crash, empty string -> no domains added
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_empty_mcp_section() {
    use crate::mcp::policy::McpProfileConfig;
    let user = file_with_mcp(vec![], McpProfileConfig::default());
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_partial_settings_file() {
    // TOML with only [mcp] section, no [settings]
    use crate::mcp::policy::McpProfileConfig;
    let user = SettingsFile {
        settings: HashMap::new(),
        mcp: Some(McpProfileConfig {
            health_check_interval_secs: Some(30),
            ..Default::default()
        }),
        ..Default::default()
    };
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // No settings -> defaults for everything else
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_partial_settings_only() {
    // Settings but no MCP section
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    assert!(user.mcp.is_none());
    let m = MergedPolicies::from_files(&user, &empty_file()).expect("policies merge");
    // Settings applied
    assert!(has_security_rule(&m, "profiles.rules.ai_anthropic_http_api"));
}

#[test]
fn merged_settings_expose_typed_plugin_policy_with_corp_override() {
    let user: SettingsFile = toml::from_str(
        r#"
[plugins]
[plugins.dummy_pre]
mode = "rewrite"
detection_level = "medium"

[plugins.dummy_post]
mode = "allow"
"#,
    )
    .expect("user plugin policy parses");
    let corp: SettingsFile = toml::from_str(
        r#"
[plugins.dummy_post]
mode = "block"
detection_level = "critical"

[plugins.dummy_disabled]
mode = "disable"
"#,
    )
    .expect("corp plugin policy parses");

    let merged = MergedPolicies::from_files(&user, &corp).expect("policies merge");

    assert_eq!(merged.plugins["dummy_pre"].mode, SecurityPluginMode::Rewrite);
    assert_eq!(merged.plugins["dummy_pre"].detection_level, DetectionLevel::Medium);
    assert_eq!(merged.plugins["dummy_post"].mode, SecurityPluginMode::Block);
    assert_eq!(merged.plugins["dummy_post"].detection_level, DetectionLevel::Critical);
    assert_eq!(merged.plugins["dummy_disabled"].mode, SecurityPluginMode::Disable);
    assert_eq!(merged.plugins["dummy_disabled"].active_detection_level(), None);
}

// The engine allows any event no rule matches, so an empty rule set is
// allow-everything. from_files used to substitute exactly that for a rule set
// that failed to compile, behind a warning, and compile_security_rule_set
// returned it as Ok: one broken rule in a profile disabled every other one.

#[test]
fn a_rule_set_that_does_not_compile_is_an_error_not_an_empty_allow_all() {
    let user: SettingsFile = toml::from_str(
        r#"
[profiles.rules.broken]
name = "broken"
action = "block"
match = 'http.host.matches("(unclosed")'
"#,
    )
    .unwrap();
    let error = MergedPolicies::from_files(&user, &empty_file())
        .err()
        .expect("a broken rule must not merge");
    assert!(error.contains("regex"), "{error}");
}

#[test]
fn an_active_profile_with_a_broken_rule_refuses_to_compile_its_rule_set() {
    // Deserialization does not validate, so a broken rule can reach the
    // runtime from active_profile.toml on disk.
    let active: ActiveProfileFile = toml::from_str(
        r#"
id = "code"
name = "Code"
description = "test"
revision = "r1"

[corp_rules.corp.rules.broken]
name = "broken"
action = "block"
match = 'http.host.matches("(unclosed")'
"#,
    )
    .unwrap();
    let error = active
        .compile_security_rule_set()
        .expect_err("the runtime must not receive an empty rule set for a broken profile");
    assert!(error.contains("regex"), "{error}");
}
