use std::time::Duration;

use crate::mcp::policy::{McpPolicy, ToolDecision};
use crate::net::mitm_proxy::McpTimeouts;

use super::*;

static MCP_TIMEOUT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    assert_eq!(decision.action, McpPolicyAction::Block);
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
