//! IP-literal URLs must produce the same `ip.*` event fields whatever their
//! spelling. A bracketed IPv6 literal (`http://[::1]/`) used to reach the
//! rules as `http.host == "[::1]"` with no `ip` event at all, so every
//! `ip.version` / `ip.value` rule that gates IPv4 loopback was blind to it.

use super::*;

fn rules(toml: &str) -> SecurityRuleSet {
    crate::net::policy_config::SecurityRuleProfile::parse_toml(toml)
        .and_then(|profile| {
            SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User)
        })
        .expect("test security rules compile")
}

fn block_ipv6() -> SecurityRuleSet {
    rules(
        r#"
        [profiles.rules.block_v6]
        name = "block_v6"
        action = "block"
        reason = "no v6 literals"
        match = 'ip.version == "6"'
        "#,
    )
}

fn block_value(value: &str) -> SecurityRuleSet {
    rules(&format!(
        r#"
        [profiles.rules.block_value]
        name = "block_value"
        action = "block"
        reason = "blocked literal"
        match = 'ip.value == "{value}"'
        "#
    ))
}

#[test]
fn bracketed_ipv6_literal_carries_an_ip_event() {
    for url in [
        "http://[::1]:8080/admin",
        "http://[::1]/",
        "https://[2001:db8::1]/x",
        "http://[::ffff:127.0.0.1]/",
        "http://[::FFFF:7F00:1]/",
    ] {
        let result = evaluate_builtin_http_request(url, "GET", &block_ipv6(), &BTreeMap::new());
        let err = result.expect_err(&format!("{url} must be blocked by the ip.version rule"));
        assert!(err.contains("blocked"), "{url}: {err}");
    }
}

#[test]
fn ip_literal_host_and_value_are_unbracketed_and_lowercase() {
    let allow_all = rules(
        r#"
        [profiles.rules.allow_all]
        name = "allow_all"
        action = "allow"
        reason = "test"
        match = 'http.host != ""'
        "#,
    );
    let checked =
        evaluate_builtin_http_request("http://[::1]:8080/", "GET", &allow_all, &BTreeMap::new()).expect("allowed");
    assert_eq!(
        checked.domain, "::1",
        "telemetry domain must not carry the URL brackets"
    );
    let checked =
        evaluate_builtin_http_request("http://[2001:DB8::1]/", "GET", &allow_all, &BTreeMap::new()).expect("allowed");
    assert_eq!(checked.domain, "2001:db8::1");
    let checked =
        evaluate_builtin_http_request("http://EXAMPLE.COM./", "GET", &allow_all, &BTreeMap::new()).expect("allowed");
    assert_eq!(checked.domain, "example.com");
    assert_eq!(extract_domain("http://[::1]:9/"), "::1");
    assert_eq!(extract_domain("http://Example.COM/"), "example.com");
}

#[test]
fn ip_value_rules_match_every_spelling_of_loopback() {
    for (url, value) in [
        ("http://127.0.0.1/", "127.0.0.1"),
        ("http://[::1]/", "::1"),
        ("http://[0:0:0:0:0:0:0:1]/", "::1"),
        ("http://[::ffff:127.0.0.1]/", "::ffff:127.0.0.1"),
    ] {
        let result = evaluate_builtin_http_request(url, "GET", &block_value(value), &BTreeMap::new());
        assert!(result.is_err(), "{url} must match ip.value == {value:?}");
    }
}

#[test]
fn legacy_ipv4_spellings_are_judged_as_the_dotted_quad() {
    for url in [
        "http://0x7f000001/",
        "http://127.1/",
        "http://2130706433/",
        "http://0177.0.0.1/",
        "http://0x7f.0.0.1:8080/",
    ] {
        let result = evaluate_builtin_http_request(url, "GET", &block_value("127.0.0.1"), &BTreeMap::new());
        assert!(
            result.is_err(),
            "{url} dials loopback and must match ip.value == \"127.0.0.1\""
        );
        assert_eq!(extract_domain(url), "127.0.0.1", "{url}");
    }
}
