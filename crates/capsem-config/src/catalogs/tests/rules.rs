use super::*;
use crate::{
    environment::EnvironmentRuleParameter as Value, CompiledEnvironmentDetectionRule, CompiledSecurityRule,
    SecurityRuleAction, SecurityRuleSet,
};
use std::collections::BTreeMap;

fn rule_entry() -> RuleCatalogEntry {
    toml::from_str(
        r#"
[metadata]
id = "endpoint"
version = "1.0.0"
name = "Endpoint access"
publisher = "Example"
provenance = "registry.example/rules"
license = "Apache-2.0"
status = "active"
[metadata.compatibility]
min_runtime_contract = 1
[definition]
kind = "access"
[definition.rule]
name = "Endpoint"
action = "ask"
match = 'http.host == {{host}} && http.method == {{method}} && http.valid == {{valid}} && http.status == {{status}}'
[parameters.host]
type = "text"
default = "api.example"
max_bytes = 256
[parameters.method]
type = "choice"
default = "GET"
choices = ["GET", "POST"]
[parameters.valid]
type = "boolean"
default = true
[parameters.status]
type = "integer"
default = 200
min = 100
max = 599
"#,
    )
    .unwrap()
}

fn compiled(entry: &RuleCatalogEntry, parameters: &BTreeMap<String, Value>) -> SecurityRuleSet {
    let ResolvedCatalogRule::Access(rule) = entry.resolve(parameters).unwrap() else {
        panic!("access preset")
    };
    SecurityRuleSet::new(vec![
        CompiledSecurityRule::from_environment_access("endpoint", &rule).unwrap()
    ])
}

#[test]
fn rule_catalog_parameters_apply_typed_defaults_and_overrides_in_the_real_matcher() {
    let entry = rule_entry();
    entry.validate_content().unwrap();
    let defaults = compiled(&entry, &BTreeMap::new());
    let subject = serde_json::json!({"http":{"host":"api.example","method":"GET","valid":true,"status":200}});
    assert_eq!(
        defaults.evaluate(&subject).unwrap().enforcement_rules()[0].action,
        SecurityRuleAction::Ask
    );
    let changes = [
        ("host".into(), Value::Text("other.example".into())),
        ("method".into(), Value::Text("POST".into())),
        ("valid".into(), Value::Boolean(false)),
        ("status".into(), Value::Integer(503)),
    ]
    .into();
    let updated = compiled(&entry, &changes);
    assert!(updated.evaluate(&subject).unwrap().matched_rules().is_empty());
    let subject = serde_json::json!({"http":{"host":"other.example","method":"POST","valid":false,"status":503}});
    assert_eq!(updated.evaluate(&subject).unwrap().enforcement_rules().len(), 1);
}

#[test]
fn rule_catalog_parameters_cannot_inject_conditions_or_hide_invalid_configuration() {
    let mut entry = rule_entry();
    let attack = "api.example\" || http.method != \"NEVER";
    let rules = compiled(&entry, &[("host".into(), Value::Text(attack.into()))].into());
    assert!(rules
        .evaluate(&serde_json::json!({"http":{"host":"api.example","method":"GET","valid":true,"status":200}}))
        .unwrap()
        .matched_rules()
        .is_empty());
    assert_eq!(
        rules
            .evaluate(&serde_json::json!({"http":{"host":attack,"method":"GET","valid":true,"status":200}}))
            .unwrap()
            .matched_rules()
            .len(),
        1
    );
    for (name, value) in [
        ("unknown", Value::Boolean(true)),
        ("host", Value::Integer(1)),
        ("host", Value::Text("x".repeat(257))),
        ("host", Value::Text("".into())),
        ("method", Value::Text("DELETE".into())),
        ("valid", Value::Text("true".into())),
        ("status", Value::Integer(99)),
        ("status", Value::Integer(600)),
        ("method", Value::Choices(vec!["GET".into()])),
    ] {
        assert!(entry.resolve(&[(name.into(), value)].into()).is_err());
    }
    entry
        .parameters
        .insert("unused".into(), CatalogRuleParameter::Boolean { default: false });
    assert!(entry.validate_content().is_err());
}

#[test]
fn rule_catalog_parameters_keep_detection_actionless_and_lifecycle_explicit() {
    let mut entry = rule_entry();
    let rule: crate::environment::EnvironmentDetectionRule =
        toml::from_str("name='Detection'\nlevel='high'\nmatch='http.host == {{host}}'").unwrap();
    entry.definition = Some(RuleCatalogDefinition::Detection { rule });
    entry.parameters.retain(|key, _| key == "host");
    entry.validate_content().unwrap();
    let ResolvedCatalogRule::Detection(rule) = entry.resolve(&BTreeMap::new()).unwrap() else {
        panic!("detection preset")
    };
    let compiled = CompiledEnvironmentDetectionRule::new("observe", &rule).unwrap();
    assert_eq!(
        compiled
            .evaluate(&serde_json::json!({"http":{"host":"api.example"}}))
            .unwrap(),
        Some(crate::DetectionLevel::High)
    );
    entry.metadata.compatibility.min_runtime_contract = CATALOG_RUNTIME_CONTRACT + 1;
    entry.validate_content().unwrap();
    assert!(entry.resolve(&BTreeMap::new()).is_err());
    entry.metadata.compatibility.min_runtime_contract = CATALOG_RUNTIME_CONTRACT;
    entry.metadata.status = CatalogEntryStatus::Revoked;
    entry.metadata.status_reason = Some("Withdrawn".into());
    entry.definition = None;
    entry.parameters.clear();
    entry.validate_content().unwrap();
    assert!(entry.resolve(&BTreeMap::new()).is_err());
    entry.metadata.status = CatalogEntryStatus::Active;
    entry.metadata.status_reason = None;
    assert!(entry.validate_content().is_err());
}

#[test]
fn rule_catalog_parameters_reject_bad_descriptors_templates_and_expansion_overflow() {
    for parameter in [
        CatalogRuleParameter::Integer {
            default: 5,
            min: 6,
            max: 9,
        },
        CatalogRuleParameter::Integer {
            default: 5,
            min: 9,
            max: 1,
        },
        CatalogRuleParameter::Text {
            default: "a".into(),
            max_bytes: 0,
            allow_empty: false,
        },
        CatalogRuleParameter::Choice {
            default: "missing".into(),
            choices: vec!["a".into()],
        },
        CatalogRuleParameter::Choice {
            default: "a".into(),
            choices: vec!["a".into(), "a".into()],
        },
    ] {
        let mut entry = rule_entry();
        entry.parameters.insert("host".into(), parameter);
        assert!(entry.validate_content().is_err());
    }
    for condition in [
        "http.host == {{missing}}",
        "http.host == {{host",
        "http.unknown == {{host}}",
    ] {
        let mut entry = rule_entry();
        let Some(RuleCatalogDefinition::Access { rule }) = &mut entry.definition else {
            panic!()
        };
        rule.condition = condition.into();
        assert!(entry.validate_content().is_err());
    }
    let mut entry = rule_entry();
    entry.parameters.retain(|key, _| key == "host");
    entry.parameters.insert(
        "host".into(),
        CatalogRuleParameter::Text {
            default: "a".repeat(16384),
            max_bytes: 16384,
            allow_empty: false,
        },
    );
    let Some(RuleCatalogDefinition::Access { rule }) = &mut entry.definition else {
        panic!()
    };
    rule.condition = std::iter::repeat_n("http.host == {{host}}", 5)
        .collect::<Vec<_>>()
        .join(" || ");
    assert!(entry.validate_content().is_err());
    let value = serde_json::to_value(rule_entry()).unwrap();
    for value in [
        {
            let mut value = value.clone();
            value["parameters"]["valid"]["unknown"] = serde_json::json!(true);
            value
        },
        {
            let mut value = value;
            value["definition"]["rule"]["corp_locked"] = serde_json::json!(true);
            value
        },
    ] {
        assert!(serde_json::from_value::<RuleCatalogEntry>(value).is_err());
    }
}

#[test]
fn rule_catalog_parameters_reject_quoted_placeholders_and_bound_descriptor_count() {
    for template in [
        "http.host == '{{host}}'",
        "http.host == \"{{host}}\"",
        "http.host.matches('{{host}}')",
    ] {
        let mut entry = rule_entry();
        entry.parameters.retain(|key, _| key == "host");
        let Some(RuleCatalogDefinition::Access { rule }) = &mut entry.definition else {
            panic!()
        };
        rule.condition = template.into();
        assert!(entry.validate_content().is_err(), "quoted placeholder must be refused");
    }
    for (count, valid) in [
        (capsem_proto::CATALOG_MAX_RULE_PARAMETERS, true),
        (capsem_proto::CATALOG_MAX_RULE_PARAMETERS + 1, false),
    ] {
        let mut entry = rule_entry();
        entry.parameters = (0..count)
            .map(|n| (format!("p{n}"), CatalogRuleParameter::Boolean { default: true }))
            .collect();
        let Some(RuleCatalogDefinition::Access { rule }) = &mut entry.definition else {
            panic!()
        };
        rule.condition = (0..count)
            .map(|n| format!("http.valid == {{{{p{n}}}}}"))
            .collect::<Vec<_>>()
            .join(" && ");
        assert_eq!(entry.validate_content().is_ok(), valid);
    }
}
