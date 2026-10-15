use super::*;
use crate::credential_broker::CredentialProvider;

#[test]
fn header_formatter_sanitizes_and_emits_broker_observations() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        hyper::header::AUTHORIZATION,
        hyper::header::HeaderValue::from_static("Bearer sk-network-format-secret"),
    );

    let formatted = format_headers_for_domain("127.0.0.1", Some(ProviderKind::OpenAi), &headers);

    assert_eq!(formatted.observations.len(), 1);
    assert_eq!(formatted.observations[0].provider, CredentialProvider::OpenAi);
    assert_eq!(formatted.observations[0].source, "http.header.authorization");
    assert_eq!(formatted.observations[0].event_type.as_deref(), Some("http.request"));
    assert_eq!(
        formatted.credential_ref.as_deref(),
        Some(formatted.observations[0].credential_ref().as_str())
    );
    assert!(formatted.formatted.contains("authorization: hash:"));
    assert!(!formatted.formatted.contains("sk-network-format-secret"));
}

// -- Host header normalization --
//
// The Host header is guest-controlled. `parse_http_host_target` is the single
// place the plain-HTTP path derives its upstream from, so the host it returns
// must already be the normalized identity policy, dial, and telemetry share.

#[test]
fn parse_http_host_target_normalizes_case_and_trailing_dot() {
    let header = hyper::header::HeaderValue::from_static("Example.COM.");
    assert_eq!(
        parse_http_host_target(Some(&header)),
        Some(("example.com".to_string(), 80))
    );

    let with_port = hyper::header::HeaderValue::from_static("PASTEBIN.com.:8080");
    assert_eq!(
        parse_http_host_target(Some(&with_port)),
        Some(("pastebin.com".to_string(), 8080))
    );

    let ip = hyper::header::HeaderValue::from_static("127.0.0.1.:19222");
    assert_eq!(
        parse_http_host_target(Some(&ip)),
        Some(("127.0.0.1".to_string(), 19222))
    );
}

#[test]
fn parse_http_host_target_rejects_hosts_that_normalize_to_nothing() {
    for raw in [".", "..", ".:80", "..:8080", " . "] {
        let header = hyper::header::HeaderValue::from_str(raw).unwrap();
        assert_eq!(parse_http_host_target(Some(&header)), None, "{raw:?} names no host");
    }
}

/// `inet_aton(3)` accepts hex, octal, and short forms, and the host's
/// resolver hands every one of them to the same loopback the rule names.
/// The proxy must dial and judge the canonical dotted quad, not the spelling.
#[test]
fn parse_http_host_target_canonicalizes_legacy_ipv4_spellings() {
    for (raw, port) in [
        ("0x7f000001:8080", 8080),
        ("0x7F.0.0.1:19222", 19222),
        ("127.1:80", 80),
        ("127.0.1:80", 80),
        ("2130706433:443", 443),
        ("0177.0.0.1:80", 80),
        ("0x7f.1", 80),
        ("127.0.0.1", 80),
    ] {
        let header = hyper::header::HeaderValue::from_str(raw).unwrap();
        assert_eq!(
            parse_http_host_target(Some(&header)),
            Some(("127.0.0.1".to_string(), port)),
            "{raw:?} dials loopback and must be judged as loopback"
        );
    }
}

/// Only strings the resolver would read as an address are rewritten: a
/// name that merely looks numeric stays a name, and an out-of-range part is
/// not an address at all.
#[test]
fn parse_http_host_target_leaves_non_addresses_alone() {
    for raw in [
        "1e3",
        "example.com",
        "0x",
        "300.1.1.1",
        "1.2.3.4.5",
        "08.1.1.1",
        "127.0.0.1.1",
        "0x7g000001",
    ] {
        let header = hyper::header::HeaderValue::from_str(raw).unwrap();
        let parsed = parse_http_host_target(Some(&header));
        assert_eq!(
            parsed.map(|(host, _)| host).as_deref(),
            Some(raw),
            "{raw:?} is not an IPv4 spelling"
        );
    }
}

fn snapshot_for_rules_and_plugins(
    rules_toml: &str,
    plugins: std::collections::BTreeMap<String, crate::net::policy_config::SecurityPluginConfig>,
) -> crate::net::proxy_engine::ProxyPolicySnapshot {
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(rules_toml).expect("valid rules toml");
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        crate::net::policy_config::SecurityRuleSource::User,
    )
    .expect("compile rules");
    crate::net::proxy_engine::ProxyPolicySnapshot::new(
        "blake3:test".into(),
        crate::net::policy::NetworkMechanics::new(),
        rules,
        plugins,
        crate::net::policy_config::ModelEndpointRegistry::default(),
    )
}

#[test]
fn policy_requires_buffered_model_response_inspects_compiled_fields_and_actions() {
    use crate::net::policy_config::{DetectionLevel, SecurityPluginConfig, SecurityPluginMode};
    use std::collections::BTreeMap;

    // Detection-only allow rules on response fields stream without buffering.
    let allow_response_rule = r#"
[profiles.rules.detect_response_secret]
name = "detect_response_secret"
action = "allow"
detection_level = "high"
match = 'model.response.body.contains("PRIVATE KEY") || http.status == "200"'
"#;
    assert!(!policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(
        allow_response_rule,
        BTreeMap::new()
    )));

    // A blocking request-stage rule whose string literal mentions "model.response.body"
    // must not trigger response buffering.
    let request_rule_with_response_literal = r#"
[profiles.rules.block_prompt_mentioning_field]
name = "block_prompt_mentioning_field"
action = "block"
match = 'model.request.body.contains("model.response.body") && http.host == "generativelanguage.googleapis.com"'
"#;
    assert!(!policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(
        request_rule_with_response_literal,
        BTreeMap::new()
    )));

    // Disabled blocking rule on model.response.body does not require buffering.
    let disabled_block_rule = r#"
[profiles.rules.disabled_response_block]
name = "disabled_response_block"
action = "block"
enabled = false
match = 'model.response.body.contains("secret")'
"#;
    assert!(!policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(
        disabled_block_rule,
        BTreeMap::new()
    )));

    // Enabled enforcing/rewriting rules on any response-stage field require buffering.
    for (action, condition) in [
        ("block", r#"model.response.body.contains("secret")"#),
        ("ask", r#"has(model.response.valid)"#),
        ("rewrite", r#"http.body.contains("token")"#),
        ("postprocess", r#"http.status == "200""#),
    ] {
        let toml = format!(
            "[profiles.rules.enforcing_resp]\nname = \"enforcing_resp\"\naction = \"{action}\"\nmatch = '{condition}'\n"
        );
        assert!(
            policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(&toml, BTreeMap::new())),
            "action={action} condition={condition} must buffer"
        );
    }

    // Active dummy_pre_eicar plugin requires buffering; disabled does not.
    let mut plugins = BTreeMap::new();
    plugins.insert(
        "dummy_pre_eicar".into(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Disable,
            detection_level: DetectionLevel::Informational,
        },
    );
    assert!(!policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(
        "",
        plugins.clone()
    )));
    plugins.insert(
        "dummy_pre_eicar".into(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Block,
            detection_level: DetectionLevel::High,
        },
    );
    assert!(policy_requires_buffered_model_response(&snapshot_for_rules_and_plugins(
        "",
        plugins
    )));
}
