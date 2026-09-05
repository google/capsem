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
    server_writer.write_all(&(declared as u32).to_be_bytes()).await.unwrap();

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

// -- Notification frames share the in-flight bound --
//
// A notification (stream 0, no JSON-RPC id) is still dispatched to the
// aggregator when its method is a request-type method such as `tools/call`.
// It must take the same `inflight` permit a request takes, or a guest can
// fire unbounded concurrent tool calls by simply omitting the id.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use capsem_proto::mcp_aggregator::{AggregatorMethod, AggregatorResponse, AggregatorResult};

struct GatedDriver {
    started: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    release: tokio::sync::watch::Sender<bool>,
}

/// Endpoint whose aggregator holds every `tools/call` until `release` is
/// flipped, counting how many were started and how many ran at once.
fn gated_endpoint(inflight: usize, hold: bool) -> (Arc<McpEndpointState>, GatedDriver) {
    let (aggregator, mut rx) = capsem_proto::mcp_aggregator::AggregatorClient::channel(64);
    let started = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let (release, release_rx) = tokio::sync::watch::channel(!hold);
    let (started_h, max_h) = (Arc::clone(&started), Arc::clone(&max_active));
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let (started, max_active, active, mut release) = (
                Arc::clone(&started_h),
                Arc::clone(&max_h),
                Arc::clone(&active),
                release_rx.clone(),
            );
            tokio::spawn(async move {
                let body = match req.method {
                    AggregatorMethod::CallTool { .. } => {
                        started.fetch_add(1, Ordering::SeqCst);
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(now, Ordering::SeqCst);
                        let _ = release.wait_for(|released| *released).await;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        AggregatorResult::CallResult {
                            result: serde_json::json!({"ok": true}),
                        }
                    }
                    _ => AggregatorResult::Error {
                        error: "unexpected method".to_string(),
                    },
                };
                let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
            });
        }
    });
    let endpoint = Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new())))),
        Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
        Arc::new(tokio::sync::Semaphore::new(inflight)),
        super::super::McpTimeouts::default(),
    ));
    (
        endpoint,
        GatedDriver {
            started,
            max_active,
            release,
        },
    )
}

fn tool_call_notification() -> Vec<u8> {
    let payload = br#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"local__slow","arguments":{}}}"#;
    capsem_proto::encode_mcp_frame(0, capsem_proto::MCP_FRAME_FLAG_NOTIFICATION, "codex", payload).unwrap()
}

async fn wait_for_started(driver: &GatedDriver, expected: usize) {
    let started = Arc::clone(&driver.started);
    capsem_foundation::poll::poll_until(
        capsem_foundation::poll::PollOpts::new("mcp-notifications-started", Duration::from_secs(5)),
        || {
            let started = Arc::clone(&started);
            async move { (started.load(Ordering::SeqCst) >= expected).then_some(()) }
        },
    )
    .await
    .unwrap_or_else(|_| panic!("expected {expected} dispatched notifications"));
}

#[tokio::test]
async fn notification_dispatch_waits_for_an_inflight_permit() {
    let (endpoint, driver) = gated_endpoint(1, true);
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));

    for _ in 0..3 {
        guest.write_all(&tool_call_notification()).await.unwrap();
    }
    wait_for_started(&driver, 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        driver.started.load(Ordering::SeqCst),
        1,
        "with one permit the second notification must wait, not dispatch"
    );

    driver.release.send(true).unwrap();
    wait_for_started(&driver, 3).await;
    assert_eq!(driver.max_active.load(Ordering::SeqCst), 1);

    drop(guest);
    serve.await.unwrap().unwrap();
}

#[tokio::test]
async fn notification_flood_never_exceeds_the_inflight_cap() {
    let (endpoint, driver) = gated_endpoint(2, false);
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));

    for _ in 0..12 {
        guest.write_all(&tool_call_notification()).await.unwrap();
    }
    wait_for_started(&driver, 12).await;
    assert!(
        driver.max_active.load(Ordering::SeqCst) <= 2,
        "a notification flood ran {} tool calls at once past a cap of 2",
        driver.max_active.load(Ordering::SeqCst)
    );

    drop(guest);
    serve.await.unwrap().unwrap();
}

// -- The ledger and the reply -------------------------------------------------

fn tool_call_request(stream_id: u32) -> Vec<u8> {
    let payload = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"local__echo","arguments":{"text":"ping"}}}"#;
    capsem_proto::encode_mcp_frame(stream_id, 0, "codex", payload).unwrap()
}

async fn read_reply(guest: &mut tokio::io::DuplexStream) -> JsonRpcResponse {
    let total_len = guest.read_u32().await.unwrap() as usize;
    let mut body = vec![0_u8; total_len];
    guest.read_exact(&mut body).await.unwrap();
    let decoded = capsem_proto::decode_mcp_frame_body(&body).unwrap();
    serde_json::from_slice(&decoded.payload).unwrap()
}

fn count(reader: &capsem_logger::DbReader, table: &str) -> i64 {
    let rows: serde_json::Value =
        serde_json::from_str(&reader.query_raw(&format!("SELECT COUNT(*) FROM {table}")).unwrap()).unwrap();
    rows["rows"][0][0].as_i64().unwrap()
}

/// Endpoint whose rules match every MCP tool call, so a rule-ledger row is
/// owed for each one.
fn endpoint_with_matching_rule() -> Arc<McpEndpointState> {
    let (aggregator, mut rx) = capsem_proto::mcp_aggregator::AggregatorClient::channel(8);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let body = AggregatorResult::CallResult {
                result: serde_json::json!({"content": [{"type": "text", "text": "pong"}]}),
            };
            let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
        }
    });
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.every_tool_call]
        name = "every_tool_call"
        action = "allow"
        detection_level = "informational"
        match = 'mcp.method == "tools/call"'
        "#,
    )
    .unwrap();
    let rules =
        SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User).unwrap();
    Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
        Arc::new(tokio::sync::Semaphore::new(4)),
        super::super::McpTimeouts::default(),
    ))
}

#[tokio::test]
async fn completing_mcp_logging_then_shutting_down_preserves_rule_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = endpoint_with_matching_rule();
    let req = request("tools/call", json!({"name":"local__echo","arguments":{"text":"ping"}}));
    let response = ok_response(json!({"content":[{"type":"text","text":"pong"}]}));
    let logged = log_mcp_call_with_policy(
        Arc::clone(&db),
        &endpoint.security_rules,
        &req,
        &response,
        "codex",
        1,
        McpCallPolicyFields::default(),
    )
    .await;
    assert!(logged.event_id.is_some());
    db.shutdown_blocking();
    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    assert_eq!(count(&reader, "tool_calls"), 1);
    assert_eq!(
        count(&reader, "security_rule_events"),
        1,
        "request completion must own the matching rule row"
    );
}

/// Both the call and its matching rule rows are accepted before the reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_call_and_rule_rows_precede_the_reply() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = endpoint_with_matching_rule();
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, Arc::clone(&db)));

    guest.write_all(&tool_call_request(1)).await.unwrap();
    let reply = read_reply(&mut guest).await;
    assert!(reply.error.is_none(), "{reply:?}");

    // Flush what the writer has accepted so far: the call row is there.
    db.flush().await;
    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    assert_eq!(
        count(&reader, "tool_calls"),
        1,
        "the call row was accepted before the reply"
    );

    assert_eq!(count(&reader, "security_rule_events"), 1);

    drop(guest);
    serve.await.unwrap().unwrap();
}
