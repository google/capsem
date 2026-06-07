use super::*;
use crate::security_engine::{ModelSecurityEvent, RuntimeSecurityEventType, SecurityEvent};

const RULE_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../sprints/security-event-rule-spine/fixtures/enforcement.toml"
));
const SIGMA_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../sprints/security-event-rule-spine/fixtures/detection.yaml"
));
const DEFAULT_PROVIDER_RULES: &str = include_str!("../default_provider_rules.toml");

#[test]
fn parses_security_event_rule_spine_fixture() {
    let profile = SecurityRuleProfile::parse_toml(RULE_FIXTURE).expect("fixture parses");
    assert_eq!(
        profile.ai.keys().cloned().collect::<Vec<_>>(),
        vec!["openai"]
    );
    assert!(profile.profiles.rules.contains_key("redact_pii"));
    assert!(profile.profiles.rules.contains_key("scan_import"));
    assert!(profile.profiles.rules.contains_key("skill_loaded"));
    assert!(profile.corp.rules.contains_key("block_openai"));

    let openai = &profile.ai["openai"].rules;
    assert_eq!(openai["http_api"].name, "openai_http_api_observed");
    assert_eq!(openai["http_api"].action, SecurityRuleAction::Allow);
    assert_eq!(
        openai["http_api"].detection_level,
        Some(DetectionLevel::Informational)
    );
    assert_eq!(
        openai["api_key_broker"].plugin.as_deref(),
        Some("credential_broker")
    );
    assert_eq!(
        openai["api_key_broker"].plugin_config["header"].as_str(),
        Some("Authorization")
    );
    assert_eq!(
        profile.profiles.rules["redact_pii"].action,
        SecurityRuleAction::Preprocess,
        "PII scanning/redaction must run before risk evaluation"
    );
}

#[test]
fn sigma_fixture_compiles_into_security_rule_profile() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(SIGMA_FIXTURE).expect("sigma fixture");
    let rule = profile
        .profiles
        .rules
        .get("openai_traffic_to_unexpected_endpoint")
        .expect("derived sigma rule key");

    assert_eq!(rule.name, "openai_traffic_to_unexpected_endpoint");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.detection_level, Some(DetectionLevel::High));
    assert_eq!(
        rule.reason.as_deref(),
        Some("OpenAI traffic must use the approved endpoint.")
    );
    assert_eq!(
        rule.condition,
        r#"model.provider == "openai" && http.host != "api.openai.com""#
    );

    let compiled = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User)
        .expect("sigma-derived rules compile");
    let rule = compiled.rules().first().expect("compiled sigma rule");
    assert_eq!(
        rule.rule_id,
        "profiles.rules.openai_traffic_to_unexpected_endpoint"
    );
}

#[test]
fn sigma_fixture_evaluates_against_security_event_roots() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(SIGMA_FIXTURE).expect("sigma fixture");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User)
        .expect("sigma-derived rules compile");

    let rogue = SecurityEvent::new(RuntimeSecurityEventType::SecurityRule)
        .with_model(ModelSecurityEvent {
            provider: Some("openai".to_string()),
            ..Default::default()
        })
        .with_http(crate::security_engine::HttpSecurityEvent {
            host: Some("proxy.internal".to_string()),
            ..Default::default()
        });
    let approved = SecurityEvent::new(RuntimeSecurityEventType::SecurityRule)
        .with_model(ModelSecurityEvent {
            provider: Some("openai".to_string()),
            ..Default::default()
        })
        .with_http(crate::security_engine::HttpSecurityEvent {
            host: Some("api.openai.com".to_string()),
            ..Default::default()
        });

    assert_eq!(rules.evaluate(&rogue).unwrap().matched_rules().len(), 1);
    assert_eq!(rules.evaluate(&approved).unwrap().matched_rules().len(), 0);
}

#[test]
fn sigma_import_rejects_stale_non_security_event_fields() {
    let err = SecurityRuleProfile::parse_sigma_yaml(
        r#"
title: Stale Callback Field
id: 22222222-2222-4222-8222-222222222222
logsource:
  product: capsem
  service: security_event
detection:
  selection:
    request.host: example.com
  condition: selection
level: high
capsem:
  action: block
"#,
    )
    .expect_err("stale callback fields must not import");

    assert!(
        err.contains("field 'request.host' is not a first-party security-event root"),
        "{err}"
    );
}

#[test]
fn compiles_fixture_with_source_priority_defaults() {
    let profile = SecurityRuleProfile::parse_toml(RULE_FIXTURE).expect("fixture parses");

    let builtin = profile
        .compile(SecurityRuleSource::BuiltinDefault)
        .expect("default rules compile");
    assert_eq!(
        builtin
            .iter()
            .find(|rule| rule.rule_key == "http_api")
            .unwrap()
            .priority,
        0
    );
    let provider_convenience = builtin
        .iter()
        .find(|rule| rule.rule_key == "http_api")
        .unwrap();
    assert_eq!(
        provider_convenience.rule_id,
        "profiles.rules.ai_openai_http_api"
    );
    assert_eq!(provider_convenience.namespace, "profiles");
    assert_eq!(provider_convenience.provider, "openai");
    assert_eq!(
        builtin
            .iter()
            .find(|rule| rule.rule_key == "block_openai")
            .unwrap()
            .priority,
        -10
    );
    let file_scan = builtin
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.scan_import")
        .expect("file scan rule compiled");
    assert_eq!(file_scan.name, "file_import_vt_scan");

    let user = profile
        .compile(SecurityRuleSource::User)
        .expect("user rules compile");
    assert_eq!(
        user.iter()
            .find(|rule| rule.rule_key == "http_api")
            .unwrap()
            .priority,
        10
    );
    assert_eq!(
        user.iter()
            .find(|rule| rule.rule_key == "block_openai")
            .unwrap()
            .priority,
        -10
    );

    let corp = profile
        .compile(SecurityRuleSource::Corp)
        .expect("corp rules compile");
    assert!(corp
        .iter()
        .all(|rule| rule.priority == -10 && rule.corp_locked));
}

#[test]
fn rule_name_is_mandatory_lowercase_and_short() {
    let missing = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.allow]
action = "allow"
detection_level = "info"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect_err("missing name rejected");
    assert!(missing.contains("missing field `name`"), "{missing}");

    let uppercase = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "OpenAI API"
action = "allow"
detection_level = "info"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect_err("uppercase/spaces rejected");
    assert!(
        uppercase.contains("rule name must use only lowercase"),
        "{uppercase}"
    );

    let long = SecurityRuleProfile::parse_toml(&format!(
        r#"
[ai.openai.rules.detect]
name = "{}"
action = "allow"
detection_level = "info"
match = 'http.host == "api.openai.com"'
"#,
        "a".repeat(65)
    ))
    .expect_err("long names rejected");
    assert!(long.contains("rule name must be at most 64"), "{long}");
}

#[test]
fn detection_level_is_optional_and_orthogonal_to_action() {
    let no_detection = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.allow]
name = "openai_allow"
action = "allow"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("rules do not need detection level");
    assert_eq!(
        no_detection.ai["openai"].rules["allow"].detection_level,
        None
    );

    let block_detection = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block]
name = "openai_block"
action = "block"
detection_level = "high"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("enforcement rules may also report detection");
    assert_eq!(
        block_detection.profiles.rules["block"].detection_level,
        Some(DetectionLevel::High)
    );

    let shorthand = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.ask]
name = "openai_ask"
action = "ask"
detection_level = "info"
match = 'model.provider == "openai"'
"#,
    )
    .expect("info alias parses");
    assert_eq!(
        shorthand.ai["openai"].rules["ask"].detection_level,
        Some(DetectionLevel::Informational)
    );
}

#[test]
fn parses_profile_scoped_rules_outside_ai_provider_blocks() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.model_pii]
name = "model_pii_preprocess"
action = "preprocess"
plugin = "pii"
match = 'has(model.request.body)'
"#,
    )
    .expect("profile-scoped rules parse");

    let compiled = profile
        .compile(SecurityRuleSource::BuiltinDefault)
        .expect("profile-scoped rules compile");
    assert_eq!(compiled.len(), 1);
    assert_eq!(compiled[0].rule_id, "profiles.rules.model_pii");
    assert_eq!(compiled[0].provider, "profiles");
    assert_eq!(compiled[0].priority, 0);

    let event =
        SecurityEvent::new(RuntimeSecurityEventType::ModelCall).with_model(ModelSecurityEvent {
            request_body: Some("hello".to_string()),
            ..Default::default()
        });
    assert!(
        compiled[0].matches_security_event(&event).unwrap(),
        "compiled rules must evaluate without reparsing their CEL string"
    );
}

#[test]
fn compiled_rule_set_evaluates_once_over_security_event() {
    let profile = SecurityRuleProfile::parse_toml(RULE_FIXTURE).expect("fixture parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault)
        .expect("rule set compiles");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(
        crate::security_engine::HttpSecurityEvent {
            host: Some("api.openai.com".to_string()),
            ..Default::default()
        },
    );

    let evaluation = rules
        .evaluate(&event)
        .expect("compiled rules evaluate against one SecurityEvent");

    assert_eq!(
        evaluation
            .detections()
            .iter()
            .map(|rule| rule.rule_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "corp.rules.block_openai",
            "profiles.rules.ai_openai_http_api",
        ]
    );
    assert_eq!(
        evaluation
            .postprocess_rules()
            .iter()
            .map(|rule| rule.plugin.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("credential_broker")]
    );
    assert_eq!(
        evaluation
            .enforcement_rules()
            .iter()
            .map(|rule| (rule.action, rule.priority))
            .collect::<Vec<_>>(),
        vec![
            (SecurityRuleAction::Block, -10),
            (SecurityRuleAction::Allow, 0),
        ]
    );
}

#[test]
fn compiled_rule_set_does_not_fan_out_cross_root_rules() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.openai_boundary]
name = "openai_boundary"
action = "allow"
detection_level = "informational"
match = 'http.host == "api.openai.com" || model.provider == "openai"'
"#,
    )
    .expect("cross-root rule parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault)
        .expect("rule set compiles");
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::ModelCall).with_model(ModelSecurityEvent {
            provider: Some("openai".to_string()),
            ..Default::default()
        });

    let evaluation = rules.evaluate(&event).expect("rule set evaluates");

    assert_eq!(evaluation.matched_rules().len(), 1);
    assert_eq!(
        evaluation.matched_rules()[0].rule_id,
        "profiles.rules.openai_boundary"
    );
}

#[test]
fn built_in_provider_defaults_use_security_rule_contract() {
    let profile = SecurityRuleProfile::parse_toml(DEFAULT_PROVIDER_RULES).expect("defaults parse");
    let openai = profile.ai.get("openai").expect("openai defaults exist");
    assert_eq!(openai.name.as_deref(), Some("OpenAI"));
    assert_eq!(openai.protocol.as_deref(), Some("openai"));
    assert!(openai
        .files
        .iter()
        .any(|path| path == "/root/.codex/config.toml"));

    let compiled = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault)
        .expect("provider defaults compile");
    assert!(compiled
        .rules()
        .iter()
        .all(|rule| rule.namespace == "profiles"));
    assert!(compiled
        .rules()
        .iter()
        .all(|rule| !rule.condition.contains("file.ingress")));
    assert!(compiled
        .rules()
        .iter()
        .all(|rule| !rule.condition.contains("credential.name")));
    assert!(compiled.rules().iter().any(|rule| {
        rule.provider == "openai"
            && rule.plugin.as_deref() == Some("credential_broker")
            && rule.action == SecurityRuleAction::Postprocess
    }));
}

#[test]
fn detect_is_not_a_rule_action_and_level_is_not_accepted() {
    let detect_action = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "openai_detect"
action = "detect"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect_err("detect is metadata, not action");
    assert!(
        detect_action.contains("unknown variant")
            || detect_action.contains("detect")
            || detect_action.contains("action"),
        "{detect_action}"
    );

    let old_level = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "openai_detect"
action = "allow"
level = "info"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect_err("old level field rejected");
    assert!(old_level.contains("detection_level"), "{old_level}");
}

#[test]
fn postprocess_and_preprocess_require_plugin() {
    let error = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.redact]
name = "openai_redact"
action = "preprocess"
match = 'has(model.request.body)'
"#,
    )
    .expect_err("preprocess requires plugin");
    assert!(error.contains("requires plugin"), "{error}");
}

#[test]
fn rewrite_is_canonical_mutation_action_with_aliases_and_requires_plugin() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.redact_model]
name = "redact_model"
action = "redact"
plugin = "dummy_pre_redact"
match = 'model.request.body.contains("secret")'

[profiles.rules.neutralize_file]
name = "neutralize_file"
action = "neutralize"
plugin = "dummy_pre_neutralize"
match = 'file.import.content.contains("bad")'

[profiles.rules.mutate_http]
name = "mutate_http"
action = "mutate"
plugin = "dummy_pre_mutate"
match = 'http.host == "example.com"'
"#,
    )
    .expect("rewrite aliases parse");

    for rule in profile.profiles.rules.values() {
        assert_eq!(rule.action, SecurityRuleAction::Rewrite);
        assert_eq!(rule.action.as_str(), "rewrite");
    }

    let compiled = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::SecurityRule)
        .with_model(ModelSecurityEvent {
            request_body: Some("secret".to_string()),
            ..Default::default()
        })
        .with_file(crate::security_engine::FileSecurityEvent {
            import_content: Some("bad".to_string()),
            ..Default::default()
        })
        .with_http(crate::security_engine::HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });
    let evaluation = compiled.evaluate(&event).unwrap();
    assert_eq!(evaluation.preprocess_rules().len(), 3);
    assert!(evaluation.enforcement_rules().is_empty());

    let err = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.rewrite_without_plugin]
name = "rewrite_without_plugin"
action = "rewrite"
match = 'http.host == "example.com"'
"#,
    )
    .expect_err("rewrite must name the mutation plugin");
    assert!(err.contains("requires plugin"), "{err}");
}

#[test]
fn rejects_old_callback_shaped_provider_authoring() {
    for (field, toml_text) in [
        (
            "on",
            r#"
[ai.openai.rules.old]
name = "old_rule"
action = "allow"
detection_level = "info"
on = "http.request"
match = 'http.host == "api.openai.com"'
"#,
        ),
        (
            "if",
            r#"
[ai.openai.rules.old]
name = "old_rule"
action = "allow"
detection_level = "info"
if = 'http.host == "api.openai.com"'
match = 'http.host == "api.openai.com"'
"#,
        ),
        (
            "decision",
            r#"
[ai.openai.rules.old]
name = "old_rule"
action = "allow"
detection_level = "info"
decision = "allow"
match = 'http.host == "api.openai.com"'
"#,
        ),
        (
            "actions",
            r#"
[ai.openai.rules.old]
name = "old_rule"
action = "allow"
detection_level = "info"
actions = ["provider.detect"]
match = 'http.host == "api.openai.com"'
"#,
        ),
    ] {
        let error = SecurityRuleProfile::parse_toml(toml_text).expect_err("old field rejected");
        assert!(error.contains(field), "expected {field} in {error}");
    }
}

#[test]
fn validates_priority_defaults_and_rejects_wrong_explicit_priority() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "openai_detect"
action = "allow"
detection_level = "info"
priority = 10
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("user-shaped explicit priority parses");
    assert!(profile.compile(SecurityRuleSource::User).is_ok());
    let default_error = profile
        .compile(SecurityRuleSource::BuiltinDefault)
        .expect_err("default source cannot use user priority");
    assert!(default_error.contains("must be 0"), "{default_error}");

    let corp_profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block]
name = "openai_block"
action = "block"
corp_locked = true
priority = -10
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("corp priority parses");
    assert!(corp_profile.compile(SecurityRuleSource::Corp).is_ok());
    let user_error = corp_profile
        .compile(SecurityRuleSource::User)
        .expect("corp locked user source defaults to corp priority");
    assert_eq!(user_error[0].priority, -10);
}

#[test]
fn priority_ranges_allow_stronger_corp_and_later_user_rules() {
    let corp_profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block]
name = "openai_block"
action = "block"
corp_locked = true
priority = -1000
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("stronger corp priority parses");
    let corp = corp_profile
        .compile(SecurityRuleSource::Corp)
        .expect("corp may use priorities below -10");
    assert_eq!(corp[0].priority, -1000);

    let user_profile = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "openai_detect"
action = "allow"
detection_level = "info"
priority = 1000
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("later user priority parses");
    let user = user_profile
        .compile(SecurityRuleSource::User)
        .expect("user may use priorities above 10");
    assert_eq!(user[0].priority, 1000);

    let negative_user = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.detect]
name = "openai_detect"
action = "allow"
detection_level = "info"
priority = -100
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("explicit negative priority parses before source validation");
    let error = negative_user
        .compile(SecurityRuleSource::User)
        .expect_err("user cannot use negative priority");
    assert!(error.contains("cannot use negative priority"), "{error}");
}

#[test]
fn corp_rules_are_locked_by_namespace_even_without_corp_locked_field() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block]
name = "corp_block"
action = "block"
match = 'http.host == "example.com"'
"#,
    )
    .expect("corp namespace parses");

    let compiled = profile
        .compile(SecurityRuleSource::User)
        .expect("corp namespace compiles as corp policy");
    assert_eq!(compiled[0].priority, -10);
    assert!(compiled[0].corp_locked);
    assert_eq!(compiled[0].namespace, "corp");
}

#[test]
fn priority_values_are_bounded_to_admin_range() {
    let too_low = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block]
name = "openai_block"
action = "block"
corp_locked = true
priority = -1001
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("priority range is checked during compilation");
    let error = too_low
        .compile(SecurityRuleSource::Corp)
        .expect_err("priority below -1000 rejected");
    assert!(error.contains("between -1000 and 1000"), "{error}");

    let too_high = SecurityRuleProfile::parse_toml(
        r#"
[ai.openai.rules.allow]
name = "openai_allow"
action = "allow"
priority = 1001
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("priority range is checked during compilation");
    let error = too_high
        .compile(SecurityRuleSource::User)
        .expect_err("priority above 1000 rejected");
    assert!(error.contains("between -1000 and 1000"), "{error}");
}

#[test]
fn plugin_policy_accepts_typed_verdicts_and_canonical_rewrite_aliases() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[plugins.dummy_pre]
mode = "rewrite"
detection_level = "medium"

[plugins.dummy_redact]
mode = "redact"

[plugins.dummy_mutate]
mode = "mutate"

[plugins.dummy_neutralize]
mode = "neutralize"

[plugins.dummy_post]
mode = "block"
detection_level = "critical"

[plugins.dummy_ask]
mode = "ask"
detection_level = "low"

[plugins.dummy_allow]
mode = "allow"

[plugins.dummy_disabled]
mode = "disable"
"#,
    )
    .expect("plugin policy parses");

    assert_eq!(
        profile.plugins["dummy_pre"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(
        profile.plugins["dummy_pre"].detection_level,
        DetectionLevel::Medium
    );
    assert_eq!(
        profile.plugins["dummy_redact"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(
        profile.plugins["dummy_mutate"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(
        profile.plugins["dummy_neutralize"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(
        profile.plugins["dummy_post"].mode,
        SecurityPluginMode::Block
    );
    assert_eq!(
        profile.plugins["dummy_post"].detection_level,
        DetectionLevel::Critical
    );
    assert_eq!(profile.plugins["dummy_ask"].mode, SecurityPluginMode::Ask);
    assert_eq!(
        profile.plugins["dummy_ask"].detection_level,
        DetectionLevel::Low
    );
    assert_eq!(
        profile.plugins["dummy_allow"].mode,
        SecurityPluginMode::Allow
    );
    assert_eq!(
        profile.plugins["dummy_allow"].detection_level,
        DetectionLevel::Informational,
        "active plugins default to informational detection level"
    );
    assert_eq!(
        profile.plugins["dummy_disabled"].mode,
        SecurityPluginMode::Disable
    );
    assert_eq!(
        profile.plugins["dummy_disabled"].active_detection_level(),
        None,
        "disabled plugins do not emit detection marks"
    );
    assert_eq!(SecurityPluginMode::Rewrite.as_str(), "rewrite");
}

#[test]
fn plugin_policy_rejects_invalid_plugin_names() {
    let error = SecurityRuleProfile::parse_toml(
        r#"
[plugins."dummy pre"]
mode = "block"
"#,
    )
    .expect_err("plugin ids are contract identifiers");

    assert!(error.contains("plugin id"), "{error}");
}
