use super::*;

fn document(title: &str, selection: &str) -> String {
    format!(
        "title: {title}\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n{selection}\n  condition: selection\nlevel: high\n"
    )
}

#[test]
fn rejects_duplicate_sigma_selection_keys() {
    let input = document(
        "Ambiguous Rule",
        "  selection:\n    http.host: approved.example\n  selection:\n    http.host: unexpected.example",
    );
    let error =
        SecurityRuleProfile::parse_sigma_yaml(&input).expect_err("duplicate selections must not overwrite policy");
    assert!(error.contains("duplicate"), "{error}");
}

#[test]
fn sigma_scalar_and_sequence_semantics_survive_import() {
    let input = document(
        "Scalar Rule",
        "  selection:\n    tcp.port: 222\n    http.host: true\n    http.method: [GET, POST]",
    );
    let profile = SecurityRuleProfile::parse_sigma_yaml(&input).unwrap();
    let rule = &profile.profiles.rules["scalar_rule"];
    assert_eq!(rule.action, SecurityRuleAction::Allow);
    assert_eq!(rule.detection_level, Some(DetectionLevel::High));
    assert!(rule.condition.contains(r#"tcp.port == "222""#), "{}", rule.condition);
    assert!(rule.condition.contains(r#"http.host == "true""#), "{}", rule.condition);
    assert!(
        rule.condition
            .contains(r#"(http.method == "GET" || http.method == "POST")"#),
        "{}",
        rule.condition
    );
}

#[test]
fn sigma_multiple_documents_keep_distinct_rules_and_reject_duplicate_ids() {
    let first = document("First Rule", "  selection:\n    http.host: first.example");
    let second = document("Second Rule", "  selection:\n    http.host: second.example");
    let profile = SecurityRuleProfile::parse_sigma_yaml(&format!("{first}---\n{second}")).unwrap();
    assert_eq!(profile.profiles.rules.len(), 2);
    assert!(profile.profiles.rules.contains_key("first_rule"));
    assert!(profile.profiles.rules.contains_key("second_rule"));
    assert!(SecurityRuleProfile::parse_sigma_yaml(&format!("{first}---\n{first}"))
        .unwrap_err()
        .contains("duplicate Sigma-derived rule"));
}

#[test]
fn sigma_scalar_aliases_remain_usable() {
    let input = document(
        "Alias Rule",
        "  selection:\n    tcp.port: &port 222\n    http.host: *port",
    );
    let profile = SecurityRuleProfile::parse_sigma_yaml(&input).unwrap();
    assert!(profile.profiles.rules["alias_rule"]
        .condition
        .contains(r#"http.host == "222""#));
}

#[test]
fn sigma_parser_bounds_document_storms() {
    let input = (0..1100)
        .map(|i| document(&format!("Rule {i}"), "  selection:\n    http.host: example.test"))
        .collect::<Vec<_>>()
        .join("---\n");
    assert!(
        SecurityRuleProfile::parse_sigma_yaml(&input).is_err(),
        "Document storms must fail before unlimited profile allocation"
    );
}

#[test]
fn sigma_rejects_malformed_and_recursive_values() {
    for selection in [
        "  selection:\n    http.host: [unterminated",
        "  selection: &loop\n    http.host: *loop",
    ] {
        assert!(SecurityRuleProfile::parse_sigma_yaml(&document("Invalid Rule", selection)).is_err());
    }
    let nested = format!("{}example.test{}", "[".repeat(128), "]".repeat(128));
    assert!(SecurityRuleProfile::parse_sigma_yaml(&document(
        "Deep Rule",
        &format!("  selection:\n    http.host: {nested}")
    ))
    .is_err());
}
