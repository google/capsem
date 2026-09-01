use super::*;
use crate::net::policy_config::{SecurityRuleAction, SecurityRuleProfile, SecurityRuleSet};

#[test]
fn collected_gzip_chunked_response_headers_are_materialized() {
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::CONTENT_ENCODING, http::HeaderValue::from_static("gzip"));
    headers.insert(
        http::header::TRANSFER_ENCODING,
        http::HeaderValue::from_static("chunked"),
    );
    headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("9999"));

    materialize_collected_response_headers(&mut headers, 1234, true);

    assert!(!headers.contains_key(http::header::CONTENT_ENCODING));
    assert!(!headers.contains_key(http::header::TRANSFER_ENCODING));
    assert_eq!(
        headers.get(http::header::CONTENT_LENGTH),
        Some(&http::HeaderValue::from_static("1234"))
    );
}

#[test]
fn provider_detection_marks_undeclared_model_path_as_unknown_provider() {
    let registry = crate::net::policy_config::ModelEndpointRegistry::default();

    assert_eq!(
        ai_identity_for_target_or_path(
            &registry,
            "rogue-openai-compatible.example",
            443,
            "/v1/chat/completions"
        ),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Unknown),
            protocol: Some(ModelProtocol::OpenAi),
        }
    );
    assert_eq!(
        ai_identity_for_target_or_path(&registry, "unknown.example", 443, "/v1/messages"),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Unknown),
            protocol: Some(ModelProtocol::Anthropic),
        }
    );
    assert_eq!(
        ai_identity_for_target_or_path(
            &registry,
            "unknown.example",
            443,
            "/v1beta/models/gemini-2.5-pro:generateContent"
        ),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Unknown),
            protocol: Some(ModelProtocol::Google),
        }
    );
    assert_eq!(
        ai_identity_for_target_or_path(&registry, "unknown.example", 443, "/api/chat"),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Unknown),
            protocol: Some(ModelProtocol::Ollama),
        }
    );
}

#[test]
fn provider_identity_keeps_ollama_endpoint_owner_with_path_protocol() {
    let profile = crate::net::policy_config::ProviderRuleProfile::parse_toml(
        r#"
[ai.ollama]
name = "Ollama"
protocol = "ollama"
url = "http://127.0.0.1:11434"
listen_ports = [11434]

[ai.ollama.rules.local]
name = "ollama_local"
action = "allow"
match = 'http.host == "127.0.0.1"'
"#,
    )
    .expect("provider profile parses");
    let registry = profile.endpoint_registry().expect("registry builds");

    assert_eq!(
        ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/v1/messages"),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Ollama),
            protocol: Some(ModelProtocol::Anthropic),
        }
    );
    assert_eq!(
        ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/v1/responses"),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Ollama),
            protocol: Some(ModelProtocol::OpenAi),
        }
    );
    assert_eq!(
        ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/api/chat"),
        ModelTrafficIdentity {
            provider: Some(ProviderKind::Ollama),
            protocol: Some(ModelProtocol::Ollama),
        }
    );
}

#[test]
fn provider_detection_promotes_unknown_host_by_bounded_body_shape() {
    assert_eq!(
        ai_protocol_for_body_preview(br#"{"model":"gpt-4.1","messages":[{"role":"user","content":"hi"}]}"#),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(
        ai_protocol_for_body_preview(
            br#"{"model":"claude-3-5-sonnet","max_tokens":128,"messages":[{"role":"user","content":"hi"}]}"#
        ),
        Some(ModelProtocol::Anthropic)
    );
    assert_eq!(
        ai_protocol_for_body_preview(br#"{"model":"gemini-2.5-pro","contents":[{"parts":[{"text":"hi"}]}]}"#),
        Some(ModelProtocol::Google)
    );
}

#[test]
fn provider_detection_body_shape_ignores_oversized_or_irrelevant_bodies() {
    let mut oversized = vec![b' '; AI_BODY_CAPTURE_LIMIT + 1];
    oversized.extend_from_slice(br#"{"model":"gpt-4.1","messages":[{"role":"user","content":"hi"}]}"#);
    assert_eq!(ai_protocol_for_body_preview(&oversized), None);
    assert_eq!(ai_protocol_for_body_preview(br#"{"hello":"world"}"#), None);
}

#[test]
fn retry_replayability_is_limited_to_empty_idempotent_requests() {
    let headers = http::HeaderMap::new();
    assert!(request_can_replay_empty_body(&http::Method::GET, &headers));
    assert!(request_can_replay_empty_body(&http::Method::HEAD, &headers));
    assert!(!request_can_replay_empty_body(&http::Method::POST, &headers));

    let mut with_body = http::HeaderMap::new();
    with_body.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("12"));
    assert!(!request_can_replay_empty_body(&http::Method::GET, &with_body));

    let mut chunked = http::HeaderMap::new();
    chunked.insert(
        http::header::TRANSFER_ENCODING,
        http::HeaderValue::from_static("chunked"),
    );
    assert!(!request_can_replay_empty_body(&http::Method::GET, &chunked));
}

#[test]
fn provider_detection_keeps_body_sniffing_protocol_only() {
    assert_eq!(
        ai_protocol_for_body_preview(
            br#"{"model":"local-model","messages":[{"role":"user","content":"hi"}],"tools":[]}"#
        ),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(
        ai_identity_for_target_or_path(
            &crate::net::policy_config::ModelEndpointRegistry::default(),
            "127.0.0.1",
            3713,
            "/model/shape"
        ),
        ModelTrafficIdentity {
            provider: None,
            protocol: None,
        }
    );
}

#[test]
fn http_request_security_event_exposes_transport_and_body_to_cel() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.allow_local_fixture]
name = "allow_local_fixture"
action = "allow"
priority = -100
match = 'http.host == "127.0.0.1" && tcp.port == "3713" && ip.value == "127.0.0.1" && http.query == "case=plain-json" && http.body.contains("ironbank_http_plain_json")'
"#,
    )
    .expect("profile parses");
    let rules = SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::Corp)
        .expect("rules compile");

    let body = Bytes::from_static(br#"{"kind":"ironbank_http_plain_json"}"#);
    let event = http_request_security_event(HttpRequestSecurityEventInput {
        domain: "127.0.0.1",
        upstream_port: 3713,
        method: "POST",
        path: "/echo",
        query: Some("case=plain-json".to_string()),
        ai_provider: None,
        headers: http::HeaderMap::new(),
        body: Some(&body),
    });
    let first = rules
        .evaluate(&event)
        .expect("event evaluates")
        .enforcement_rules()
        .into_iter()
        .next()
        .expect("transport/body rule matches");

    assert_eq!(first.rule_id, "corp.rules.allow_local_fixture");
    assert_eq!(first.action, SecurityRuleAction::Allow);
}

#[test]
fn unknown_model_body_sniffing_is_json_and_length_bounded() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("128"));
    assert!(should_sniff_unknown_model_body(None, &http::Method::POST, &headers));
    assert!(!should_sniff_unknown_model_body(
        Some(ProviderKind::OpenAi),
        &http::Method::POST,
        &headers
    ));
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&(AI_BODY_CAPTURE_LIMIT + 1).to_string()).unwrap(),
    );
    assert!(!should_sniff_unknown_model_body(None, &http::Method::POST, &headers));
    headers.remove(http::header::CONTENT_LENGTH);
    assert!(!should_sniff_unknown_model_body(None, &http::Method::POST, &headers));
}

#[test]
fn unknown_mcp_http_body_sniffing_is_json_and_length_bounded() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("128"));
    assert!(should_sniff_mcp_http_body(&http::Method::POST, &headers));

    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&(MCP_BODY_CAPTURE_LIMIT + 1).to_string()).unwrap(),
    );
    assert!(!should_sniff_mcp_http_body(&http::Method::POST, &headers));

    headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("128"));
    assert!(!should_sniff_mcp_http_body(&http::Method::GET, &headers));

    headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("text/plain"));
    assert!(!should_sniff_mcp_http_body(&http::Method::POST, &headers));
}

#[test]
fn observed_mcp_http_request_requires_mcp_json_rpc_shape() {
    let body = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fetch_http","arguments":{"url":"https://example.com"}}}"#;
    let observed = observed_mcp_http_request_for_body(body, "mcp.example.test", 443, "/mcp").unwrap();
    assert_eq!(observed.method, "tools/call");
    assert_eq!(observed.tool_name.as_deref(), Some("fetch_http"));
    assert_eq!(observed.request_id.as_deref(), Some("7"));
    assert_eq!(observed.server_name, "observed:mcp.example.test:443/mcp");

    assert!(observed_mcp_http_request_for_body(
        br#"{"jsonrpc":"2.0","method":"eth_call"}"#,
        "rpc.example.test",
        443,
        "/"
    )
    .is_none());
    assert!(observed_mcp_http_request_for_body(
        br#"{"method":"tools/call","params":{"name":"fetch_http"}}"#,
        "mcp.example.test",
        443,
        "/mcp"
    )
    .is_none());
}

#[test]
fn observed_mcp_http_request_extracts_tool_name_past_large_arguments() {
    // The targeted deserializer must still read tools/call `name` even when
    // params carries a large `arguments` blob (which it must not DOM-parse).
    let big = "z".repeat(300_000);
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":"abc","method":"tools/call","params":{{"name":"do_it","arguments":{{"blob":"{big}"}}}}}}"#
    );
    let observed = observed_mcp_http_request_for_body(body.as_bytes(), "mcp.example.test", 443, "/mcp").unwrap();
    assert_eq!(observed.method, "tools/call");
    assert_eq!(observed.tool_name.as_deref(), Some("do_it"));
    assert_eq!(observed.request_id.as_deref(), Some("abc"));
}

#[test]
fn observed_mcp_http_request_preview_is_capped() {
    // A guest can send a valid MCP JSON-RPC request with a huge params blob (up
    // to MCP_BODY_CAPTURE_LIMIT). The stored preview must be bounded like the
    // framed path, not the whole multi-megabyte body pushed into the ledger.
    let filler = "a".repeat(200_000);
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"x","pad":"{filler}"}}}}"#
    );
    let observed = observed_mcp_http_request_for_body(body.as_bytes(), "mcp.example.test", 443, "/mcp").unwrap();
    let preview = observed.request_preview.expect("preview present");
    assert!(
        preview.len() <= mcp_frame::MCP_REQUEST_PREVIEW_BYTES,
        "observed-MCP preview must be capped at {} bytes, got {}",
        mcp_frame::MCP_REQUEST_PREVIEW_BYTES,
        preview.len()
    );
}

#[test]
fn body_capture_limit_captures_oauth_broker_candidates_without_body_logging() {
    assert_eq!(
        body_capture_limit(None, "oauth2.googleapis.com", "/token", false, 0),
        CREDENTIAL_BODY_CAPTURE_LIMIT
    );
    assert_eq!(
        body_capture_limit(None, "api.github.com", "/login/oauth/access_token", false, 0),
        CREDENTIAL_BODY_CAPTURE_LIMIT
    );
}

#[test]
fn body_capture_limit_keeps_unrelated_non_ai_bodies_off_without_body_logging() {
    assert_eq!(
        body_capture_limit(
            None,
            "daily-cloudcode-pa.googleapis.com",
            "/v1internal:streamGenerateContent",
            false,
            0
        ),
        0
    );
}

#[test]
fn response_body_capture_limit_captures_broker_replay_proof_without_body_logging() {
    assert_eq!(
        response_body_capture_limit(
            None,
            "127.0.0.1",
            "/echo",
            false,
            0,
            Some("credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        ),
        CREDENTIAL_BODY_CAPTURE_LIMIT
    );
    assert_eq!(
        response_body_capture_limit(None, "127.0.0.1", "/echo", false, 0, None),
        0
    );
}

#[test]
fn body_capture_limit_keeps_ai_capture_independent_from_body_logging() {
    assert_eq!(
        body_capture_limit(
            Some(ProviderKind::Google),
            "daily-cloudcode-pa.googleapis.com",
            "/v1internal:streamGenerateContent",
            false,
            0
        ),
        AI_BODY_CAPTURE_LIMIT
    );
    assert_eq!(
        body_capture_limit(
            Some(ProviderKind::Anthropic),
            "127.0.0.1",
            "/v1/messages",
            false,
            128 * 1024
        ),
        AI_BODY_CAPTURE_LIMIT
    );
}

// -----------------------------------------------------------------------
// Plain-HTTP upstream port allowlist. The port comes from the guest's Host
// header, so a guest reaching the proxy directly must not be able to make the
// host dial an arbitrary port. TLS and an empty allowlist are unrestricted.
// -----------------------------------------------------------------------

#[test]
fn http_upstream_port_gate_denies_port_off_allowlist() {
    let policy = NetworkMechanics::default(); // default allowlist: 80, 3128, 3713, 8080, 11434
    assert!(!http_upstream_port_allowed(&policy, Protocol::Http, 22));
    assert!(!http_upstream_port_allowed(&policy, Protocol::Http, 443));
}

#[test]
fn http_upstream_port_gate_allows_listed_ports() {
    let policy = NetworkMechanics::default();
    assert!(http_upstream_port_allowed(&policy, Protocol::Http, 80));
    assert!(http_upstream_port_allowed(&policy, Protocol::Http, 8080));
}

#[test]
fn http_upstream_port_gate_ignores_tls_and_empty_allowlist() {
    let policy = NetworkMechanics::default();
    // TLS terminates at 443 and is not gated by the plain-HTTP allowlist.
    assert!(http_upstream_port_allowed(&policy, Protocol::Tls, 22));

    let mut open = NetworkMechanics::default();
    open.http_upstream_ports.clear();
    // Empty allowlist means "no restriction configured".
    assert!(http_upstream_port_allowed(&open, Protocol::Http, 22));
}
