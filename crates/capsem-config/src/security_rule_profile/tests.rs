use serde_json::json;

use super::*;

mod yaml_migration;

#[test]
fn native_approval_user_fixture_compiles_with_the_real_priority_boundary() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/config/security-rule-profile/native-approval.toml"
    ));
    let profile = SecurityRuleProfile::parse_toml(fixture).expect("native fixture parses");
    let compiled = profile
        .compile(SecurityRuleSource::User)
        .expect("native user rules admitted");
    assert_eq!(compiled.len(), 3);
    assert!(compiled.iter().all(|rule| rule.priority == USER_PRIORITY_MIN));
    assert!(CORP_PRIORITY_MAX < compiled[0].priority && compiled[0].priority < DEFAULT_RULE_PRIORITY);
    for invalid in [-100, 0, USER_PRIORITY_MIN - 1] {
        let mut rejected = profile.clone();
        rejected
            .profiles
            .rules
            .get_mut("ask_native_file_import")
            .unwrap()
            .priority = Some(SecurityRulePriority::Explicit(invalid));
        assert!(rejected.compile(SecurityRuleSource::User).is_err());
    }
}

#[test]
fn sigma_sequence_alternatives_still_require_every_selection_field() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(
        "title: Grouped Rule\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n  selection:\n    http.host: approved.example\n    http.method: [GET, POST]\n  condition: selection\nlevel: high\n",
    ).unwrap();
    let rule = &profile.profiles.rules["grouped_rule"];
    for method in ["GET", "POST"] {
        assert!(evaluate_security_event_match(
            &rule.condition,
            &json!({"http": {"host": "approved.example", "method": method}})
        )
        .unwrap());
        assert!(
            !evaluate_security_event_match(
                &rule.condition,
                &json!({"http": {"host": "unexpected.example", "method": method}})
            )
            .unwrap(),
            "{method} cannot bypass the required host"
        );
    }
}

#[test]
fn sigma_negated_selection_alternatives_still_require_the_guard() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(
        "title: Guarded Rule\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n  guard:\n    tcp.port: 443\n  selection:\n    http.host: approved.example\n    http.method: [GET, POST]\n  condition: guard and not selection\nlevel: high\n",
    ).unwrap();
    let rule = &profile.profiles.rules["guarded_rule"];
    for (port, host, method, expected) in [
        ("443", "approved.example", "GET", false),
        ("443", "approved.example", "POST", false),
        ("443", "approved.example", "DELETE", true),
        ("443", "unexpected.example", "GET", true),
        ("80", "approved.example", "DELETE", false),
        ("80", "unexpected.example", "GET", false),
    ] {
        assert_eq!(
            evaluate_security_event_match(
                &rule.condition,
                &json!({"tcp": {"port": port}, "http": {"host": host, "method": method}})
            )
            .unwrap(),
            expected,
            "{port} {host} {method}"
        );
    }
}

const SIGMA_HEADER: &str = r#"
logsource:
  product: capsem
  service: security_event
"#;

#[test]
fn sigma_yaml_preserves_multiple_documents_and_scalar_sequences() {
    let input = format!(
        r#"
title: Rate limited traffic
{SIGMA_HEADER}
detection:
  selection:
    http.status:
      - 429
      - 503
    http.valid: true
  condition: selection
level: high
---
title: OpenAI traffic
{SIGMA_HEADER}
detection:
  selection:
    model.provider: openai
  condition: selection
level: medium
"#
    );

    let profile = SecurityRuleProfile::parse_sigma_yaml(&input).unwrap();
    assert_eq!(profile.profiles.rules.len(), 2);
    let rate_limited = &profile.profiles.rules["rate_limited_traffic"];
    assert!(rate_limited.condition.contains(r#"http.status == "429""#));
    assert!(rate_limited.condition.contains(r#"http.status == "503""#));
    assert!(rate_limited.condition.contains(r#"http.valid == "true""#));
    assert_eq!(
        profile.profiles.rules["openai_traffic"].condition,
        r#"model.provider == "openai""#
    );
}

#[test]
fn sigma_yaml_rejects_duplicate_detection_keys() {
    let input = format!(
        r#"
title: Duplicate selection
{SIGMA_HEADER}
detection:
  selection:
    model.provider: openai
  selection:
    model.provider: anthropic
  condition: selection
level: high
"#
    );

    assert!(SecurityRuleProfile::parse_sigma_yaml(&input).is_err());
}
#[test]
fn content_plugin_options_are_typed_and_reject_unknown_entities() {
    let config: super::SecurityPluginConfig = toml::from_str(
        "mode = 'rewrite'\n[content]\ndirection = 'outbound'\nemail = true\nphone = false\ncard = true\nssn = false\n",
    )
    .expect("content filter configuration is supported");
    let encoded = toml::to_string(&config).unwrap();
    assert!(encoded.contains("outbound"));
    assert!(encoded.contains("phone = false"));
    assert!(toml::from_str::<super::SecurityPluginConfig>("mode='block'\n[content]\npassport=true").is_err());
}
