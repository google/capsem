use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
fn response_content_prefers_the_error_message() {
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

    assert!(response_content(&resp).is_none());
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

// These tests deliberately exercise the byte-stream boundary rather than only
// the codec. A malformed guest frame must be classified without desynchronizing
// the next response or losing the stream id needed for an error reply.

#[tokio::test]
async fn frame_reader_distinguishes_eof_valid_and_invalid_frames() {
    let (mut peer, mut reader) = tokio::io::duplex(4096);
    let valid = capsem_proto::encode_mcp_frame(7, 0, "claude", br#"{"jsonrpc":"2.0"}"#).unwrap();
    peer.write_all(&valid).await.unwrap();

    let frame = match read_next_frame(&mut reader).await.unwrap() {
        FrameRead::Frame(frame) => frame,
        other => panic!("expected valid frame, got {other:?}"),
    };
    assert_eq!(frame.stream_id, 7);
    assert_eq!(frame.process_name, "claude");

    let mut invalid_body = vec![0_u8; capsem_proto::MCP_FRAME_HEADER_LEN as usize];
    invalid_body[4..8].copy_from_slice(&11_u32.to_be_bytes());
    peer.write_all(&(invalid_body.len() as u32).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&invalid_body).await.unwrap();

    match read_next_frame(&mut reader).await.unwrap() {
        FrameRead::InvalidFrame { stream_id, error } => {
            assert_eq!(stream_id, Some(11));
            assert!(error.contains("magic"), "unexpected error: {error}");
        }
        other => panic!("expected invalid frame, got {other:?}"),
    }

    drop(peer);
    assert_eq!(read_next_frame(&mut reader).await.unwrap(), FrameRead::Eof);
}

#[tokio::test]
async fn frame_reader_rejects_impossible_lengths_and_truncated_bodies() {
    let (mut peer, mut reader) = tokio::io::duplex(128);
    peer.write_all(&1_u32.to_be_bytes()).await.unwrap();
    let error = read_next_frame(&mut reader).await.unwrap_err().to_string();
    assert!(error.contains("invalid MCP frame length"), "unexpected error: {error}");

    let (mut peer, mut reader) = tokio::io::duplex(128);
    peer.write_all(&u32::from(capsem_proto::MCP_FRAME_HEADER_LEN).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&[0_u8; 3]).await.unwrap();
    drop(peer);
    let error = read_next_frame(&mut reader).await.unwrap_err().to_string();
    assert!(error.contains("read MCP frame body"), "unexpected error: {error}");
}

#[tokio::test]
async fn response_writer_emits_a_decodable_attributed_frame() {
    let (mut writer, mut peer) = tokio::io::duplex(4096);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let response = ok_response(json!({"content": [{"type": "text", "text": "ok"}]}));

    send_response(&tx, 9, "codex", &response).await.unwrap();
    let outbound = rx.recv().await.unwrap();
    write_frame(&mut writer, &outbound).await.unwrap();

    let total_len = peer.read_u32().await.unwrap() as usize;
    let mut body = vec![0_u8; total_len];
    peer.read_exact(&mut body).await.unwrap();
    let decoded = capsem_proto::decode_mcp_frame_body(&body).unwrap();
    assert_eq!(decoded.stream_id, 9);
    assert_eq!(decoded.process_name, "codex");
    let decoded_response: JsonRpcResponse = serde_json::from_slice(&decoded.payload).unwrap();
    assert_eq!(decoded_response.result, response.result);
}

#[test]
fn json_rpc_parser_preserves_ids_and_rejects_untrusted_shapes() {
    let valid = parse_json_rpc_payload(br#"{"jsonrpc":"2.0","id":"req-7","method":"tools/list"}"#).unwrap();
    assert_eq!(valid.id, Some(json!("req-7")));

    let malformed = parse_json_rpc_payload(b"{").unwrap_err();
    assert_eq!(malformed.code, -32700);
    assert!(malformed.id.is_none());

    let wrong_version = parse_json_rpc_payload(br#"{"jsonrpc":"1.0","id":4,"method":"tools/list"}"#).unwrap_err();
    assert_eq!(wrong_version.code, -32600);
    assert_eq!(wrong_version.id, Some(json!(4)));

    let missing_method = parse_json_rpc_payload(br#"{"jsonrpc":"2.0","id":5}"#).unwrap_err();
    assert_eq!(missing_method.id, Some(json!(5)));
    assert!(missing_method.message.contains("missing JSON-RPC method"));

    let oversized = vec![b' '; MCP_JSON_RPC_MAX_BYTES + 1];
    let too_large = parse_json_rpc_payload(&oversized).unwrap_err();
    assert_eq!(too_large.code, -32600);
    assert!(too_large.message.contains("too large"));
}

#[test]
fn frame_and_json_rpc_notification_shapes_must_agree() {
    let request_frame = capsem_proto::McpFrame {
        stream_id: 1,
        flags: 0,
        process_name: "codex".to_string(),
        payload: Vec::new(),
    };
    let notification_frame = capsem_proto::McpFrame {
        stream_id: 0,
        flags: capsem_proto::MCP_FRAME_FLAG_NOTIFICATION,
        process_name: "codex".to_string(),
        payload: Vec::new(),
    };
    let with_id = request("tools/list", json!({}));
    let mut without_id = with_id.clone();
    without_id.id = None;

    assert!(validate_frame_request_pair(&request_frame, &with_id).is_ok());
    assert!(validate_frame_request_pair(&notification_frame, &without_id).is_ok());
    assert!(validate_frame_request_pair(&request_frame, &without_id)
        .unwrap_err()
        .to_string()
        .contains("missing"));
    assert!(validate_frame_request_pair(&notification_frame, &with_id)
        .unwrap_err()
        .to_string()
        .contains("carried"));
}

// -- frame body read deadline (slowloris) --

#[tokio::test(start_paused = true)]
async fn read_next_frame_times_out_when_body_stalls_after_length_prefix() {
    use tokio::io::AsyncWriteExt;
    let (client, mut server_writer) = tokio::io::duplex(1024);
    // Announce a valid frame length, then never send the body.
    let declared = (capsem_proto::MCP_FRAME_HEADER_LEN as usize) + 32;
    server_writer
        .write_all(&(declared as u32).to_be_bytes())
        .await
        .unwrap();

    let mut client = client;
    let handle = tokio::spawn(async move { read_next_frame(&mut client).await });
    tokio::time::advance(std::time::Duration::from_secs(FRAME_BODY_TIMEOUT.as_secs() + 1)).await;

    let result = handle.await.unwrap();
    assert!(
        result.is_err(),
        "a stalled frame body must time out, not hold the read loop forever"
    );
    let _ = server_writer; // keep the pipe open until here
}
