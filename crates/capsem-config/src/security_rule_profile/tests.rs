use super::SecurityRuleProfile;

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
