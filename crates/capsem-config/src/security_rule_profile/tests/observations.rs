use super::*;

fn observation(id: &str, enabled: bool) -> CompiledEnvironmentDetectionRule {
    let rule = crate::environment::EnvironmentDetectionRule {
        name: "Observed access".into(),
        condition: "http.host == \"observed.example\"".into(),
        level: DetectionLevel::High,
        enabled,
    };
    CompiledEnvironmentDetectionRule::new(id, &rule).unwrap()
}

#[test]
fn environment_observations_remain_actionless_in_the_shared_rule_set() {
    let profile = SecurityRuleProfile::parse_toml(
        "[corp.rules.deny]\nname='mandatory_denial'\naction='block'\nmatch='http.host == \"observed.example\"'",
    )
    .unwrap();
    let rules = SecurityRuleSet::with_observations(
        profile.compile(SecurityRuleSource::Corp).unwrap(),
        vec![observation("observed", true), observation("disabled", false)],
    );
    let subject = json!({"http": {"host": "observed.example"}});
    for rules in [&rules, &rules.clone()] {
        let evaluated = rules.evaluate(&subject).unwrap();
        assert_eq!(evaluated.observations().len(), 1);
        assert_eq!(evaluated.observations()[0].rule_id, "environment.detection.observed");
        assert_eq!(evaluated.observations()[0].level, DetectionLevel::High);
        assert_eq!(evaluated.enforcement_rules().len(), 1);
        assert_eq!(evaluated.enforcement_rules()[0].action, SecurityRuleAction::Block);
        assert!(evaluated.preprocess_rules().is_empty());
        assert!(evaluated.postprocess_rules().is_empty());
        assert!(evaluated.rules_for_action(SecurityRuleAction::Allow).is_empty());
        assert!(evaluated.detections().is_empty());
        let missed = rules.evaluate(&json!({"http": {"host": "elsewhere.example"}})).unwrap();
        assert!(missed.observations().is_empty());
        assert!(missed.matched_rules().is_empty());
    }
}

#[test]
fn environment_observations_without_access_rules_cannot_create_a_decision() {
    let rules = SecurityRuleSet::with_observations(Vec::new(), vec![observation("only", true)]);
    let result = rules.evaluate(&json!({"http": {"host": "observed.example"}})).unwrap();
    assert_eq!(rules.observations().len(), 1);
    assert_eq!(result.observations().len(), 1);
    assert!(result.matched_rules().is_empty());
    assert!(result.enforcement_rules().is_empty());
    assert!(SecurityRuleSet::new(Vec::new()).observations().is_empty());
}
