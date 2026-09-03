use super::*;

fn pii_rule_matches(field: &str, event: SecurityEvent) -> bool {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.pii_guard]
name = "pii_guard"
action = "block"
priority = 10
match = '{field}.contains_pii()'
"#
    ))
    .expect("contains_pii is a supported CEL term");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("compiles");
    !rules.evaluate(&event).expect("evaluates").matched_rules().is_empty()
}

fn export_event(content: Option<&str>) -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::FileExport).with_file(FileSecurityEvent {
        export_path: Some("/workspace/out.txt".to_string()),
        export_content: content.map(str::to_string),
        ..Default::default()
    })
}

#[test]
fn contains_pii_matches_addresses_and_social_security_numbers() {
    for content in [
        "mail me at person@example.com",
        "ssn 123-45-6789 on file",
        "@",
        "111-22-3333",
    ] {
        assert!(
            pii_rule_matches("file.export.content", export_event(Some(content))),
            "contains_pii() must match {content:?}"
        );
    }
}

#[test]
fn contains_pii_does_not_match_plain_text_or_absent_fields() {
    for content in ["nothing sensitive here", "1234-56-7890", "12-34-5678", "123-45-678", ""] {
        assert!(
            !pii_rule_matches("file.export.content", export_event(Some(content))),
            "contains_pii() must not match {content:?}"
        );
    }

    assert!(
        !pii_rule_matches("file.export.content", export_event(None)),
        "an absent field carries no PII, so the rule must not fire"
    );
    assert!(
        !pii_rule_matches("http.body", export_event(Some("person@example.com"))),
        "contains_pii() reads only the field it names, not the whole event"
    );
}

#[test]
fn contains_pii_rejects_arguments() {
    let error = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.pii_guard]
name = "pii_guard"
action = "block"
priority = 10
match = 'file.export.content.contains_pii("email")'
"#,
    )
    .expect_err("contains_pii takes no arguments");

    assert!(error.contains("contains_pii() does not accept arguments"), "{error}");
}

fn http_event(method: Option<&str>) -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("example.test".to_string()),
        method: method.map(str::to_string),
        ..Default::default()
    })
}

fn dns_event() -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::DnsQuery).with_dns(DnsSecurityEvent {
        qname: Some("example.test".to_string()),
        qtype: Some("1".to_string()),
    })
}

fn condition_matches(condition: &str, event: &SecurityEvent) -> bool {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.probe]
name = "probe"
action = "block"
priority = 10
match = '{condition}'
"#
    ))
    .unwrap_or_else(|error| panic!("condition {condition:?} must compile: {error}"));
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("compiles");
    !rules.evaluate(event).expect("evaluates").matched_rules().is_empty()
}

/// An absent field makes *every* atom false, including a negated comparison.
///
/// This is deliberate and load-bearing: it is the only thing that scopes a rule
/// to one event family, so `file.write.path != "x"` must not fire on an HTTP
/// event. The cost is that a negation is a filter over data that is present, not
/// a deny-by-default: see `negation_is_not_deny_by_default` for the pattern that
/// actually denies.
#[test]
fn an_absent_field_makes_every_atom_kind_false() {
    let present = http_event(Some("POST"));
    let absent = http_event(None);

    // (condition, matches when method="POST", matches when method is absent)
    let matrix = [
        ("has(http.method)", true, false),
        (r#"http.method == "POST""#, true, false),
        (r#"http.method == "GET""#, false, false),
        // The fail-open direction: a negation cannot fire on data it never saw.
        (r#"http.method != "GET""#, true, false),
        (r#"http.method != "POST""#, false, false),
        (r#"http.method.contains("OS")"#, true, false),
        (r#"http.method.startsWith("PO")"#, true, false),
        (r#"http.method.endsWith("ST")"#, true, false),
        (r#"http.method.matches("^P.ST$")"#, true, false),
        ("http.method.contains_pii()", false, false),
    ];

    for (condition, when_present, when_absent) in matrix {
        assert_eq!(
            condition_matches(condition, &present),
            when_present,
            "{condition:?} against method=POST"
        );
        assert_eq!(
            condition_matches(condition, &absent),
            when_absent,
            "{condition:?} against an absent method"
        );
    }
}

/// The same rule must not leak onto another family, which is what absent-is-false
/// buys us. A DNS event carries no `http.*` field, so no `http` atom can fire.
#[test]
fn absent_field_semantics_scope_a_rule_to_its_own_family() {
    let dns = dns_event();

    for condition in [
        "has(http.method)",
        r#"http.method == "GET""#,
        r#"http.method != "GET""#,
        r#"http.host != "example.test""#,
        r#"http.method.contains("GET")"#,
        "http.method.contains_pii()",
    ] {
        assert!(
            !condition_matches(condition, &dns),
            "{condition:?} must not fire on a DNS event"
        );
    }

    // And the DNS family still evaluates normally on the same event.
    assert!(condition_matches(r#"dns.qname == "example.test""#, &dns));
    assert!(condition_matches(r#"dns.qname != "other.test""#, &dns));
}

/// A negation cannot express deny-by-default, because it goes quiet exactly when
/// the fact it judges is missing. Default-deny belongs in the priority ladder: a
/// low-precedence `block` catch-all with higher-precedence `allow` exceptions.
#[test]
fn negation_is_not_deny_by_default() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.deny_by_negation]
name = "deny_by_negation"
action = "block"
priority = 10
match = 'http.host != "allowed.test"'
"#,
    )
    .expect("parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("compiles");

    let known_bad = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("evil.test".to_string()),
        ..Default::default()
    });
    let host_missing =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent::default());

    assert!(
        !rules.evaluate(&known_bad).unwrap().enforcement_rules().is_empty(),
        "the negation does block a host it can see"
    );
    assert!(
        rules.evaluate(&host_missing).unwrap().enforcement_rules().is_empty(),
        "and goes quiet when the host is missing -- which is why default-deny \
         must be a block catch-all, not a negation"
    );
}

/// The pattern that does deny by default: a block catch-all at weak precedence,
/// beaten by an allow at stronger precedence. It holds whether or not the field
/// the exception reads is present.
#[test]
fn block_catchall_with_an_allow_exception_denies_by_default() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.allow_known_host]
name = "allow_known_host"
action = "allow"
priority = 10
match = 'http.host == "allowed.test"'

[profiles.rules.deny_the_rest]
name = "deny_the_rest"
action = "block"
priority = 900
match = 'has(http.valid)'
"#,
    )
    .expect("parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("compiles");

    let cases = [
        (Some("allowed.test"), SecurityRuleAction::Allow),
        (Some("evil.test"), SecurityRuleAction::Block),
        // The case the negation missed: nothing to judge still denies.
        (None, SecurityRuleAction::Block),
    ];

    for (host, expected) in cases {
        let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: host.map(str::to_string),
            ..Default::default()
        });
        let evaluation = rules.evaluate(&event).expect("evaluates");
        let selected = evaluation.enforcement_rules();
        assert_eq!(
            selected.first().map(|rule| rule.action),
            Some(expected),
            "host {host:?} must resolve to {expected:?}"
        );
    }
}
