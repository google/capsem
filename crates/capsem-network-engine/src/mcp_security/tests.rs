use std::time::{Duration, UNIX_EPOCH};

use capsem_security_engine::{SecurityAction, SecurityEventSubject};

use super::*;

fn mcp_input() -> McpSecurityEventInput {
    McpSecurityEventInput {
        server_name: "local".into(),
        tool_name: "echo".into(),
        request_id: Some("8".into()),
        policy_fields: McpPolicyFields {
            policy_action: Some("allow".into()),
            policy_rule: Some("mcp.tool.local__echo".into()),
            policy_reason: Some("allowed by profile MCP policy".into()),
        },
        decision: Some("allowed".into()),
        response_error_message: None,
    }
}

#[test]
fn build_mcp_security_event_uses_canonical_subject() {
    let event = build_mcp_security_event(
        &mcp_input(),
        Some("trace_mcp_runtime".into()),
        UNIX_EPOCH + Duration::from_nanos(42),
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
fn build_mcp_resolved_security_event_records_allow_step() {
    let resolved = build_mcp_resolved_security_event(
        &mcp_input(),
        Some("trace_mcp_runtime".into()),
        UNIX_EPOCH + Duration::from_nanos(42),
    );

    assert!(matches!(resolved.final_action, SecurityAction::Continue));
    assert_eq!(resolved.steps.len(), 1);
    assert_eq!(
        resolved.steps[0].rule_id.as_deref(),
        Some("mcp.tool.local__echo")
    );
}

#[test]
fn build_mcp_resolved_security_event_maps_denied_to_block() {
    let mut input = mcp_input();
    input.policy_fields.policy_action = Some("block".into());
    input.policy_fields.policy_rule = Some("mcp.tool.local__echo.block".into());
    input.policy_fields.policy_reason = Some("blocked by profile MCP policy".into());
    input.decision = Some("denied".into());

    let resolved = build_mcp_resolved_security_event(&input, None, UNIX_EPOCH);

    assert!(matches!(resolved.final_action, SecurityAction::Block(_)));
    assert_eq!(
        resolved
            .event
            .decision
            .as_ref()
            .and_then(|decision| decision.rule.as_deref()),
        Some("mcp.tool.local__echo.block")
    );
}
