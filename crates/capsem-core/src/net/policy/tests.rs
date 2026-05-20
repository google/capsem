use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    policy: PolicyConfig,
}

fn policy_from_toml(toml_text: &str) -> PolicyConfig {
    toml::from_str::<PolicyDocument>(toml_text)
        .expect("Policy TOML should parse")
        .policy
}

#[test]
fn policy_config_module_is_not_public_or_present() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let net_mod = std::fs::read_to_string(manifest_dir.join("src/net/mod.rs"))
        .expect("net/mod.rs should be readable");

    assert!(
        !net_mod.contains("pub mod policy_config"),
        "legacy net::policy_config must not be exported"
    );
    assert!(
        !manifest_dir.join("src/net/policy_config").exists(),
        "legacy policy_config module directory must be removed"
    );
}

#[test]
fn runtime_call_sites_do_not_import_legacy_policy_config() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("capsem-core should live under crates/");
    let runtime_files = [
        "crates/capsem-core/src/net/policy_confirm.rs",
        "crates/capsem-core/src/net/dns/server.rs",
        "crates/capsem-core/src/net/mitm_proxy/mod.rs",
        "crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs",
        "crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs",
        "crates/capsem-core/src/net/mitm_proxy/policy_http_hook.rs",
        "crates/capsem-core/src/net/mitm_proxy/policy_model.rs",
        "crates/capsem-process/src/mcp_runtime.rs",
        "crates/capsem-service/src/main.rs",
        "crates/capsem/src/setup.rs",
    ];

    for file in runtime_files {
        let source = std::fs::read_to_string(repo_root.join(file))
            .unwrap_or_else(|err| panic!("failed to read {file}: {err}"));
        assert!(
            !source.contains("policy_config"),
            "{file} must not import or call legacy policy_config"
        );
    }
}

#[test]
fn runtime_call_sites_do_not_import_legacy_network_policy_runtime() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("capsem-core should live under crates/");

    for deleted in [
        "src/net/policy.rs",
        "src/net/policy_hook.rs",
        "src/net/policy_hook/tests.rs",
        "src/net/mitm_proxy/policy_hook.rs",
        "src/net/mitm_proxy/policy_hook/tests.rs",
    ] {
        assert!(
            !manifest_dir.join(deleted).exists(),
            "legacy NetworkPolicy/V1 hook file must stay deleted: {deleted}"
        );
    }

    let runtime_files = [
        "crates/capsem-core/src/lib.rs",
        "crates/capsem-core/src/net/mod.rs",
        "crates/capsem-core/src/net/dns/server.rs",
        "crates/capsem-core/src/net/mitm_proxy/mod.rs",
        "crates/capsem-core/src/vm/boot.rs",
        "crates/capsem-core/src/vm/registry.rs",
        "crates/capsem-process/src/main.rs",
        "crates/capsem-process/src/mcp_runtime.rs",
        "crates/capsem-process/src/vsock.rs",
    ];

    for file in runtime_files {
        let source = std::fs::read_to_string(repo_root.join(file))
            .unwrap_or_else(|err| panic!("failed to read {file}: {err}"));
        for forbidden in [
            "NetworkPolicy",
            "crate::net::policy::",
            "net::policy::",
            "policy_hook::PolicyHook",
            "mitm_proxy::policy_hook",
        ] {
            assert!(
                !source.contains(forbidden),
                "{file} must not import or call legacy NetworkPolicy/V1 hook token {forbidden:?}"
            );
        }
    }
}

#[test]
fn policy_parses_named_rules_with_priority_and_rewrite_captures() {
    let policy = policy_from_toml(
        r#"
[policy.http.block_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Do not let this session fetch OpenAI-owned GitHub code"

[policy.http.rewrite_openai_github_to_openclaw]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai/(?P<repo>[^/?#]+)")'
decision = "rewrite"
priority = 20
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
reason = "Route the strawman repository namespace through the allowed mirror"
"#,
    );

    let block = policy.http.get("block_openai_github").expect("block rule");
    assert_eq!(block.on, PolicyCallback::HttpRequest);
    assert_eq!(block.decision, PolicyDecisionKind::Block);
    assert_eq!(block.priority, 10);

    let rewrite = policy
        .http
        .get("rewrite_openai_github_to_openclaw")
        .expect("rewrite rule");
    assert_eq!(rewrite.on, PolicyCallback::HttpRequest);
    assert_eq!(rewrite.decision, PolicyDecisionKind::Rewrite);
    assert_eq!(
        rewrite.rewrite_value.as_deref(),
        Some("https://github.com/openclaw/${repo}${rest}")
    );

    assert_eq!(
        policy
            .rules_for_callback(PolicyCallback::HttpRequest)
            .iter()
            .map(|(name, rule)| (*name, rule.priority))
            .collect::<Vec<_>>(),
        vec![
            ("block_openai_github", 10),
            ("rewrite_openai_github_to_openclaw", 20)
        ]
    );
}

#[test]
fn policy_supports_http_read_write_callbacks() {
    let policy = policy_from_toml(
        r#"
[policy.http.default_read]
on = "http.read"
if = 'true'
decision = "ask"
priority = 900
reason = "default read gate"

[policy.http.default_write]
on = "http.write"
if = 'request.method == "POST"'
decision = "block"
priority = 901
reason = "default write gate"
"#,
    );

    let read = policy.http.get("default_read").expect("read rule");
    assert_eq!(read.on, PolicyCallback::HttpRead);
    assert_eq!(read.decision, PolicyDecisionKind::Ask);

    let write = policy.http.get("default_write").expect("write rule");
    assert_eq!(write.on, PolicyCallback::HttpWrite);
    assert_eq!(write.decision, PolicyDecisionKind::Block);

    let subject = serde_json::json!({
        "request": {
            "method": "POST"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpWrite, &subject)
        .unwrap()
        .expect("http.write rule should evaluate request fields");
    assert_eq!(hit.name, "default_write");
}

#[test]
fn policy_rejects_invalid_rule_shapes() {
    let cases = [
        (
            "warn_is_not_a_decision",
            r#"
[policy.mcp.warn_is_not_a_decision]
on = "mcp.request"
if = 'method == "tools/call"'
decision = "warn"
priority = 10
"#,
        ),
        (
            "callback_type_mismatch",
            r#"
[policy.http.mcp_callback_in_http_table]
on = "mcp.request"
if = 'method == "tools/call"'
decision = "block"
priority = 10
"#,
        ),
        (
            "missing_rewrite_value",
            r#"
[policy.http.bad]
on = "http.request"
if = 'request.host == "github.com"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)$"'
"#,
        ),
        (
            "missing_capture",
            r#"
[policy.http.bad_rewrite_capture]
on = "http.request"
if = 'request.host == "github.com"'
decision = "rewrite"
priority = 10
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)$"'
rewrite_value = "https://github.com/openclaw/${missing}"
"#,
        ),
    ];

    for (name, toml_text) in cases {
        assert!(
            toml::from_str::<PolicyDocument>(toml_text).is_err(),
            "case {name} should reject invalid Policy config"
        );
    }
}

#[test]
fn policy_evaluates_http_rules_by_priority_and_condition() {
    let policy = policy_from_toml(
        r#"
[policy.http.allow_github]
on = "http.request"
if = 'request.host == "github.com"'
decision = "allow"
priority = 20

[policy.http.block_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
"#,
    );

    let blocked = serde_json::json!({
        "request": {
            "host": "github.com",
            "path": "/openai/codex"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpRequest, &blocked)
        .unwrap()
        .expect("openai path should match block rule before broad allow");
    assert_eq!(hit.name, "block_openai_github");
    assert_eq!(hit.rule.decision, PolicyDecisionKind::Block);

    let allowed = serde_json::json!({
        "request": {
            "host": "github.com",
            "path": "/rust-lang/rust"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpRequest, &allowed)
        .unwrap()
        .expect("other github path should match broad allow");
    assert_eq!(hit.name, "allow_github");
    assert_eq!(hit.rule.decision, PolicyDecisionKind::Allow);
}

#[test]
fn policy_cel_allows_method_like_text_inside_string_literals() {
    let policy = policy_from_toml(
        r#"
[policy.http.block_literal_method_text]
on = "http.request"
if = 'request.path == "/literal.contains(value)" && request.query != "debug.matches(nope)"'
decision = "block"
priority = 10
"#,
    );

    let subject = serde_json::json!({
        "request": {
            "path": "/literal.contains(value)",
            "query": "release"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpRequest, &subject)
        .unwrap()
        .expect("quoted method-looking text should be treated as a string literal");
    assert_eq!(hit.name, "block_literal_method_text");
}

#[test]
fn policy_cel_does_not_match_not_equal_against_missing_fields() {
    let policy = policy_from_toml(
        r#"
[policy.http.block_missing_auth]
on = "http.request"
if = 'request.headers.authorization != "Bearer allowed"'
decision = "block"
priority = 10
"#,
    );

    let subject = serde_json::json!({
        "request": {
            "headers": {}
        }
    });

    assert!(
        policy
            .find_matching_rule(PolicyCallback::HttpRequest, &subject)
            .unwrap()
            .is_none(),
        "missing fields must not satisfy != comparisons by accident"
    );
}

#[test]
fn policy_evaluates_http_response_body_headers_and_request_context() {
    let policy = policy_from_toml(
        r#"
[policy.http.block_secret_json]
on = "http.response"
if = 'request.host == "api.openai.com" && response.status == "200" && response.headers.content_type.contains("application/json") && response.text.contains("response-secret")'
decision = "block"
priority = 10

[policy.http.allow_other_openai]
on = "http.response"
if = 'request.host == "api.openai.com"'
decision = "allow"
priority = 20
"#,
    );

    let blocked = serde_json::json!({
        "request": {
            "host": "api.openai.com"
        },
        "response": {
            "status": "200",
            "headers": {
                "content_type": "application/json; charset=utf-8"
            },
            "text": "{\"message\":\"response-secret\"}"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpResponse, &blocked)
        .unwrap()
        .expect("secret JSON response should match block rule");
    assert_eq!(hit.name, "block_secret_json");
    assert_eq!(hit.rule.decision, PolicyDecisionKind::Block);

    let clean = serde_json::json!({
        "request": {
            "host": "api.openai.com"
        },
        "response": {
            "status": "200",
            "headers": {
                "content_type": "application/json; charset=utf-8"
            },
            "text": "{\"message\":\"safe\"}"
        }
    });
    let hit = policy
        .find_matching_rule(PolicyCallback::HttpResponse, &clean)
        .unwrap()
        .expect("clean OpenAI response should match fallback allow");
    assert_eq!(hit.name, "allow_other_openai");
    assert_eq!(hit.rule.decision, PolicyDecisionKind::Allow);
}

#[test]
fn policy_http_response_header_rule_does_not_match_when_header_missing() {
    let policy = policy_from_toml(
        r#"
[policy.http.block_sensitive_download]
on = "http.response"
if = 'response.headers.content_disposition.contains("attachment") && response.text.contains("secret")'
decision = "block"
priority = 10
"#,
    );

    let subject = serde_json::json!({
        "response": {
            "status": "200",
            "headers": {},
            "text": "secret"
        }
    });
    assert!(
        policy
            .find_matching_rule(PolicyCallback::HttpResponse, &subject)
            .unwrap()
            .is_none(),
        "missing response headers must not satisfy string-helper CEL expressions"
    );
}
