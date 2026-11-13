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

/// A tool permission saved from Settings names the tool the way the settings
/// page does (`fetch_http`), while a call carries the namespaced name
/// (`local__fetch_http`), and it used to share the built-in default's priority,
/// where `default_mcp` sorts first. Either alone left a tool the user had
/// blocked fully callable while the settings page reported it blocked. The
/// rule as old settings files stored it must enforce.
#[test]
fn a_managed_tool_permission_enforces_over_the_default_mcp_rule() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"[default.mcp]
name = "mcp"
action = "allow"
priority = "default"
match = 'has(mcp.method) || has(mcp.tool_call.name)'

[profiles.rules.mcp_local_fetch_http_permission]
name = "mcp_local_fetch_http_permission"
action = "block"
priority = "default"
match = 'mcp.server.name == "local" && mcp.tool_call.name == "fetch_http"'

[profiles.rules.mcp_local_fetch_http_permission.managed]
kind = "mcp_tool"
server = "local"
tool = "fetch_http"
operation = "permission"
"#,
    )
    .expect("stored managed rule parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("rules compile");
    let call =
        |tool: &str| json!({"mcp": {"method": "tools/call", "server": {"name": "local"}, "tool_call": {"name": tool}}});

    let blocked = call("local__fetch_http");
    let evaluation = rules.evaluate(&blocked).expect("call evaluates");
    let enforced = evaluation.enforcement_rules()[0];
    assert_eq!(enforced.rule_id, "profiles.rules.mcp_local_fetch_http_permission");
    assert_eq!(enforced.action, SecurityRuleAction::Block);
    assert_eq!(enforced.priority, MANAGED_PERMISSION_PRIORITY);

    let other = call("local__echo");
    let evaluation = rules.evaluate(&other).expect("call evaluates");
    assert_eq!(evaluation.enforcement_rules()[0].rule_id, "profiles.rules.default_mcp");
}

/// A managed rule the user ranked ahead of their other rules keeps that rank,
/// and corp's managed rules stay in the corp band.
#[test]
fn a_managed_tool_permission_never_loses_rank_it_was_given() {
    let managed = SecurityRuleManagedTarget::McpTool {
        server: "wiki".to_string(),
        tool: "search".to_string(),
        operation: SecurityRuleManagedOperation::Permission,
    };
    assert_eq!(
        managed.enforced_condition().as_deref(),
        Some(r#"mcp.server.name == "wiki" && mcp.tool_call.name == "wiki__search""#)
    );
    let mut profile = SecurityRuleProfile::default();
    profile.profiles.rules.insert(
        "mcp_wiki_search_permission".to_string(),
        SecurityRule {
            name: "mcp_wiki_search_permission".to_string(),
            action: SecurityRuleAction::Ask,
            condition: "has(mcp.method)".to_string(),
            enabled: true,
            detection_level: None,
            priority: Some(SecurityRulePriority::Explicit(USER_PRIORITY_MIN)),
            corp_locked: false,
            reason: None,
            managed: Some(managed),
            plugin_config: BTreeMap::new(),
        },
    );
    let user = profile.compile(SecurityRuleSource::User).expect("user rule compiles");
    assert_eq!(user[0].priority, USER_PRIORITY_MIN);
    profile
        .profiles
        .rules
        .get_mut("mcp_wiki_search_permission")
        .unwrap()
        .priority = None;
    let corp = profile.compile(SecurityRuleSource::Corp).expect("corp rule compiles");
    assert_eq!(corp[0].priority, CORP_PRIORITY_MAX);
    assert!(corp[0].condition.contains("wiki__search"));
}
