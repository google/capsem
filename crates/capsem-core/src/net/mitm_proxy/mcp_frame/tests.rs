use serde_json::json;

use capsem_proto::mcp_contracts::JsonRpcError;

use super::*;

fn request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params: Some(params),
        meta: None,
    }
}

#[test]
fn log_attribution_reads_tool_namespace() {
    let req = request("tools/call", json!({"name": "local__echo"}));

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "local");
    assert_eq!(tool_name.as_deref(), Some("local__echo"));
}

#[test]
fn log_attribution_reads_resource_namespace() {
    let req = request("resources/read", json!({"uri": "capsem://slowlist/doc://slow"}));

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "slowlist");
    assert!(tool_name.is_none());
}

#[test]
fn log_attribution_reads_prompt_namespace() {
    let req = request("prompts/get", json!({"name": "writer__poem"}));

    let (server_name, tool_name) = mcp_log_attribution(&req);

    assert_eq!(server_name, "writer");
    assert!(tool_name.is_none());
}

// ── Stream framing invariants ──────────────────────────────────────
//
// Stream ids arrive from the guest. StreamTracker is the only thing standing
// between a hostile guest and response confusion: reusing an in-flight id, or
// walking ids backwards, would let one request's reply be matched to another.
// Every rejection below must stay a rejection.

fn tracker() -> StreamTracker {
    StreamTracker::default()
}

#[test]
fn first_request_stream_is_accepted_and_tracked() {
    let mut t = tracker();

    assert_eq!(t.begin(1, false).unwrap(), StreamDisposition::Request);
    assert!(!t.is_empty(), "an open request stays in flight");

    t.complete(1);
    assert!(t.is_empty(), "completion releases the stream");
}

#[test]
fn request_stream_ids_must_increase() {
    let mut t = tracker();
    t.begin(5, false).unwrap();
    t.complete(5);

    // Replaying a retired id, or any id at or below the high-water mark, is
    // refused even though nothing is in flight.
    assert!(t.begin(5, false).is_err(), "replayed id must be refused");
    assert!(t.begin(4, false).is_err(), "backwards id must be refused");
    assert!(t.begin(6, false).is_ok(), "forward progress still allowed");
}

#[test]
fn duplicate_inflight_stream_id_is_refused() {
    let mut t = tracker();
    t.begin(7, false).unwrap();

    let err = t.begin(7, false).unwrap_err().to_string();
    assert!(err.contains("duplicate"), "unexpected error: {err}");
}

#[test]
fn stream_zero_is_reserved_for_notifications() {
    let mut t = tracker();

    let err = t.begin(0, false).unwrap_err().to_string();
    assert!(err.contains("reserved"), "unexpected error: {err}");
    assert_eq!(t.begin(0, true).unwrap(), StreamDisposition::Notification);
}

#[test]
fn notifications_may_not_claim_a_request_stream() {
    let mut t = tracker();

    let err = t.begin(3, true).unwrap_err().to_string();
    assert!(err.contains("stream id 0"), "unexpected error: {err}");
}

#[test]
fn notifications_never_occupy_the_inflight_set() {
    let mut t = tracker();
    t.begin(0, true).unwrap();
    t.begin(0, true).unwrap();

    assert!(t.is_empty(), "notifications are fire-and-forget");
    assert!(
        t.begin(1, false).is_ok(),
        "notifications must not advance the request high-water mark"
    );
}

// ── Method classification ──────────────────────────────────────────

#[test]
fn every_known_method_maps_to_its_label() {
    for method in [
        "initialize",
        "notifications/initialized",
        "tools/list",
        "tools/call",
        "resources/list",
        "resources/read",
        "prompts/list",
        "prompts/get",
    ] {
        let summary = interpret_mcp_method(&request(method, json!({})));
        assert_eq!(summary.kind.label(), method, "{method} round-trips through its kind");
    }
}

#[test]
fn unrecognized_method_is_classified_unknown_not_guessed() {
    let summary = interpret_mcp_method(&request("tools/callx", json!({})));
    assert_eq!(summary.kind, McpMethodKind::Unknown);
    assert_eq!(summary.kind.label(), "unknown");
    assert!(summary.tool_name.is_none());
}

#[test]
fn list_methods_attribute_to_the_wildcard_server() {
    for method in ["tools/list", "resources/list", "prompts/list"] {
        let summary = interpret_mcp_method(&request(method, json!({})));
        assert_eq!(summary.server_name.as_deref(), Some("*"), "{method}");
    }
}

#[test]
fn unnamespaced_tool_call_attributes_to_an_empty_server_not_a_guess() {
    let summary = interpret_mcp_method(&request("tools/call", json!({"name": "bare"})));

    assert_eq!(summary.tool_name.as_deref(), Some("bare"));
    assert_eq!(
        summary.server_name.as_deref(),
        Some(""),
        "an unroutable name must not be attributed to a real server"
    );
}

#[test]
fn tool_call_without_a_name_carries_no_attribution() {
    let summary = interpret_mcp_method(&request("tools/call", json!({})));

    assert_eq!(summary.kind, McpMethodKind::ToolsCall);
    assert!(summary.tool_name.is_none());
    assert!(summary.server_name.is_none());
}

#[test]
fn identical_params_hash_identically_and_differing_params_do_not() {
    let a = interpret_mcp_method(&request("tools/call", json!({"name": "s__t"})));
    let b = interpret_mcp_method(&request("tools/call", json!({"name": "s__t"})));
    let c = interpret_mcp_method(&request("tools/call", json!({"name": "s__u"})));

    assert_eq!(a.request_hash, b.request_hash);
    assert_ne!(a.request_hash, c.request_hash);
    assert!(!a.request_hash.is_empty());
}

// ── Response text extraction ───────────────────────────────────────
//
// Server responses are attacker-influenced too. Extraction walks arbitrary
// nesting for telemetry, so it must terminate, ignore non-string `text`, and
// surface errors in preference to results.

fn ok_response(result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        result: Some(result),
        error: None,
        meta: None,
    }
}

#[test]
fn response_text_collects_nested_text_fields_in_order() {
    let resp = ok_response(json!({
        "content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"},
            {"nested": {"deeper": [{"text": "third"}]}}
        ]
    }));

    assert_eq!(response_text(&resp).as_deref(), Some("first\nsecond\nthird"));
}

#[test]
fn response_text_ignores_non_string_text_fields() {
    let resp = ok_response(json!({"text": 42, "items": [{"text": null}, {"text": true}]}));

    assert!(
        response_text(&resp).is_none(),
        "only string `text` values are telemetry text"
    );
}

#[test]
fn response_text_and_content_prefer_the_error_message() {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        result: Some(json!({"text": "ignored"})),
        error: Some(JsonRpcError {
            code: -32000,
            message: "tool denied by policy".to_string(),
            data: None,
        }),
        meta: None,
    };

    assert_eq!(response_text(&resp).as_deref(), Some("tool denied by policy"));
    assert_eq!(response_content(&resp).as_deref(), Some("tool denied by policy"));
}

#[test]
fn response_without_result_or_error_yields_nothing() {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        result: None,
        error: None,
        meta: None,
    };

    assert!(response_text(&resp).is_none());
    assert!(response_content(&resp).is_none());
}

#[test]
fn deeply_nested_response_does_not_blow_the_stack() {
    let mut value = json!({"text": "bottom"});
    for _ in 0..512 {
        value = json!({"child": value});
    }

    assert_eq!(response_text(&ok_response(value)).as_deref(), Some("bottom"));
}

// ── Policy field projection ────────────────────────────────────────

#[test]
fn security_decision_projects_into_mcp_call_policy_fields() {
    let decision = SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Block,
        rule_id: Some("rule-42".to_string()),
        rule_name: Some("no-exfil".to_string()),
        reason: Some("blocked by corp policy".to_string()),
        ask_id: None,
    };

    let fields = McpCallPolicyFields::from(&decision);

    assert_eq!(fields.policy_mode.as_deref(), Some("security_event"));
    assert_eq!(fields.policy_action.as_deref(), Some("block"));
    assert_eq!(fields.policy_rule.as_deref(), Some("rule-42"));
    assert_eq!(fields.policy_reason.as_deref(), Some("blocked by corp policy"));
}
