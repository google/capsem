//! The built-in HTTP tools judge the addresses a host resolves to, refuse
//! non-public addresses no rule allowed, and pin the connection to the
//! addresses they judged. Before this the rules saw only the name: a name
//! that resolved to 127.0.0.1 or 169.254.169.254 reached it, and a name
//! whose answer changed between check and dial reached anything.

use super::*;
use std::net::SocketAddr;

fn rules(toml: &str) -> SecurityRuleSet {
    crate::net::policy_config::SecurityRuleProfile::parse_toml(toml)
        .and_then(|profile| {
            SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User)
        })
        .expect("test security rules compile")
}

#[tokio::test]
async fn a_name_that_resolves_to_loopback_needs_an_explicit_allow_rule() {
    let no_matching_rule = rules(
        r#"
        [profiles.rules.block_elsewhere]
        name = "block_elsewhere"
        action = "block"
        reason = "unrelated"
        match = 'http.host == "elsewhere.example"'
        "#,
    );
    for url in [
        "http://localhost:1/",
        "http://127.0.0.1:1/",
        "http://[::1]:1/",
        "http://0x7f000001:1/",
    ] {
        let refused = authorize_upstream(url, "GET", &no_matching_rule, &BTreeMap::new())
            .await
            .expect_err(&format!("{url} reaches a non-public address without an allow rule"));
        assert!(refused.contains("non-public"), "{url}: {refused}");
    }
}

#[tokio::test]
async fn an_ip_rule_blocks_a_name_that_resolves_to_it() {
    // The host rule allows the name; the address rule blocks what it
    // resolves to. The address is what the socket reaches, so it wins.
    let allow_name_block_address = rules(
        r#"
        [profiles.rules.a_block_loopback]
        name = "a_block_loopback"
        action = "block"
        reason = "no loopback"
        match = 'ip.value == "127.0.0.1" || ip.value == "::1"'

        [profiles.rules.z_allow_localhost]
        name = "z_allow_localhost"
        action = "allow"
        reason = "test"
        match = 'http.host == "localhost"'
        "#,
    );
    let refused = authorize_upstream(
        "http://localhost:1/",
        "GET",
        &allow_name_block_address,
        &BTreeMap::new(),
    )
    .await
    .expect_err("the resolved address is judged");
    assert!(refused.contains("blocked"), "{refused}");
}

#[tokio::test]
async fn an_explicit_allow_rule_reaches_loopback_through_the_tool() {
    let fixture = spawn_builtin_http_fixture().await;
    let port = fixture.base_url.rsplit(':').next().unwrap().parse::<u16>().unwrap();
    let allow_localhost = rules(
        r#"
        [profiles.rules.allow_localhost]
        name = "allow_localhost"
        action = "allow"
        reason = "local fixture"
        match = 'http.host == "localhost"'
        "#,
    );
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({ "url": format!("http://localhost:{port}/about") }),
        &test_client(),
        &allow_localhost,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &mut Vec::new(),
    )
    .await;
    assert!(!is_tool_error(&resp), "{resp:?}");
    assert!(extract_tool_text(&resp).contains("Elie"), "{resp:?}");
}

#[tokio::test]
async fn the_pinned_client_connects_only_to_the_judged_address() {
    // `pinned.invalid` never resolves. Pinning it to the fixture's address
    // is the only way this request can succeed, so success proves the
    // connection went where the boundary looked and nowhere else.
    let fixture = spawn_builtin_http_fixture().await;
    let port = fixture.base_url.rsplit(':').next().unwrap().parse::<u16>().unwrap();
    let client = test_client()
        .pinned("pinned.invalid", &[SocketAddr::from(([127, 0, 0, 1], port))])
        .unwrap();
    let response = client
        .get(format!("http://pinned.invalid:{port}/about"))
        .send()
        .await
        .expect("pinned connection reaches the fixture");
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("Elie"), "{text}");
}

#[tokio::test]
async fn refusals_are_recorded_as_denied_requests() {
    let mut ledger = Vec::new();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({ "url": "http://localhost:1/" }),
        &test_client(),
        &default_dev_security_rules(),
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &mut ledger,
    )
    .await;
    assert!(is_tool_error(&resp), "{resp:?}");
    let [BuiltinLedgerRecord::HttpRequest(request)] = ledger.as_slice() else {
        panic!("a refusal owes exactly one request record: {ledger:?}");
    };
    assert_eq!(request.decision, HttpDecision::Denied);
    assert_eq!(request.domain, "localhost");
    assert_eq!(request.policy_action, "block");
}
