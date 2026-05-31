use std::collections::BTreeMap;

use super::*;

fn sample_policy_context() -> PolicyContext {
    let mut request_headers = BTreeMap::new();
    request_headers.insert(
        "Authorization".to_string(),
        vec!["Bearer redacted".to_string()],
    );
    request_headers.insert("x-capsem-trace".to_string(), vec!["trace-1".to_string()]);

    let mut response_headers = BTreeMap::new();
    response_headers.insert(
        "content-type".to_string(),
        vec!["application/json".to_string()],
    );

    PolicyContext {
        common: CommonPolicyContext {
            session_id: Some("session-1".to_string()),
            vm_id: Some("vm-1".to_string()),
            profile_id: Some("profile-1".to_string()),
            profile_revision: Some("rev-1".to_string()),
            user_id: Some("user-1".to_string()),
            event_type: Some("http.request".to_string()),
            enforceability: Some("enforceable".to_string()),
            actor: Some("agent".to_string()),
            process: Some(ProcessIdentityPolicyContext {
                pid: Some(123),
                ppid: Some(1),
                executable: Some("/usr/bin/curl".to_string()),
                command: Some("curl".to_string()),
                cwd: Some("/workspace".to_string()),
            }),
            labels: BTreeMap::from([("profile".to_string(), "default".to_string())]),
        },
        http: HttpPolicyContext {
            request: Some(HttpRequestPolicyContext {
                method: Some("POST".to_string()),
                scheme: Some("https".to_string()),
                host: Some("api.example.test".to_string()),
                port: Some(443),
                path: Some("/v1/messages".to_string()),
                query: None,
                url: Some("https://api.example.test/v1/messages".to_string()),
                path_class: Some("api".to_string()),
                bytes: Some(128),
                headers: request_headers,
                body: BodyPolicyContext::text(r#"{"hello":"world"}"#),
            }),
            response: Some(HttpResponsePolicyContext {
                status: Some(200),
                bytes: Some(256),
                headers: response_headers,
                body: BodyPolicyContext::redacted("contains model output"),
            }),
        },
        dns: DnsPolicyContext {
            request: Some(DnsRequestPolicyContext {
                qname: Some("api.example.test".to_string()),
                qtype: Some("A".to_string()),
                domain_class: Some("external".to_string()),
                transport: Some("udp".to_string()),
            }),
        },
        mcp: McpPolicyContext {
            request: Some(McpRequestPolicyContext {
                method: Some("tools/call".to_string()),
                server_id: Some("local-server".to_string()),
                tool_name: Some("shell".to_string()),
                server_name: Some("local".to_string()),
                arguments_status: Some("valid_json".to_string()),
                arguments: BodyPolicyContext::text(r#"{"path":"/workspace/a.txt"}"#),
            }),
            response: Some(McpResponsePolicyContext {
                method: Some("tools/call".to_string()),
                server_id: Some("local-server".to_string()),
                tool_name: Some("shell".to_string()),
                is_error: Some(false),
                result_status: Some("ok".to_string()),
                result: BodyPolicyContext::text(r#"{"ok":true}"#),
            }),
        },
        model: ModelPolicyContext {
            request: Some(ModelRequestPolicyContext {
                provider: Some("anthropic".to_string()),
                api_family: Some("messages".to_string()),
                model: Some("claude-sonnet".to_string()),
                stream: Some(true),
                operation: Some("messages.create".to_string()),
                estimated_input_tokens: Some(100),
                estimated_output_tokens: Some(40),
                estimated_cost_micros: Some(12),
                body: BodyPolicyContext::redacted("prompt redacted"),
                tool_calls: vec![ModelToolCallPolicyContext {
                    tool_call_id: Some("toolu_1".to_string()),
                    provider_call_id: Some("provider-toolu-1".to_string()),
                    raw_name: Some("filesystem.read_file".to_string()),
                    name: Some("filesystem.read_file".to_string()),
                    origin: Some("mcp_tool".to_string()),
                    arguments_status: Some("valid_json".to_string()),
                    arguments: BodyPolicyContext::text(r#"{"path":"/workspace/a.txt"}"#),
                    status: Some("executed".to_string()),
                    linked_mcp_call_id: Some("mcp-call-1".to_string()),
                    parse_confidence: Some("high".to_string()),
                }],
            }),
            response: Some(ModelResponsePolicyContext {
                provider: Some("anthropic".to_string()),
                api_family: Some("messages".to_string()),
                model: Some("claude-sonnet".to_string()),
                status: Some(200),
                stop_reason: Some("end_turn".to_string()),
                estimated_output_tokens: Some(40),
                body: BodyPolicyContext::missing(),
                tool_results: vec![ModelToolResultPolicyContext {
                    tool_call_id: Some("toolu_1".to_string()),
                    linked_mcp_call_id: Some("mcp-call-1".to_string()),
                    content_kind: Some("json".to_string()),
                    content_preview: Some("{\"ok\":true}".to_string()),
                    content_json: Some("{\"ok\":true}".to_string()),
                    is_error: Some(false),
                    result_status: Some("returned_to_model".to_string()),
                    returned_to_model: Some(true),
                    parse_confidence: Some("high".to_string()),
                }],
            }),
        },
        file: FilePolicyContext {
            activity: Some(FileActivityPolicyContext {
                operation: Some("read".to_string()),
                path: Some("/workspace/README.md".to_string()),
                path_class: Some("workspace".to_string()),
                byte_count: Some(512),
                content: BodyPolicyContext::text("hello workspace"),
            }),
        },
        process: ProcessPolicyContext {
            activity: Some(ProcessActivityPolicyContext {
                operation: Some("exec".to_string()),
                executable: Some("/usr/bin/curl".to_string()),
                command: Some("curl".to_string()),
                command_class: Some("network_client".to_string()),
                argv: vec!["curl".to_string(), "https://api.example.test".to_string()],
                cwd: Some("/workspace".to_string()),
            }),
        },
        credential: CredentialPolicyContext {
            activity: Some(CredentialActivityPolicyContext {
                operation: Some("read".to_string()),
                credential_id: Some("api-token".to_string()),
            }),
        },
        vm: VmPolicyContext {
            activity: Some(VmActivityPolicyContext {
                operation: Some("start".to_string()),
            }),
        },
        profile: ProfilePolicyContext {
            activity: Some(ProfileActivityPolicyContext {
                operation: Some("select".to_string()),
                profile_id: Some("profile-1".to_string()),
                profile_revision: Some("rev-1".to_string()),
                profile_name: Some("Default".to_string()),
            }),
        },
        conversation: ConversationPolicyContext {
            activity: Some(ConversationActivityPolicyContext {
                operation: Some("append".to_string()),
                conversation_id: Some("conv-1".to_string()),
            }),
        },
        snapshot: SnapshotPolicyContext {
            activity: Some(SnapshotActivityPolicyContext {
                operation: Some("create".to_string()),
                snapshot_id: Some("snap-1".to_string()),
            }),
        },
        ..PolicyContext::new()
    }
}

#[test]
fn policy_context_roundtrips_json_and_messagepack() {
    let context = sample_policy_context();

    let json = serde_json::to_vec(&context).unwrap();
    let from_json: PolicyContext = serde_json::from_slice(&json).unwrap();
    assert_eq!(from_json, context);

    let msgpack = rmp_serde::to_vec_named(&context).unwrap();
    let from_msgpack: PolicyContext = rmp_serde::from_slice(&msgpack).unwrap();
    assert_eq!(from_msgpack, context);
}

#[test]
fn default_and_new_policy_context_set_schema_version() {
    assert_eq!(
        PolicyContext::new().schema_version,
        POLICY_CONTEXT_SCHEMA_VERSION
    );
    assert_eq!(
        PolicyContext::default().schema_version,
        POLICY_CONTEXT_SCHEMA_VERSION
    );
}

#[test]
fn policy_context_rejects_unknown_fields() {
    let err = serde_json::from_str::<PolicyContext>(
        r#"{"schema_version":1,"common":{},"surprise":true}"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn nested_policy_context_rejects_unknown_fields() {
    let err = serde_json::from_str::<PolicyContext>(
        r#"{"schema_version":1,"http":{"request":{"host":"example.test","surprise":true}}}"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn http_header_lookup_is_case_insensitive_and_deterministic() {
    let request = HttpRequestPolicyContext {
        headers: BTreeMap::from([
            ("authorization".to_string(), vec!["lower".to_string()]),
            ("Authorization".to_string(), vec!["upper".to_string()]),
        ]),
        ..HttpRequestPolicyContext::default()
    };

    assert_eq!(request.header("AUTHORIZATION"), Some("upper"));
    assert_eq!(
        request.header_values("authorization"),
        Some(vec!["upper".to_string()].as_slice())
    );

    let keys: Vec<_> = request.headers.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["Authorization", "authorization"]);
}

#[test]
fn missing_and_redacted_body_semantics_are_explicit() {
    let missing = BodyPolicyContext::missing();
    assert_eq!(missing.state, BodyState::Missing);
    assert!(missing.text.is_none());
    assert!(missing.redaction_reason.is_none());

    let redacted = BodyPolicyContext::redacted("sensitive");
    assert_eq!(redacted.state, BodyState::Redacted);
    assert!(redacted.text.is_none());
    assert_eq!(redacted.redaction_reason.as_deref(), Some("sensitive"));

    let json = serde_json::to_string(&redacted).unwrap();
    assert!(json.contains(r#""state":"redacted""#));
    assert!(json.contains(r#""redaction_reason":"sensitive""#));
}

#[test]
fn public_policy_context_type_names_do_not_end_with_v1() {
    let source = include_str!("../policy_context.rs");

    for line in source.lines() {
        let trimmed = line.trim_start();
        let public_name = trimmed
            .strip_prefix("pub struct ")
            .or_else(|| trimmed.strip_prefix("pub enum "))
            .or_else(|| trimmed.strip_prefix("pub type "));

        if let Some(rest) = public_name {
            let name = rest
                .split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
                .next()
                .unwrap_or_default();
            assert!(
                !name.ends_with("V1"),
                "public policy context type has a V1 suffix: {name}"
            );
        }
    }
}
