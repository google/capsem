use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use capsem_logger::{DbReader, DbWriter};
use capsem_proto::MCP_FRAME_FLAG_NOTIFICATION;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::mcp::aggregator::{
    AggregatorMethod, AggregatorRequest, AggregatorResponse, AggregatorResult,
    AggregatorServerStatus,
};
use crate::mcp::policy::{
    McpDecisionRule, McpDecisionRuleAction, McpDecisionRuleMatch, McpPolicy, ToolDecision,
};
use crate::mcp::types::McpToolDef;
use crate::net::mitm_proxy::{McpEndpointState, McpTimeouts};
use crate::net::policy_confirm::{
    ConfirmArgs, Confirmer, ConfirmerKind, Decision as ConfirmDecision,
};
use crate::net::policy_v2::PolicyConfig;

use super::*;

static MCP_TIMEOUT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn request_payload(id: u64, method: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    }))
    .unwrap()
}

fn request_payload_with_json_id(id: serde_json::Value, method: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    }))
    .unwrap()
}

fn request_payload_with_json_id_and_params(
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .unwrap()
}

fn request_payload_with_params(id: u64, method: &str, params: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .unwrap()
}

fn request_summary(payload: &[u8]) -> McpMethodSummary {
    let req = parse_json_rpc_payload(payload).unwrap();
    interpret_mcp_method(&req)
}

fn decision_request(process_name: &str, payload: &[u8]) -> McpDecisionRequest {
    let req = parse_json_rpc_payload(payload).unwrap();
    let summary = interpret_mcp_method(&req);
    McpDecisionRequest::from_request(process_name, &req, &summary)
}

fn rule(id: &str, matches: McpDecisionRuleMatch) -> McpDecisionRule {
    McpDecisionRule {
        id: id.to_string(),
        action: McpDecisionRuleAction::Deny,
        matches,
        reason: Some(format!("{id} blocked")),
    }
}

fn policy_with_rules(rules: Vec<McpDecisionRule>) -> McpPolicy {
    McpPolicy {
        audit_rules: rules,
        ..McpPolicy::new()
    }
}

struct MockConfirmer {
    decision: ConfirmDecision,
    calls: std::sync::Mutex<Vec<ConfirmArgs>>,
}

impl MockConfirmer {
    fn new(decision: ConfirmDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<ConfirmArgs> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Confirmer for MockConfirmer {
    async fn confirm(&self, args: ConfirmArgs) -> ConfirmDecision {
        self.calls.lock().unwrap().push(args);
        self.decision
    }

    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Automated
    }
}

fn restore_env(key: &str, value: Option<String>) {
    // SAFETY: callers hold MCP_TIMEOUT_ENV_LOCK because environment
    // variables are process-global and Rust tests run concurrently.
    unsafe {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[tokio::test]
async fn mcp_endpoint_default_timeouts_match_t3_contract() {
    let timeouts = McpTimeouts::default();

    assert_eq!(timeouts.default_timeout, Duration::from_secs(60));
    assert_eq!(timeouts.tool_call_default, Duration::from_secs(300));
    assert_eq!(timeouts.tool_call_ceiling, Duration::from_secs(300));
}

#[test]
fn mcp_endpoint_timeouts_read_env_overrides() {
    let _guard = MCP_TIMEOUT_ENV_LOCK.lock().unwrap();
    let default_prev = std::env::var("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS").ok();
    let tool_prev = std::env::var("CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS").ok();
    let ceiling_prev = std::env::var("CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS").ok();

    // SAFETY: guarded by MCP_TIMEOUT_ENV_LOCK because environment variables
    // are process-global and Rust tests run concurrently by default.
    unsafe {
        std::env::set_var("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS", "5");
        std::env::set_var("CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS", "7");
        std::env::set_var("CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS", "9");
    }

    let timeouts = McpTimeouts::from_env();

    assert_eq!(timeouts.default_timeout, Duration::from_secs(5));
    assert_eq!(timeouts.tool_call_default, Duration::from_secs(7));
    assert_eq!(timeouts.tool_call_ceiling, Duration::from_secs(9));

    restore_env("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS", default_prev);
    restore_env("CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS", tool_prev);
    restore_env("CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS", ceiling_prev);
}

#[tokio::test]
async fn mcp_endpoint_clamps_catalog_tool_timeout_overrides() {
    let state = test_mcp_endpoint_state_with_timeouts(
        McpPolicy::new(),
        McpTimeouts {
            default_timeout: Duration::from_secs(60),
            tool_call_default: Duration::from_secs(300),
            tool_call_ceiling: Duration::from_secs(300),
        },
    );
    state
        .record_tool_catalog_timeouts(&[McpToolDef {
            namespaced_name: "github__slow_search".to_string(),
            original_name: "slow_search".to_string(),
            description: None,
            input_schema: serde_json::json!({}),
            server_name: "github".to_string(),
            annotations: None,
            timeout_secs: Some(600),
        }])
        .await;

    assert_eq!(
        state
            .timeout_for_request("tools/call", Some("github__slow_search"))
            .await,
        Duration::from_secs(300)
    );
}

#[tokio::test]
async fn mcp_endpoint_tools_list_populates_catalog_timeout_overrides() {
    let state = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts {
            default_timeout: Duration::from_secs(60),
            tool_call_default: Duration::from_secs(300),
            tool_call_ceiling: Duration::from_secs(300),
        },
        |req| async move {
            assert!(matches!(req.method, AggregatorMethod::ListTools));
            AggregatorResult::Tools {
                tools: vec![McpToolDef {
                    namespaced_name: "github__slow_search".to_string(),
                    original_name: "slow_search".to_string(),
                    description: None,
                    input_schema: serde_json::json!({}),
                    server_name: "github".to_string(),
                    annotations: None,
                    timeout_secs: Some(120),
                }],
            }
        },
    );
    let req =
        parse_json_rpc_payload(br#"{"jsonrpc":"2.0","id":32,"method":"tools/list"}"#).unwrap();

    let response = state.handle_request(&req).await.unwrap();

    assert!(response.error.is_none());
    assert_eq!(
        state
            .timeout_for_request("tools/call", Some("github__slow_search"))
            .await,
        Duration::from_secs(120)
    );
}

#[tokio::test]
async fn mcp_endpoint_times_out_non_tool_methods() {
    let state = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts {
            default_timeout: Duration::from_millis(10),
            tool_call_default: Duration::from_secs(300),
            tool_call_ceiling: Duration::from_secs(300),
        },
        |req| async move {
            if matches!(req.method, AggregatorMethod::ListResources) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            AggregatorResult::Resources { resources: vec![] }
        },
    );
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":31,"method":"resources/list","params":{}}"#,
    )
    .unwrap();

    let response = state.handle_request(&req).await.unwrap();

    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("timed out")));
}

#[tokio::test]
async fn frame_reader_discards_corrupt_body_and_reads_next_frame() {
    let first =
        capsem_proto::encode_mcp_frame(7, 0, "codex", &request_payload(7, "tools/list")).unwrap();
    let mut corrupt = first.clone();
    corrupt[4] = b'X';
    let second =
        capsem_proto::encode_mcp_frame(8, 0, "claude", &request_payload(8, "resources/list"))
            .unwrap();

    let mut wire = corrupt;
    wire.extend_from_slice(&second);
    let mut reader = Cursor::new(wire);

    let first = read_next_frame(&mut reader).await.unwrap();
    assert!(matches!(
        first,
        FrameRead::InvalidFrame {
            stream_id: Some(7),
            ..
        }
    ));

    let second = read_next_frame(&mut reader).await.unwrap();
    let FrameRead::Frame(frame) = second else {
        panic!("expected valid second frame");
    };
    assert_eq!(frame.stream_id, 8);
    assert_eq!(frame.process_name, "claude");
}

#[tokio::test]
async fn frame_reader_rejects_invalid_total_length_as_connection_error() {
    let mut reader = Cursor::new([0xff, 0xff, 0xff, 0xff]);
    let err = read_next_frame(&mut reader).await.unwrap_err();
    assert!(err.to_string().contains("invalid MCP frame length"));
}

#[test]
fn stream_tracker_accepts_monotonic_requests_and_skips_notifications() {
    let mut tracker = StreamTracker::default();

    assert_eq!(tracker.begin(1, false).unwrap(), StreamDisposition::Request);
    assert_eq!(tracker.begin(2, false).unwrap(), StreamDisposition::Request);
    assert_eq!(
        tracker.begin(0, true).unwrap(),
        StreamDisposition::Notification
    );

    tracker.complete(1);
    tracker.complete(2);
    assert!(tracker.is_empty());
}

#[test]
fn stream_tracker_rejects_duplicate_inflight_stream_id() {
    let mut tracker = StreamTracker::default();

    assert_eq!(tracker.begin(4, false).unwrap(), StreamDisposition::Request);
    let err = tracker.begin(4, false).unwrap_err();
    assert!(err.to_string().contains("duplicate MCP stream id"));
}

#[test]
fn stream_tracker_rejects_non_monotonic_reuse_after_completion() {
    let mut tracker = StreamTracker::default();

    assert_eq!(tracker.begin(4, false).unwrap(), StreamDisposition::Request);
    tracker.complete(4);
    let err = tracker.begin(4, false).unwrap_err();
    assert!(err.to_string().contains("non-monotonic MCP stream id"));
}

#[test]
fn stream_tracker_rejects_request_on_reserved_notification_stream() {
    let mut tracker = StreamTracker::default();

    let err = tracker.begin(0, false).unwrap_err();
    assert!(err.to_string().contains("stream id 0 is reserved"));
}

#[test]
fn parse_json_rpc_payload_rejects_oversized_payload_before_deserialize() {
    let payload = vec![b' '; MCP_JSON_RPC_MAX_BYTES + 1];
    let err = parse_json_rpc_payload(&payload).unwrap_err();
    assert!(err.to_string().contains("JSON-RPC payload too large"));
}

#[test]
fn parse_json_rpc_payload_requires_jsonrpc_2() {
    let err =
        parse_json_rpc_payload(br#"{"jsonrpc":"1.0","id":1,"method":"tools/list"}"#).unwrap_err();
    assert!(err.to_string().contains("unsupported JSON-RPC version"));
}

#[test]
fn parse_json_rpc_payload_preserves_string_request_id() {
    let req = parse_json_rpc_payload(&request_payload_with_json_id(
        serde_json::json!("tools-list-string"),
        "tools/list",
    ))
    .unwrap();
    assert_eq!(
        req.id.as_ref(),
        Some(&serde_json::json!("tools-list-string"))
    );
}

#[test]
fn interpret_tools_call_extracts_server_tool_and_arguments() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"github__search_repos","arguments":{"q":"capsem"}}}"#,
    )
    .unwrap();

    let summary = interpret_mcp_method(&req);
    assert_eq!(summary.kind, McpMethodKind::ToolsCall);
    assert_eq!(summary.method, "tools/call");
    assert_eq!(summary.server_name.as_deref(), Some("github"));
    assert_eq!(summary.tool_name.as_deref(), Some("github__search_repos"));
    assert_eq!(summary.request_hash.len(), 64);
    assert!(summary
        .request_preview
        .as_deref()
        .unwrap()
        .contains("capsem"));
}

#[test]
fn interpret_resources_read_extracts_server_and_resource_uri() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"capsem://docs/file:///workspace/readme.md"}}"#,
    )
    .unwrap();

    let summary = interpret_mcp_method(&req);
    assert_eq!(summary.kind, McpMethodKind::ResourcesRead);
    assert_eq!(summary.server_name.as_deref(), Some("docs"));
    assert_eq!(
        summary.resource_uri.as_deref(),
        Some("capsem://docs/file:///workspace/readme.md")
    );
}

#[test]
fn interpret_prompts_get_extracts_server_and_prompt() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"linear__triage","arguments":{"issue":"CAP-1"}}}"#,
    )
    .unwrap();

    let summary = interpret_mcp_method(&req);
    assert_eq!(summary.kind, McpMethodKind::PromptsGet);
    assert_eq!(summary.server_name.as_deref(), Some("linear"));
    assert_eq!(summary.prompt_name.as_deref(), Some("linear__triage"));
}

#[test]
fn interpret_notification_is_marked_without_request_id() {
    let req = parse_json_rpc_payload(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .unwrap();

    let summary = interpret_mcp_method(&req);
    assert_eq!(summary.kind, McpMethodKind::InitializedNotification);
    assert!(!summary.has_request_id);
}

#[test]
fn local_decision_provider_preserves_request_preview_and_hash() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"github__delete_repo","arguments":{"owner":"capsem","repo":"demo"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);

    let decision_request = McpDecisionRequest::from_request("codex", &req, &summary);

    assert_eq!(decision_request.process_name, "codex");
    assert_eq!(
        decision_request.arguments.as_ref().unwrap()["owner"],
        "capsem"
    );
    assert_eq!(
        decision_request.request_preview.as_deref(),
        summary.request_preview.as_deref()
    );
    assert_eq!(decision_request.request_hash, summary.request_hash);
}

#[test]
fn local_decision_provider_marks_blocked_tool_as_audit_deny() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"github__delete_repo","arguments":{}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let mut policy = McpPolicy::new();
    policy
        .tool_decisions
        .insert("github__delete_repo".to_string(), ToolDecision::Block);
    let provider = LocalMcpDecisionProvider::audit_only(policy);

    let decision = provider.decide(&McpDecisionRequest::from_summary("codex", &summary));

    assert_eq!(decision.mode, McpPolicyMode::AuditOnly);
    assert_eq!(decision.action, McpPolicyAction::Block);
    assert_eq!(decision.rule, "mcp.tool.github__delete_repo");
    assert!(decision.reason.contains("block"));
}

#[tokio::test]
async fn local_decision_provider_applies_policy_v2_mcp_request_rules() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.block_prod_token]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && has(arguments.prod_token)'
decision = "block"
priority = 10
reason = "Do not send production tokens to MCP tools"

[policy.mcp.ask_prod_issue]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && arguments.issue == "prod"'
decision = "ask"
priority = 20
reason = "Production issue creation needs approval"
"#,
    )
    .unwrap();
    let provider =
        LocalMcpDecisionProvider::audit_only_with_policy_v2(McpPolicy::new(), Arc::new(policy));

    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"issue":"prod","prod_token":"secret"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let decision = provider.decide(&McpDecisionRequest::from_request("codex", &req, &summary));
    assert_eq!(decision.mode, McpPolicyMode::Enforce);
    assert_eq!(decision.action, McpPolicyAction::Block);
    assert_eq!(decision.rule, "policy.mcp.block_prod_token");
    assert_eq!(
        decision.reason,
        "Do not send production tokens to MCP tools"
    );

    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"issue":"prod"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let request = McpDecisionRequest::from_request("codex", &req, &summary);
    let decision = provider
        .resolve_ask_request(provider.decide(&request), &request)
        .await;
    assert_eq!(decision.mode, McpPolicyMode::Enforce);
    assert_eq!(decision.action, McpPolicyAction::Allow);
    assert_eq!(decision.rule, "policy.mcp.ask_prod_issue");
    assert_eq!(decision.reason, "Production issue creation needs approval");
}

#[tokio::test]
async fn policy_v2_mcp_request_ask_with_deny_confirmer_blocks() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.ask_prod_issue]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && arguments.issue == "prod"'
decision = "ask"
priority = 10
reason = "Production issue creation needs approval"
"#,
    )
    .unwrap();
    let confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let provider = LocalMcpDecisionProvider::audit_only_with_policy_v2_and_confirmer(
        McpPolicy::new(),
        Arc::new(policy),
        confirmer.clone() as Arc<dyn Confirmer>,
    );
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"issue":"prod","prod_token":"secret"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let request = McpDecisionRequest::from_request("codex", &req, &summary);

    let decision = provider
        .resolve_ask_request(provider.decide(&request), &request)
        .await;

    assert_eq!(decision.mode, McpPolicyMode::Enforce);
    assert_eq!(decision.action, McpPolicyAction::Block);
    assert_eq!(decision.rule, "policy.mcp.ask_prod_issue");
    let calls = confirmer.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rule_id, "security.rules.mcp.ask_prod_issue");
    assert_eq!(
        calls[0].callback,
        crate::net::policy_v2::PolicyCallback::McpRequest
    );
    assert_eq!(
        calls[0]
            .args_snapshot
            .get("request")
            .and_then(|value| value.get("tool_name")),
        Some(&serde_json::json!("github__create_issue"))
    );
    let snapshot = serde_json::to_string(&calls[0].args_snapshot).unwrap();
    assert!(
        !snapshot.contains("prod_token") && !snapshot.contains("secret"),
        "confirm snapshots must not expose MCP argument contents: {snapshot}"
    );
}

#[tokio::test]
async fn policy_v2_mcp_response_ask_with_accept_and_deny_confirmer_resolves() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.ask_tool_response]
on = "mcp.response"
if = 'method == "tools/call" && tool.name == "github__create_issue"'
decision = "ask"
priority = 10
reason = "Confirm before surfacing tool response"
"#,
    )
    .unwrap();
    let request = decision_request(
        "codex",
        br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"issue":"prod"}}}"#,
    );
    let response = JsonRpcResponse::ok(
        Some(serde_json::json!(4)),
        serde_json::json!({"content":[{"type":"text","text":"created"}]}),
    );

    let accept_confirmer = MockConfirmer::new(ConfirmDecision::Accept);
    let accept_provider = LocalMcpDecisionProvider::audit_only_with_policy_v2_and_confirmer(
        McpPolicy::new(),
        Arc::new(policy.clone()),
        accept_confirmer.clone() as Arc<dyn Confirmer>,
    );
    let base = accept_provider.decide(&request);
    let decision = accept_provider
        .resolve_ask_response(
            accept_provider.decide_response(&request, &response, base),
            &request,
            &response,
        )
        .await;
    assert_eq!(decision.action, McpPolicyAction::Allow);
    assert_eq!(decision.rule, "policy.mcp.ask_tool_response");
    assert_eq!(
        accept_confirmer.calls()[0].callback,
        crate::net::policy_v2::PolicyCallback::McpResponse
    );

    let deny_confirmer = MockConfirmer::new(ConfirmDecision::Deny);
    let deny_provider = LocalMcpDecisionProvider::audit_only_with_policy_v2_and_confirmer(
        McpPolicy::new(),
        Arc::new(policy),
        deny_confirmer.clone() as Arc<dyn Confirmer>,
    );
    let base = deny_provider.decide(&request);
    let decision = deny_provider
        .resolve_ask_response(
            deny_provider.decide_response(&request, &response, base),
            &request,
            &response,
        )
        .await;
    assert_eq!(decision.action, McpPolicyAction::Block);
    assert_eq!(decision.rule, "policy.mcp.ask_tool_response");
    assert_eq!(
        deny_confirmer.calls()[0].rule_id,
        "security.rules.mcp.ask_tool_response"
    );
}

#[test]
fn local_decision_provider_preserves_policy_v2_allow_match() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.allow_safe_search]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__search_repos" && arguments.query == "capsem"'
decision = "allow"
priority = 10
reason = "Safe repository search"
"#,
    )
    .unwrap();
    let provider =
        LocalMcpDecisionProvider::audit_only_with_policy_v2(McpPolicy::new(), Arc::new(policy));

    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"github__search_repos","arguments":{"query":"capsem"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let decision = provider.decide(&McpDecisionRequest::from_request("codex", &req, &summary));

    assert_eq!(decision.mode, McpPolicyMode::Enforce);
    assert_eq!(decision.action, McpPolicyAction::Allow);
    assert_eq!(decision.rule, "policy.mcp.allow_safe_search");
    assert_eq!(decision.reason, "Safe repository search");
}

#[test]
fn local_decision_provider_maps_warn_to_allow_for_v1() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github__search_repos","arguments":{}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let mut policy = McpPolicy::new();
    policy
        .tool_decisions
        .insert("github__search_repos".to_string(), ToolDecision::Warn);
    let provider = LocalMcpDecisionProvider::audit_only(policy);

    let decision = provider.decide(&McpDecisionRequest::from_summary("codex", &summary));

    assert_eq!(decision.mode, McpPolicyMode::AuditOnly);
    assert_eq!(decision.action, McpPolicyAction::Allow);
    assert_eq!(decision.rule, "mcp.tool.github__search_repos");
    assert!(decision.reason.contains("warn"));
}

#[test]
fn local_decision_provider_allows_non_target_methods_in_audit_mode() {
    let provider = LocalMcpDecisionProvider::audit_only(McpPolicy::new());
    for payload in [
        br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"# as &[u8],
        br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        br#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#,
        br#"{"jsonrpc":"2.0","id":4,"method":"prompts/list"}"#,
        br#"{"jsonrpc":"2.0","id":5,"method":"experimental/ping"}"#,
    ] {
        let req = parse_json_rpc_payload(payload).unwrap();
        let summary = interpret_mcp_method(&req);
        let decision = provider.decide(&McpDecisionRequest::from_summary("codex", &summary));

        assert_eq!(decision.mode, McpPolicyMode::AuditOnly);
        assert_eq!(
            decision.action,
            McpPolicyAction::Allow,
            "{}",
            summary.method
        );
        assert!(decision.rule.starts_with("mcp.method."));
    }
}

#[test]
fn local_decision_provider_uses_server_level_policy_for_resources_and_prompts() {
    let mut policy = McpPolicy::new();
    policy.blocked_servers = vec!["docs".to_string(), "linear".to_string()];
    let provider = LocalMcpDecisionProvider::audit_only(policy);

    let resource_req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"capsem://docs/file:///workspace/readme.md"}}"#,
    )
    .unwrap();
    let resource_summary = interpret_mcp_method(&resource_req);
    let resource_decision = provider.decide(&McpDecisionRequest::from_summary(
        "codex",
        &resource_summary,
    ));

    assert_eq!(resource_decision.action, McpPolicyAction::Block);
    assert_eq!(resource_decision.rule, "mcp.resource.docs");

    let prompt_req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":5,"method":"prompts/get","params":{"name":"linear__triage","arguments":{}}}"#,
    )
    .unwrap();
    let prompt_summary = interpret_mcp_method(&prompt_req);
    let prompt_decision =
        provider.decide(&McpDecisionRequest::from_summary("codex", &prompt_summary));

    assert_eq!(prompt_decision.action, McpPolicyAction::Block);
    assert_eq!(prompt_decision.rule, "mcp.prompt.linear");
}

#[test]
fn local_decision_provider_blocks_tool_resource_arg_name_and_arg_value_rules() {
    let cases: Vec<(&str, McpDecisionRule, Vec<u8>, &str)> = vec![
        (
            "tool-name",
            rule(
                "deny-github-admin",
                McpDecisionRuleMatch::ToolName {
                    name: "github__delete_repo".to_string(),
                },
            ),
            request_payload_with_params(
                10,
                "tools/call",
                serde_json::json!({
                    "name": "github__delete_repo",
                    "arguments": {"owner": "capsem", "repo": "demo"}
                }),
            ),
            "mcp.rule.deny-github-admin",
        ),
        (
            "resource-uri",
            rule(
                "deny-secret-doc",
                McpDecisionRuleMatch::ResourceUri {
                    uri: "capsem://docs/file:///workspace/secret.md".to_string(),
                },
            ),
            request_payload_with_params(
                11,
                "resources/read",
                serde_json::json!({
                    "uri": "capsem://docs/file:///workspace/secret.md"
                }),
            ),
            "mcp.rule.deny-secret-doc",
        ),
        (
            "argument-name",
            rule(
                "deny-token-arg",
                McpDecisionRuleMatch::ArgumentName {
                    method: Some("tools/call".to_string()),
                    name: "token".to_string(),
                },
            ),
            request_payload_with_params(
                12,
                "tools/call",
                serde_json::json!({
                    "name": "github__search_repos",
                    "arguments": {"query": "capsem", "token": "secret"}
                }),
            ),
            "mcp.rule.deny-token-arg",
        ),
        (
            "argument-value",
            rule(
                "deny-danger-query",
                McpDecisionRuleMatch::ArgumentValue {
                    method: Some("tools/call".to_string()),
                    name: "query".to_string(),
                    equals: serde_json::json!("DROP TABLE"),
                },
            ),
            request_payload_with_params(
                13,
                "tools/call",
                serde_json::json!({
                    "name": "github__search_repos",
                    "arguments": {"query": "DROP TABLE"}
                }),
            ),
            "mcp.rule.deny-danger-query",
        ),
    ];

    for (name, audit_rule, payload, expected_rule) in cases {
        let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![audit_rule]));
        let request = decision_request("codex", &payload);
        let decision = provider.decide(&request);

        assert_eq!(decision.action, McpPolicyAction::Block, "{name}");
        assert_eq!(decision.rule, expected_rule, "{name}");
        assert!(
            decision.reason.contains("blocked"),
            "missing denial reason for {name}: {}",
            decision.reason
        );
    }
}

#[test]
fn local_decision_provider_argument_value_rule_does_not_match_other_values() {
    let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![rule(
        "deny-danger-query",
        McpDecisionRuleMatch::ArgumentValue {
            method: Some("tools/call".to_string()),
            name: "query".to_string(),
            equals: serde_json::json!("DROP TABLE"),
        },
    )]));
    let payload = request_payload_with_params(
        14,
        "tools/call",
        serde_json::json!({
            "name": "github__search_repos",
            "arguments": {"query": "capsem"}
        }),
    );
    let summary = request_summary(&payload);

    let request = decision_request("codex", &payload);
    let decision = provider.decide(&request);

    assert_eq!(decision.action, McpPolicyAction::Allow);
    assert_eq!(decision.rule, "mcp.tool.github__search_repos");
    assert_eq!(summary.tool_name.as_deref(), Some("github__search_repos"));
}

#[test]
fn local_decision_provider_denies_take_precedence_over_allow_rules() {
    let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![
        McpDecisionRule {
            id: "allow-github-search".to_string(),
            action: McpDecisionRuleAction::Allow,
            matches: McpDecisionRuleMatch::ToolName {
                name: "github__search_repos".to_string(),
            },
            reason: Some("explicit allow".to_string()),
        },
        rule(
            "deny-token-arg",
            McpDecisionRuleMatch::ArgumentName {
                method: Some("tools/call".to_string()),
                name: "token".to_string(),
            },
        ),
    ]));
    let payload = request_payload_with_params(
        16,
        "tools/call",
        serde_json::json!({
            "name": "github__search_repos",
            "arguments": {"query": "capsem", "token": "secret"}
        }),
    );

    let decision = provider.decide(&decision_request("codex", &payload));

    assert_eq!(decision.action, McpPolicyAction::Block);
    assert_eq!(decision.rule, "mcp.rule.deny-token-arg");
}

#[test]
fn local_decision_provider_matches_prompt_argument_rules() {
    let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![
        rule(
            "deny-prod-issue",
            McpDecisionRuleMatch::ArgumentValue {
                method: Some("prompts/get".to_string()),
                name: "issue".to_string(),
                equals: serde_json::json!("PROD-1"),
            },
        ),
        rule(
            "deny-token-arg",
            McpDecisionRuleMatch::ArgumentName {
                method: Some("prompts/get".to_string()),
                name: "token".to_string(),
            },
        ),
    ]));

    let value_payload = request_payload_with_params(
        17,
        "prompts/get",
        serde_json::json!({
            "name": "linear__triage",
            "arguments": {"issue": "PROD-1"}
        }),
    );
    let name_payload = request_payload_with_params(
        18,
        "prompts/get",
        serde_json::json!({
            "name": "linear__triage",
            "arguments": {"issue": "CAP-1", "token": "secret"}
        }),
    );

    let value_decision = provider.decide(&decision_request("codex", &value_payload));
    let name_decision = provider.decide(&decision_request("codex", &name_payload));

    assert_eq!(value_decision.action, McpPolicyAction::Block);
    assert_eq!(value_decision.rule, "mcp.rule.deny-prod-issue");
    assert_eq!(name_decision.action, McpPolicyAction::Block);
    assert_eq!(name_decision.rule, "mcp.rule.deny-token-arg");
}

#[test]
fn local_decision_provider_blocks_return_value_rules_after_response() {
    let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![rule(
        "deny-secret-return",
        McpDecisionRuleMatch::ReturnValue {
            method: Some("tools/call".to_string()),
            path: "classification".to_string(),
            equals: serde_json::json!("secret"),
        },
    )]));
    let payload = request_payload_with_params(
        15,
        "tools/call",
        serde_json::json!({
            "name": "github__search_repos",
            "arguments": {"query": "capsem"}
        }),
    );
    let request = decision_request("codex", &payload);
    let before_response = provider.decide(&request);
    assert_eq!(before_response.action, McpPolicyAction::Allow);

    let response = JsonRpcResponse::ok(
        Some(serde_json::json!(15)),
        serde_json::json!({"classification": "secret", "items": []}),
    );
    let after_response = provider.decide_response(&request, &response, before_response);

    assert_eq!(after_response.action, McpPolicyAction::Block);
    assert_eq!(after_response.rule, "mcp.rule.deny-secret-return");
}

#[test]
fn local_decision_provider_return_rules_match_nested_paths_and_ignore_misses() {
    let provider = LocalMcpDecisionProvider::audit_only(policy_with_rules(vec![rule(
        "deny-nested-secret-return",
        McpDecisionRuleMatch::ReturnValue {
            method: Some("tools/call".to_string()),
            path: "metadata.classification".to_string(),
            equals: serde_json::json!("secret"),
        },
    )]));
    let payload = request_payload_with_params(
        19,
        "tools/call",
        serde_json::json!({
            "name": "github__search_repos",
            "arguments": {"query": "capsem"}
        }),
    );
    let request = decision_request("codex", &payload);
    let base = provider.decide(&request);
    let public_response = JsonRpcResponse::ok(
        Some(serde_json::json!(19)),
        serde_json::json!({"metadata": {"classification": "public"}}),
    );
    let secret_response = JsonRpcResponse::ok(
        Some(serde_json::json!(19)),
        serde_json::json!({"metadata": {"classification": "secret"}}),
    );
    let wrong_method = request_payload_with_params(
        20,
        "prompts/get",
        serde_json::json!({
            "name": "github__search_repos",
            "arguments": {"query": "capsem"}
        }),
    );
    let wrong_request = decision_request("codex", &wrong_method);

    let public_decision = provider.decide_response(&request, &public_response, base.clone());
    let secret_decision = provider.decide_response(&request, &secret_response, base);
    let wrong_method_decision = provider.decide_response(
        &wrong_request,
        &secret_response,
        provider.decide(&wrong_request),
    );

    assert_eq!(public_decision.action, McpPolicyAction::Allow);
    assert_eq!(secret_decision.action, McpPolicyAction::Block);
    assert_eq!(secret_decision.rule, "mcp.rule.deny-nested-secret-return");
    assert_eq!(wrong_method_decision.action, McpPolicyAction::Allow);
}

#[tokio::test]
async fn framed_session_records_policy_fields_after_live_policy_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(&db_path, McpPolicy::new());
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        21,
        request_payload_with_params(
            21,
            "tools/call",
            serde_json::json!({
                "name": "github__search_repos",
                "arguments": {"query": "capsem"}
            }),
        ),
    )
    .await;
    let first_response = read_next_frame(&mut client).await.unwrap();
    assert!(matches!(first_response, FrameRead::Frame(_)));

    *config.policy.write().await = Arc::new(policy_with_rules(vec![rule(
        "deny-danger-query",
        McpDecisionRuleMatch::ArgumentValue {
            method: Some("tools/call".to_string()),
            name: "query".to_string(),
            equals: serde_json::json!("DROP TABLE"),
        },
    )]));

    write_mcp_request_frame(
        &mut client,
        22,
        request_payload_with_params(
            22,
            "tools/call",
            serde_json::json!({
                "name": "github__search_repos",
                "arguments": {"query": "DROP TABLE"}
            }),
        ),
    )
    .await;
    let second_response = read_response_frame(&mut client).await;
    assert!(second_response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("blocked by policy")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    let db = Arc::clone(&config.db);
    drop(config);
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    let first = calls
        .iter()
        .find(|call| call.request_id.as_deref() == Some("21"))
        .expect("first framed MCP call should be logged");
    let second = calls
        .iter()
        .find(|call| call.request_id.as_deref() == Some("22"))
        .expect("second framed MCP call should be logged");

    assert_eq!(first.policy_mode.as_deref(), Some("audit_only"));
    assert_eq!(first.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        first.policy_rule.as_deref(),
        Some("mcp.tool.github__search_repos")
    );
    assert!(first
        .request_preview
        .as_deref()
        .is_some_and(|preview| preview.contains("capsem")));
    assert!(first.response_preview.as_deref().is_some_and(|preview| {
        preview.contains("\"tool\"") && preview.contains("github__search_repos")
    }));

    assert_eq!(second.policy_mode.as_deref(), Some("audit_only"));
    assert_eq!(second.policy_action.as_deref(), Some("block"));
    assert_eq!(
        second.policy_rule.as_deref(),
        Some("mcp.rule.deny-danger-query")
    );
    assert!(second
        .policy_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("blocked")));
    assert!(second
        .request_preview
        .as_deref()
        .is_some_and(|preview| preview.contains("DROP TABLE")));
}

#[test]
fn json_rpc_id_log_string_preserves_spec_id_shapes() {
    assert_eq!(
        json_rpc_id_to_log_string(&serde_json::json!("req-abc")).as_deref(),
        Some("req-abc")
    );
    assert_eq!(
        json_rpc_id_to_log_string(&serde_json::json!(42)).as_deref(),
        Some("42")
    );
    assert_eq!(
        json_rpc_id_to_log_string(&serde_json::Value::Null).as_deref(),
        Some("null")
    );
}

#[tokio::test]
async fn framed_session_records_string_json_rpc_request_id() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(&db_path, McpPolicy::new());
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        23,
        request_payload_with_json_id_and_params(
            serde_json::json!("string-id-23"),
            "tools/call",
            serde_json::json!({
                "name": "github__search_repos",
                "arguments": {"query": "capsem"}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(
        response.error.is_none(),
        "unexpected response: {response:?}"
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("string-id-23"))
        .expect("string JSON-RPC id should be preserved in mcp_calls");

    assert_eq!(call.method, "tools/call");
    assert_eq!(call.tool_name.as_deref(), Some("github__search_repos"));
    assert_eq!(call.policy_action.as_deref(), Some("allow"));
}

#[tokio::test]
async fn framed_session_blocks_request_rule_matrix_and_records_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(
        &db_path,
        policy_with_rules(vec![
            rule(
                "deny-tool-name",
                McpDecisionRuleMatch::ToolName {
                    name: "github__delete_repo".to_string(),
                },
            ),
            rule(
                "deny-resource-uri",
                McpDecisionRuleMatch::ResourceUri {
                    uri: "capsem://docs/file:///workspace/secret.md".to_string(),
                },
            ),
            rule(
                "deny-token-arg",
                McpDecisionRuleMatch::ArgumentName {
                    method: Some("tools/call".to_string()),
                    name: "token".to_string(),
                },
            ),
            rule(
                "deny-danger-query",
                McpDecisionRuleMatch::ArgumentValue {
                    method: Some("tools/call".to_string()),
                    name: "query".to_string(),
                    equals: serde_json::json!("DROP TABLE"),
                },
            ),
        ]),
    );
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });
    let cases = vec![
        (
            25,
            request_payload_with_params(
                25,
                "tools/call",
                serde_json::json!({
                    "name": "github__delete_repo",
                    "arguments": {"owner": "capsem", "repo": "prod"}
                }),
            ),
            "mcp.rule.deny-tool-name",
        ),
        (
            26,
            request_payload_with_params(
                26,
                "resources/read",
                serde_json::json!({
                    "uri": "capsem://docs/file:///workspace/secret.md"
                }),
            ),
            "mcp.rule.deny-resource-uri",
        ),
        (
            27,
            request_payload_with_params(
                27,
                "tools/call",
                serde_json::json!({
                    "name": "github__search_repos",
                    "arguments": {"query": "capsem", "token": "secret"}
                }),
            ),
            "mcp.rule.deny-token-arg",
        ),
        (
            28,
            request_payload_with_params(
                28,
                "tools/call",
                serde_json::json!({
                    "name": "github__search_repos",
                    "arguments": {"query": "DROP TABLE"}
                }),
            ),
            "mcp.rule.deny-danger-query",
        ),
    ];

    for (stream_id, payload, expected_rule) in &cases {
        write_mcp_request_frame(&mut client, *stream_id, payload.clone()).await;
        let response = read_response_frame(&mut client).await;
        assert!(
            response
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("blocked by policy")),
            "missing block for {expected_rule}"
        );
    }
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    for (stream_id, _, expected_rule) in cases {
        let request_id = stream_id.to_string();
        let call = calls
            .iter()
            .find(|call| call.request_id.as_deref() == Some(request_id.as_str()))
            .unwrap_or_else(|| panic!("blocked call {request_id} should be logged"));

        assert_eq!(call.decision, "denied", "{expected_rule}");
        assert_eq!(call.policy_mode.as_deref(), Some("audit_only"));
        assert_eq!(call.policy_action.as_deref(), Some("block"));
        assert_eq!(call.policy_rule.as_deref(), Some(expected_rule));
        assert!(call
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("request blocked by policy")));
        assert!(call.response_preview.is_none());
    }
}

#[tokio::test]
async fn framed_session_blocks_policy_v2_mcp_request_rule_and_records_fields() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.block_prod_token]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && has(arguments.prod_token)'
decision = "block"
priority = 10
reason = "Do not send production tokens to MCP tools"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_h = Arc::clone(&dispatch_count);
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts::default(),
        move |_req| {
            dispatch_count_h.fetch_add(1, Ordering::SeqCst);
            async move {
                AggregatorResult::CallResult {
                    result: serde_json::json!({"unexpected": "dispatch"}),
                }
            }
        },
    );
    *endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        31,
        request_payload_with_params(
            31,
            "tools/call",
            serde_json::json!({
                "name": "github__create_issue",
                "arguments": {
                    "issue": "prod",
                    "prod_token": "secret"
                }
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("blocked by policy")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    assert_eq!(
        dispatch_count.load(Ordering::SeqCst),
        0,
        "ask policy must not dispatch to the aggregator"
    );
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("31"))
        .expect("Policy V2 blocked request should be logged");

    assert_eq!(call.decision, "denied");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("block"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.block_prod_token")
    );
    assert_eq!(
        call.policy_reason.as_deref(),
        Some("Do not send production tokens to MCP tools")
    );
    assert!(call.response_preview.is_none());
    let preview = call
        .request_preview
        .as_deref()
        .expect("blocked request preview should be scrubbed");
    assert!(preview.contains("redacted_by_policy"));
    assert!(
        !preview.contains("secret"),
        "Policy V2 blocked request telemetry must not retain original arguments"
    );
}

#[tokio::test]
async fn framed_session_records_policy_v2_allow_rule_fields() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.allow_safe_search]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__search_repos" && arguments.query == "capsem"'
decision = "allow"
priority = 10
reason = "Safe repository search"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(&db_path, McpPolicy::new());
    *config.endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        37,
        request_payload_with_params(
            37,
            "tools/call",
            serde_json::json!({
                "name": "github__search_repos",
                "arguments": {"query": "capsem"}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(
        response.error.is_none(),
        "allow rule should preserve successful dispatch: {response:?}"
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("37"))
        .expect("Policy V2 allow request should be logged");

    assert_eq!(call.decision, "allowed");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.allow_safe_search")
    );
    assert_eq!(
        call.policy_reason.as_deref(),
        Some("Safe repository search")
    );
}

#[tokio::test]
async fn framed_session_accepts_policy_v2_mcp_request_ask_with_placeholder_confirmer() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.ask_prod_issue]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && arguments.issue == "prod"'
decision = "ask"
priority = 10
reason = "Production issue creation needs approval"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(&db_path, McpPolicy::new());
    *config.endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        32,
        request_payload_with_params(
            32,
            "tools/call",
            serde_json::json!({
                "name": "github__create_issue",
                "arguments": {
                    "issue": "prod"
                }
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response.error.is_none());
    assert_eq!(
        response
            .result
            .as_ref()
            .and_then(|result| result.get("tool"))
            .and_then(|value| value.as_str()),
        Some("github__create_issue")
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("32"))
        .expect("Policy V2 ask request should be logged");

    assert_eq!(call.decision, "allowed");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.ask_prod_issue")
    );
    assert!(call
        .response_preview
        .as_deref()
        .is_some_and(|preview| { preview.contains("github__create_issue") }));
}

#[tokio::test]
async fn framed_session_blocks_policy_v2_mcp_response_rule_and_redacts_result() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.block_secret_response]
on = "mcp.response"
if = 'method == "tools/call" && tool.name == "github__get_secret" && response.content.contains("PROD_SECRET")'
decision = "block"
priority = 10
reason = "Do not return production secrets from MCP tools"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts::default(),
        |_req| async move {
            AggregatorResult::CallResult {
                result: serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "PROD_SECRET=abc123"
                        }
                    ]
                }),
            }
        },
    );
    *endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        33,
        request_payload_with_params(
            33,
            "tools/call",
            serde_json::json!({
                "name": "github__get_secret",
                "arguments": {}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("blocked by policy")));
    assert!(
        !serde_json::to_string(&response)
            .unwrap()
            .contains("PROD_SECRET"),
        "blocked response frame must not contain the original secret"
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("33"))
        .expect("Policy V2 response block should be logged");

    assert_eq!(call.decision, "denied");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("block"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.block_secret_response")
    );
    assert!(
        call.response_preview.is_none(),
        "blocked response telemetry must not retain original secret payload"
    );
}

#[tokio::test]
async fn framed_session_rewrites_policy_v2_mcp_response_and_redacts_telemetry() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.rewrite_secret_response]
on = "mcp.response"
if = 'method == "tools/call" && tool.name == "github__get_secret" && response.content.contains("PROD_SECRET")'
decision = "rewrite"
priority = 10
reason = "Redact production secrets from MCP tool output"
rewrite_target = 'response.content =~ "PROD_SECRET=[A-Za-z0-9]+"'
rewrite_value = "PROD_SECRET=[redacted]"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts::default(),
        |_req| async move {
            AggregatorResult::CallResult {
                result: serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "PROD_SECRET=abc123"
                        }
                    ]
                }),
            }
        },
    );
    *endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        34,
        request_payload_with_params(
            34,
            "tools/call",
            serde_json::json!({
                "name": "github__get_secret",
                "arguments": {}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    let response_text = serde_json::to_string(&response).unwrap();
    assert!(
        response.error.is_none(),
        "rewrite should preserve a successful MCP response: {response:?}"
    );
    assert!(response_text.contains("PROD_SECRET=[redacted]"));
    assert!(
        !response_text.contains("PROD_SECRET=abc123"),
        "rewritten response frame must not contain the original secret"
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("34"))
        .expect("Policy V2 response rewrite should be logged");

    assert_eq!(call.decision, "allowed");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.rewrite_secret_response")
    );
    let preview = call
        .response_preview
        .as_deref()
        .expect("rewritten response preview should be recorded");
    assert!(preview.contains("PROD_SECRET=[redacted]"));
    assert!(
        !preview.contains("PROD_SECRET=abc123"),
        "rewritten response telemetry must not retain original secret payload"
    );
}

#[tokio::test]
async fn framed_session_rewrites_policy_v2_mcp_request_and_redacts_telemetry() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.rewrite_prod_token_arg]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && has(arguments.prod_token)'
decision = "rewrite"
priority = 10
reason = "Redact production token before MCP dispatch"
rewrite_target = 'arguments.prod_token =~ ".+"'
rewrite_value = "[redacted]"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let seen_args = Arc::new(Mutex::new(Vec::new()));
    let seen_args_h = Arc::clone(&seen_args);
    let endpoint =
        test_mcp_endpoint_state_with_driver(McpPolicy::new(), McpTimeouts::default(), move |req| {
            let seen_args = Arc::clone(&seen_args_h);
            async move {
                if let AggregatorMethod::CallTool { arguments, .. } = req.method {
                    seen_args
                        .lock()
                        .expect("seen args lock poisoned")
                        .push(arguments.clone());
                    AggregatorResult::CallResult {
                        result: serde_json::json!({
                            "arguments": arguments
                        }),
                    }
                } else {
                    AggregatorResult::Ok { ok: true }
                }
            }
        });
    *endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        35,
        request_payload_with_params(
            35,
            "tools/call",
            serde_json::json!({
                "name": "github__create_issue",
                "arguments": {
                    "issue": "prod",
                    "prod_token": "secret-token"
                }
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    let response_text = serde_json::to_string(&response).unwrap();
    assert!(
        response.error.is_none(),
        "unexpected response: {response:?}"
    );
    assert!(response_text.contains("[redacted]"));
    assert!(
        !response_text.contains("secret-token"),
        "rewritten request result must not echo the original secret"
    );
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    {
        let seen_args = seen_args.lock().expect("seen args lock poisoned");
        assert_eq!(seen_args.len(), 1);
        assert_eq!(seen_args[0]["prod_token"], serde_json::json!("[redacted]"));
        assert!(
            !serde_json::to_string(&seen_args[0])
                .unwrap()
                .contains("secret-token"),
            "aggregator must not receive the original secret argument"
        );
    }

    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("35"))
        .expect("Policy V2 request rewrite should be logged");

    assert_eq!(call.decision, "allowed");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.rewrite_prod_token_arg")
    );
    let preview = call
        .request_preview
        .as_deref()
        .expect("rewritten request preview should be recorded");
    assert!(preview.contains("[redacted]"));
    assert!(
        !preview.contains("secret-token"),
        "rewritten request telemetry must not retain original secret payload"
    );
}

#[tokio::test]
async fn framed_session_rewrite_policy_v2_mcp_request_error_redacts_telemetry() {
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
[policy.mcp.bad_request_rewrite_target]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && has(arguments.prod_token)'
decision = "rewrite"
priority = 10
reason = "Bad rewrite target must fail closed without leaking arguments"
rewrite_target = 'tool.name =~ ".+"'
rewrite_value = "github__redacted"
"#,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let dispatches = Arc::new(AtomicUsize::new(0));
    let dispatches_h = Arc::clone(&dispatches);
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts::default(),
        move |_req| {
            let dispatches = Arc::clone(&dispatches_h);
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                AggregatorResult::Ok { ok: true }
            }
        },
    );
    *endpoint.policy_v2.write().await = Arc::new(policy);

    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        36,
        request_payload_with_params(
            36,
            "tools/call",
            serde_json::json!({
                "name": "github__create_issue",
                "arguments": {
                    "issue": "prod",
                    "prod_token": "secret-token"
                }
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("request rewrite blocked by policy")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    assert_eq!(
        dispatches.load(Ordering::SeqCst),
        0,
        "bad rewrite targets must not dispatch to the aggregator"
    );

    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("36"))
        .expect("Policy V2 request rewrite error should be logged");

    assert_eq!(call.decision, "denied");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("policy.mcp.bad_request_rewrite_target")
    );
    let preview = call
        .request_preview
        .as_deref()
        .expect("rewrite failure request preview should be scrubbed");
    assert!(preview.contains("redacted_by_policy"));
    assert!(
        !preview.contains("secret-token"),
        "rewrite failure telemetry must not retain original secret payload"
    );
}

#[tokio::test]
async fn framed_session_times_out_non_tool_methods_and_records_terminal_error() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts {
            default_timeout: Duration::from_millis(10),
            tool_call_default: Duration::from_secs(300),
            tool_call_ceiling: Duration::from_secs(300),
        },
        |req| async move {
            if matches!(req.method, AggregatorMethod::ListResources) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            AggregatorResult::Resources { resources: vec![] }
        },
    );
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        29,
        request_payload_with_params(29, "resources/list", serde_json::json!({})),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("timed out")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("29"))
        .expect("timed-out framed MCP call should be logged");

    assert_eq!(call.method, "resources/list");
    assert_eq!(call.decision, "error");
    assert_eq!(call.policy_mode.as_deref(), Some("audit_only"));
    assert_eq!(call.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("mcp.method.resources_list")
    );
    assert!(call
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("timed out")));
}

#[tokio::test]
async fn framed_session_records_response_rule_policy_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(
        &db_path,
        policy_with_rules(vec![rule(
            "deny-public-return",
            McpDecisionRuleMatch::ReturnValue {
                method: Some("tools/call".to_string()),
                path: "classification".to_string(),
                equals: serde_json::json!("public"),
            },
        )]),
    );
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        23,
        request_payload_with_params(
            23,
            "tools/call",
            serde_json::json!({
                "name": "github__search_repos",
                "arguments": {"query": "capsem"}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("blocked by policy")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("23"))
        .expect("framed MCP call should be logged");

    assert_eq!(call.policy_mode.as_deref(), Some("audit_only"));
    assert_eq!(call.policy_action.as_deref(), Some("block"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("mcp.rule.deny-public-return")
    );
    assert!(call
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("response blocked by policy")));
    assert!(call.response_preview.is_none());
}

#[tokio::test]
async fn framed_session_blocks_policy_denied_tool_and_records_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let mut policy = McpPolicy::new();
    policy
        .tool_decisions
        .insert("github__delete_repo".to_string(), ToolDecision::Block);
    let config = test_mcp_frame_config(&db_path, policy);
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_request_frame(
        &mut client,
        24,
        request_payload_with_params(
            24,
            "tools/call",
            serde_json::json!({
                "name": "github__delete_repo",
                "arguments": {"owner": "capsem", "repo": "prod"}
            }),
        ),
    )
    .await;
    let response = read_response_frame(&mut client).await;
    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("blocked by policy")));
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    shutdown_db_writer(&config).await;

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.request_id.as_deref() == Some("24"))
        .expect("blocked framed MCP call should be logged");

    assert_eq!(call.decision, "denied");
    assert_eq!(call.policy_mode.as_deref(), Some("audit_only"));
    assert_eq!(call.policy_action.as_deref(), Some("block"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("mcp.tool.github__delete_repo")
    );
}

#[tokio::test]
async fn framed_session_rejects_stream_id_reuse_after_invalid_json() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let config = test_mcp_frame_config(&db_path, McpPolicy::new());
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&config.endpoint);
    let serve_db = Arc::clone(&config.db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_raw_mcp_frame(&mut client, 31, b"{not json".to_vec()).await;
    let invalid_response = read_response_frame(&mut client).await;
    assert_eq!(invalid_response.error.as_ref().unwrap().code, -32700);

    write_mcp_request_frame(&mut client, 31, request_payload(31, "tools/list")).await;
    client.shutdown().await.unwrap();
    drop(client);

    let err = serve_task
        .await
        .unwrap()
        .expect_err("stream id reuse after invalid JSON must close the framed session");
    assert!(
        err.2.contains("non-monotonic MCP stream id"),
        "unexpected error: {err:?}"
    );

    shutdown_db_writer(&config).await;
    let reader = DbReader::open(&db_path).unwrap();
    let calls = reader.recent_mcp_calls(10).unwrap();
    assert!(
        calls.is_empty(),
        "invalid JSON and rejected reuse must not create mcp_calls rows: {calls:?}"
    );
}

#[tokio::test]
async fn tools_call_notification_is_blocked_without_dispatch_or_argument_leak() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_h = Arc::clone(&dispatch_count);
    let endpoint = test_mcp_endpoint_state_with_driver(
        McpPolicy::new(),
        McpTimeouts::default(),
        move |_req| {
            dispatch_count_h.fetch_add(1, Ordering::SeqCst);
            async move {
                AggregatorResult::CallResult {
                    result: serde_json::json!({"unexpected": "dispatch"}),
                }
            }
        },
    );
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let serve_endpoint = Arc::clone(&endpoint);
    let serve_db = Arc::clone(&db);
    let serve_task =
        tokio::spawn(async move { serve_io(Vec::new(), server, serve_endpoint, serve_db).await });

    write_mcp_notification_frame(
        &mut client,
        serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "github__delete_repo",
                "arguments": {
                    "token": "secret-token"
                }
            }
        }))
        .unwrap(),
    )
    .await;
    client.shutdown().await.unwrap();
    drop(client);

    serve_task.await.unwrap().unwrap();
    assert_eq!(
        dispatch_count.load(Ordering::SeqCst),
        0,
        "disallowed notification must not dispatch to aggregator"
    );
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();

    let reader = DbReader::open(&db_path).unwrap();
    let call = reader
        .recent_mcp_calls(10)
        .unwrap()
        .into_iter()
        .find(|call| call.method == "tools/call")
        .expect("disallowed notification should be audited");

    assert_eq!(call.request_id, None);
    assert_eq!(call.decision, "denied");
    assert_eq!(call.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(call.policy_action.as_deref(), Some("block"));
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("mcp.notification.disallowed")
    );
    let preview = call
        .request_preview
        .as_deref()
        .expect("notification audit should retain a safe preview");
    assert!(preview.contains("redacted_by_policy"));
    assert!(
        !preview.contains("secret-token"),
        "notification bypass audit must not leak original arguments"
    );
}

#[test]
fn notification_frame_and_request_agree() {
    let frame = capsem_proto::decode_mcp_frame_body(
        &capsem_proto::encode_mcp_frame(
            0,
            MCP_FRAME_FLAG_NOTIFICATION,
            "codex",
            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .unwrap()[4..],
    )
    .unwrap();
    let req = parse_json_rpc_payload(&frame.payload).unwrap();

    assert!(validate_frame_request_pair(&frame, &req).is_ok());
}

#[test]
fn notification_stream_cannot_carry_request_id() {
    let frame = capsem_proto::decode_mcp_frame_body(
        &capsem_proto::encode_mcp_frame(
            0,
            MCP_FRAME_FLAG_NOTIFICATION,
            "codex",
            br#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#,
        )
        .unwrap()[4..],
    )
    .unwrap();
    let req = parse_json_rpc_payload(&frame.payload).unwrap();

    let err = validate_frame_request_pair(&frame, &req).unwrap_err();
    assert!(err
        .to_string()
        .contains("notification stream carried a JSON-RPC id"));
}

async fn write_mcp_request_frame(
    client: &mut tokio::io::DuplexStream,
    stream_id: u32,
    payload: Vec<u8>,
) {
    write_raw_mcp_frame(client, stream_id, payload).await;
}

async fn write_raw_mcp_frame(
    client: &mut tokio::io::DuplexStream,
    stream_id: u32,
    payload: Vec<u8>,
) {
    let frame = capsem_proto::encode_mcp_frame(stream_id, 0, "codex", &payload).unwrap();
    client.write_all(&frame).await.unwrap();
    client.flush().await.unwrap();
}

async fn write_mcp_notification_frame(client: &mut tokio::io::DuplexStream, payload: Vec<u8>) {
    let frame =
        capsem_proto::encode_mcp_frame(0, MCP_FRAME_FLAG_NOTIFICATION, "codex", &payload).unwrap();
    client.write_all(&frame).await.unwrap();
    client.flush().await.unwrap();
}

async fn read_response_frame(client: &mut tokio::io::DuplexStream) -> JsonRpcResponse {
    let frame = read_next_frame(client).await.unwrap();
    let FrameRead::Frame(frame) = frame else {
        panic!("expected response frame");
    };
    serde_json::from_slice(&frame.payload).unwrap()
}

struct TestMcpFrameConfig {
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    policy: Arc<RwLock<Arc<McpPolicy>>>,
}

async fn shutdown_db_writer(config: &Arc<TestMcpFrameConfig>) {
    let db = Arc::clone(&config.db);
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();
}

fn test_mcp_frame_config(db_path: &std::path::Path, policy: McpPolicy) -> Arc<TestMcpFrameConfig> {
    let (aggregator, mut rx) = crate::mcp::aggregator::AggregatorClient::channel(16);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let body = match req.method {
                AggregatorMethod::ListServers => AggregatorResult::Servers {
                    servers: vec![AggregatorServerStatus {
                        name: "github".to_string(),
                        url: "stdio://github".to_string(),
                        enabled: true,
                        source: "test".to_string(),
                        is_stdio: true,
                        connected: true,
                        tool_count: 1,
                        resource_count: 0,
                        prompt_count: 0,
                    }],
                },
                AggregatorMethod::ListTools => AggregatorResult::Tools { tools: vec![] },
                AggregatorMethod::ListResources => {
                    AggregatorResult::Resources { resources: vec![] }
                }
                AggregatorMethod::ListPrompts => AggregatorResult::Prompts { prompts: vec![] },
                AggregatorMethod::CallTool { name, arguments } => AggregatorResult::CallResult {
                    result: serde_json::json!({
                        "tool": name,
                        "arguments": arguments,
                        "classification": "public"
                    }),
                },
                AggregatorMethod::ReadResource { uri } => AggregatorResult::CallResult {
                    result: serde_json::json!({"uri": uri, "contents": []}),
                },
                AggregatorMethod::GetPrompt { name, arguments } => AggregatorResult::CallResult {
                    result: serde_json::json!({"name": name, "arguments": arguments}),
                },
                AggregatorMethod::Refresh { .. } => AggregatorResult::Ok { ok: true },
                AggregatorMethod::Shutdown => AggregatorResult::Ok { ok: true },
            };
            let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
        }
    });

    let db = Arc::new(DbWriter::open(db_path, 64).unwrap());
    let policy = Arc::new(RwLock::new(Arc::new(policy)));
    let endpoint = Arc::new(McpEndpointState::new(
        aggregator,
        Arc::clone(&policy),
        Arc::new(RwLock::new(Arc::new(PolicyConfig::default()))),
        Arc::new(tokio::sync::Semaphore::new(
            crate::mcp::default_inflight_cap(),
        )),
        McpTimeouts::default(),
    ));
    Arc::new(TestMcpFrameConfig {
        endpoint,
        db,
        policy,
    })
}

fn test_mcp_endpoint_state_with_timeouts(
    policy: McpPolicy,
    timeouts: McpTimeouts,
) -> Arc<McpEndpointState> {
    let (aggregator, _rx) = crate::mcp::aggregator::AggregatorClient::channel(16);
    Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(RwLock::new(Arc::new(policy))),
        Arc::new(RwLock::new(Arc::new(PolicyConfig::default()))),
        Arc::new(tokio::sync::Semaphore::new(
            crate::mcp::default_inflight_cap(),
        )),
        timeouts,
    ))
}

fn test_mcp_endpoint_state_with_driver<F, Fut>(
    policy: McpPolicy,
    timeouts: McpTimeouts,
    mut respond: F,
) -> Arc<McpEndpointState>
where
    F: FnMut(AggregatorRequest) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = AggregatorResult> + Send + 'static,
{
    let (aggregator, mut rx) = crate::mcp::aggregator::AggregatorClient::channel(16);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let id = req.id;
            let body = respond(req).await;
            let _ = resp_tx.send(AggregatorResponse { id, body });
        }
    });
    Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(RwLock::new(Arc::new(policy))),
        Arc::new(RwLock::new(Arc::new(PolicyConfig::default()))),
        Arc::new(tokio::sync::Semaphore::new(
            crate::mcp::default_inflight_cap(),
        )),
        timeouts,
    ))
}
