use std::time::Duration;

use capsem_logger::DbWriter;
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
    SecurityEventSubject,
};

use crate::mcp::policy::{McpPolicy, ToolDecision};
use crate::net::mitm_proxy::McpTimeouts;

use super::*;

static MCP_TIMEOUT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn same_millisecond_mcp_events_keep_distinct_security_ids() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"filesystem__read_file","arguments":{"path":"README.md"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let first = build_mcp_security_event_from_request(
        "codex",
        &req,
        &summary,
        Some("trace_mcp".into()),
        std::time::UNIX_EPOCH + Duration::from_millis(42),
    )
    .common
    .event_id;
    let second = build_mcp_security_event_from_request(
        "codex",
        &req,
        &summary,
        Some("trace_mcp".into()),
        std::time::UNIX_EPOCH + Duration::from_millis(42) + Duration::from_nanos(1),
    )
    .common
    .event_id;

    assert_ne!(first, second);
}

fn restore_env(key: &str, value: Option<String>) {
    // SAFETY: callers hold MCP_TIMEOUT_ENV_LOCK because environment variables
    // are process-global and Rust tests run concurrently.
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
    assert_eq!(decision.action, McpEnforcementAction::Block);
    assert_eq!(decision.rule, "mcp.tool.github__delete_repo");
    assert!(decision.reason.contains("block"));
}

#[test]
fn mcp_decision_request_captures_tool_call_shape_without_arguments() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"owner":"capsem","token":"secret"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let decision_request = McpDecisionRequest::from_request("codex", &req, &summary);

    assert_eq!(decision_request.method, "tools/call");
    assert_eq!(
        decision_request.tool_name.as_deref(),
        Some("github__create_issue")
    );
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
fn build_mcp_security_event_from_request_uses_canonical_mcp_subject() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"local__echo","arguments":{"text":"hi"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let event = build_mcp_security_event_from_request(
        "codex",
        &req,
        &summary,
        Some("trace_mcp_runtime".into()),
        std::time::UNIX_EPOCH + Duration::from_nanos(42),
    );

    assert_eq!(event.common.event_type, "mcp.request");
    assert_eq!(event.common.trace_id.as_deref(), Some("trace_mcp_runtime"));
    assert_eq!(event.common.tool_call_id.as_deref(), Some("8"));
    match event.subject {
        SecurityEventSubject::Mcp(subject) => {
            assert_eq!(subject.server_id, "local");
            assert_eq!(subject.tool_name, "echo");
        }
        other => panic!("expected MCP subject, got {other:?}"),
    }
}

#[test]
fn mcp_stage_metric_labels_are_bounded() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"local__echo","arguments":{"text":"hi"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);

    assert_eq!(mcp_method_kind_label("tools/call"), "tools/call");
    assert_eq!(mcp_method_kind_label("bogus/method"), "unknown");
    assert_eq!(mcp_tool_kind_from_summary(&summary), "local_echo");
    assert_eq!(
        mcp_tool_kind_from_name(Some("local__snapshots_list")),
        "local_snapshot"
    );
    assert_eq!(
        mcp_tool_kind_from_name(Some("local__http_headers")),
        "local_http"
    );
    assert_eq!(mcp_tool_kind_from_name(Some("github__issue")), "external");
    assert_eq!(mcp_tool_kind_from_name(None), "none");
}

#[test]
fn runtime_mcp_block_projects_to_pre_dispatch_policy_decision() {
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"local__echo","arguments":{"text":"hi"}}}"#,
    )
    .unwrap();
    let summary = interpret_mcp_method(&req);
    let event = build_mcp_security_event_from_request(
        "codex",
        &req,
        &summary,
        Some("trace_mcp_runtime".into()),
        std::time::UNIX_EPOCH + Duration::from_nanos(43),
    );
    let evaluator = CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
        id: "runtime.block-mcp".into(),
        pack_id: Some("runtime-benchmark".into()),
        condition: "mcp.request.server_id == 'local' && mcp.request.tool_name == 'echo'".into(),
        decision: SecurityDecisionAction::Block,
        reason: Some("blocked MCP benchmark tool".into()),
        mutations: Vec::new(),
    }])
    .unwrap();
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(evaluator));

    let result = engine.evaluate(event).unwrap();
    assert!(!mcp_security_result_allows_dispatch(&result));

    let decision = mcp_policy_decision_from_security_result(&result, "fallback");
    assert_eq!(decision.mode, McpPolicyMode::Enforce);
    assert_eq!(decision.action, McpEnforcementAction::Block);
    assert_eq!(decision.rule, "runtime.block-mcp");
    assert_eq!(decision.reason, "blocked MCP benchmark tool");
}

#[tokio::test]
async fn log_mcp_call_writes_canonical_security_event() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    let recorder = DebuggingRecorder::new();
    let snapshotter: Snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(DbWriter::open(&dir.path().join("session.db"), 64).unwrap());
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"github__create_issue","arguments":{"owner":"capsem"}}}"#,
    )
    .unwrap();
    let resp = JsonRpcResponse::ok(
        req.id.clone(),
        serde_json::json!({"content":[{"type":"text","text":"created"}]}),
    );
    let decision = McpEnforcementDecision {
        mode: McpPolicyMode::Enforce,
        action: McpEnforcementAction::Allow,
        rule: "mcp.tool.github__create_issue".into(),
        reason: "allowed by profile MCP policy".into(),
        rewrite_target: None,
        rewrite_value: None,
        policy_rule_name: None,
    };

    log_mcp_call_with_policy(
        &db,
        &req,
        &resp,
        "codex",
        12,
        McpCallEnforcementFields::from(&decision),
        None,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reader = db.reader().unwrap();
    let security = reader
        .query_raw(
            "SELECT event_family, event_type, final_action, steps.rule_id \
             FROM security_events se \
             LEFT JOIN security_event_steps steps ON steps.event_id = se.event_id",
        )
        .unwrap();
    assert!(security.contains("mcp"));
    assert!(security.contains("mcp.request"));
    assert!(security.contains("continue"));
    assert!(security.contains("mcp.tool.github__create_issue"));

    let telemetry_metric_present =
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .any(|(key, _, _, value)| {
                key.key().name() == metrics::MCP_STAGE_DURATION_MS
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "stage" && label.value() == "telemetry_enqueue")
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "method_kind" && label.value() == "tools/call")
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "tool_kind" && label.value() == "external")
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "result" && label.value() == "ok")
                    && matches!(value, DebugValue::Histogram(_))
            });
    assert!(
        telemetry_metric_present,
        "MCP telemetry enqueue histogram should be recorded"
    );
}

#[tokio::test]
async fn log_mcp_call_writes_blocked_security_event() {
    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(DbWriter::open(&dir.path().join("session.db"), 64).unwrap());
    let req = parse_json_rpc_payload(
        br#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"github__delete_repo","arguments":{"owner":"capsem"}}}"#,
    )
    .unwrap();
    let decision = McpEnforcementDecision {
        mode: McpPolicyMode::Enforce,
        action: McpEnforcementAction::Block,
        rule: "mcp.tool.github__delete_repo".into(),
        reason: "blocked by profile MCP policy".into(),
        rewrite_target: None,
        rewrite_value: None,
        policy_rule_name: None,
    };
    let resp = policy_blocked_response(req.id.clone(), "request", &decision);

    log_mcp_call_with_policy(
        &db,
        &req,
        &resp,
        "codex",
        0,
        McpCallEnforcementFields::from(&decision),
        None,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reader = db.reader().unwrap();
    let security = reader
        .query_raw(
            "SELECT event_family, event_type, final_action, steps.rule_id \
             FROM security_events se \
             LEFT JOIN security_event_steps steps ON steps.event_id = se.event_id",
        )
        .unwrap();
    assert!(security.contains("mcp"));
    assert!(security.contains("mcp.request"));
    assert!(security.contains("block"));
    assert!(security.contains("mcp.tool.github__delete_repo"));
}
