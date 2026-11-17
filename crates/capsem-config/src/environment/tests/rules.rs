use super::*;

const RULES: &str = r#"
[access.custom.network]
name = "Approved network"
action = "allow"
match = 'http.host == "approved.example"'
priority = 10
[detection.custom.monitor]
name = "Observe host"
match = 'http.host == "approved.example"'
level = "high"
"#;

#[test]
fn environment_access_and_detection_are_distinct_typed_sections() {
    let doc = EnvironmentDocument::from_toml(&format!("{BASE}{RULES}")).unwrap();
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    let draft: EnvironmentDraft = toml::from_str(&format!("name='development'\n{RULES}")).unwrap();
    draft.validate().unwrap();
    for bad in [
        RULES.replace("action = \"allow\"", "action = \"rewrite\""),
        RULES.replace("priority = 10", "priority = -10"),
        RULES.replace("priority = 10", "priority = 10\ncorp_locked = true"),
        RULES.replace("level = \"high\"", "level = \"high\"\naction = \"allow\""),
        RULES.replace("http.host ==", "untrusted.field =="),
        RULES.replace(
            "Approved network",
            &"x".repeat(capsem_proto::ENVIRONMENT_MAX_NAME_BYTES + 1),
        ),
    ] {
        assert!(EnvironmentDocument::from_toml(&format!("{BASE}{bad}")).is_err());
    }
}

#[test]
fn environment_rule_catalog_parameters_require_selected_ids_and_typed_values() {
    let source = r#"name='development'
[[access.catalog.selected]]
id='network'
version='1.0.0'
[access.parameters.network]
enabled=true
port=443
host='approved.example'
methods=['GET','POST']
"#;
    let draft: EnvironmentDraft = toml::from_str(source).unwrap();
    draft.validate().unwrap();
    assert!(!toml::to_string(&draft).unwrap().contains("resolved_"));
    let pinned = source.replace(
        "version='1.0.0'",
        &format!(
            "version='1.0.0'\ncatalog_reference='ghcr.io/google/capsem/rules:stable'\nresolved_digest='sha256:{}'",
            "a".repeat(64)
        ),
    );
    assert!(toml::from_str::<EnvironmentDraft>(&pinned).is_err());
    let canonical = format!("{BASE}{}", pinned.strip_prefix("name='development'\n").unwrap());
    let doc = EnvironmentDocument::from_toml(&canonical).unwrap();
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    assert!(!format!("{draft:?}").contains("approved.example"));
    let bad: EnvironmentDraft = toml::from_str(&source.replace("parameters.network", "parameters.missing")).unwrap();
    assert!(bad.validate().is_err());
    let bad: EnvironmentDraft = toml::from_str(&source.replace(
        "approved.example",
        &"x".repeat(capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES + 1),
    ))
    .unwrap();
    assert!(bad.validate().is_err());
    let invalid = format!("{BASE}{RULES}").replace(
        "http.host == \"approved.example\"",
        &"x".repeat(capsem_proto::ENVIRONMENT_MAX_RULE_BYTES + 1),
    );
    assert!(EnvironmentDocument::from_toml(&invalid).is_err());
}

#[test]
fn environment_rule_counts_and_parameter_collections_enforce_exact_bounds() {
    let mut draft: EnvironmentDraft = toml::from_str(&format!("name='development'\n{RULES}")).unwrap();
    let rule = draft.access.custom.remove("network").unwrap();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_RULES {
        draft.access.custom.insert(format!("rule-{index}"), rule.clone());
    }
    draft.validate().unwrap();
    draft.access.custom.insert("too-many".into(), rule);
    assert!(draft.validate().is_err());
    draft.access.custom.clear();
    draft.access.catalog.selected.push(EnvironmentCatalogSelection {
        id: "network".into(),
        version: "1.0.0".into(),
    });
    let values = draft.access.parameters.entry("network".into()).or_default();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES {
        values.insert(format!("parameter-{index}"), EnvironmentRuleParameter::Integer(443));
    }
    draft.validate().unwrap();
    draft
        .access
        .parameters
        .get_mut("network")
        .unwrap()
        .insert("extra".into(), EnvironmentRuleParameter::Boolean(true));
    assert!(draft.validate().is_err());
    draft.access.parameters.get_mut("network").unwrap().clear();
    draft.access.parameters.get_mut("network").unwrap().insert(
        "choices".into(),
        EnvironmentRuleParameter::Choices(vec![
            "choice".into();
            capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
        ]),
    );
    draft.validate().unwrap();
    let EnvironmentRuleParameter::Choices(values) = draft
        .access
        .parameters
        .get_mut("network")
        .unwrap()
        .get_mut("choices")
        .unwrap()
    else {
        panic!("choices")
    };
    values.push("extra".into());
    assert!(draft.validate().is_err());
}
