//! One session written, flushed, reopened, and read back field by field.
//!
//! Everything else in this directory checks one behaviour. This checks that a
//! whole session survives the round trip exactly: the same rows, the same
//! correlation ids, the same archived bodies, before the flush and after a
//! restart. The fixture builders live here because the snapshot is what they
//! exist for; the body tests borrow them.

use super::*;

/// The payload a rule match archives, named here because the correctness
/// snapshot asserts the archived bytes as well as the row.
pub(super) const CORRECTNESS_SECURITY_PAYLOAD: &str =
    r#"{"event_type":"http.request","http":{"host":"correctness.example"},"decision":{"effective":"allow"}}"#;

pub(super) fn make_correctness_net_event(credential_ref: &str) -> NetEvent {
    NetEvent {
        event_id: Some("111111111111".into()),
        timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_771_100_001),
        domain: "correctness.example".into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: Some("curl".into()),
        pid: Some(101),
        method: Some("POST".into()),
        path: Some("/v1/test".into()),
        query: Some("mode=exact".into()),
        status_code: Some(200),
        bytes_sent: 77,
        bytes_received: 88,
        duration_ms: 9,
        matched_rule: Some("profiles.rules.default_http".into()),
        request_headers: Some("content-type: application/json".into()),
        response_headers: Some("content-type: application/json".into()),
        request_body: Some(r#"{"prompt":"ledger","nonce":"net-before-after"}"#.into()),
        response_body: Some(r#"{"ok":true,"nonce":"net-response-before-after"}"#.into()),
        conn_type: Some("https".into()),
        policy_mode: Some("default".into()),
        policy_action: Some("allow".into()),
        policy_rule: Some("profiles.rules.default_http".into()),
        policy_reason: Some("default allow".into()),
        trace_id: Some("trace-correctness-1".into()),
        credential_ref: Some(credential_ref.to_string()),
    }
}

pub(super) fn make_correctness_tool_emit_model_call(credential_ref: &str) -> ModelCall {
    let mut usage_details = BTreeMap::new();
    usage_details.insert("thinking".into(), 3);
    ModelCall {
        event_id: Some("222222222222".into()),
        timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_771_100_002),
        provider: "openai".into(),
        protocol: Some("openai".into()),
        model: Some("gpt-ledger-test".into()),
        process_name: Some("codex".into()),
        pid: Some(202),
        method: "POST".into(),
        path: "/v1/responses".into(),
        stream: false,
        system_prompt_preview: Some("ledger system".into()),
        messages_count: 1,
        tools_count: 1,
        request_bytes: 123,
        request_body: Some(r#"{"input":"write file","nonce":"model-request-before-after"}"#.into()),
        message_id: Some("resp_correctness_1".into()),
        status_code: Some(200),
        text_content: Some("I wrote the file.".into()),
        thinking_content: Some("Need to call write_file.".into()),
        response_body: Some(r#"{"output_text":"I wrote the file.","nonce":"model-response-before-after"}"#.into()),
        stop_reason: Some("tool_use".into()),
        input_tokens: Some(17),
        output_tokens: Some(19),
        usage_details,
        duration_ms: 44,
        response_bytes: 456,
        estimated_cost_usd: 0.00012,
        trace_id: Some("trace-correctness-1".into()),
        credential_ref: Some(credential_ref.to_string()),
        tool_calls: vec![ToolCallEntry {
            event_id: None,
            call_index: 0,
            call_id: "tool-call-correctness-1".into(),
            tool_name: "write_file".into(),
            arguments: Some(r#"{"path":"poem.md","content":"ledger"}"#.into()),
            origin: "native".into(),
            trace_id: None,
        }],
        tool_responses: Vec::new(),
    }
}

pub(super) fn make_correctness_tool_response_model_call(credential_ref: &str) -> ModelCall {
    ModelCall {
        event_id: Some("333333333333".into()),
        timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_771_100_004),
        provider: "openai".into(),
        protocol: Some("openai".into()),
        model: Some("gpt-ledger-test".into()),
        process_name: Some("codex".into()),
        pid: Some(202),
        method: "POST".into(),
        path: "/v1/responses".into(),
        stream: false,
        system_prompt_preview: Some("ledger system".into()),
        messages_count: 2,
        tools_count: 0,
        request_bytes: 111,
        request_body: Some(r#"{"tool_result":"write_file","nonce":"model-tool-response-request-before-after"}"#.into()),
        message_id: Some("resp_correctness_2".into()),
        status_code: Some(200),
        text_content: Some("The tool result was accepted.".into()),
        thinking_content: Some("Need to summarize the tool result.".into()),
        response_body: Some(
            r#"{"output_text":"The tool result was accepted.","nonce":"model-tool-response-output"}"#.into(),
        ),
        stop_reason: Some("end_turn".into()),
        input_tokens: Some(23),
        output_tokens: Some(29),
        usage_details: BTreeMap::new(),
        duration_ms: 55,
        response_bytes: 333,
        estimated_cost_usd: 0.00014,
        trace_id: Some("trace-correctness-1".into()),
        credential_ref: Some(credential_ref.to_string()),
        tool_calls: Vec::new(),
        tool_responses: vec![ToolResponseEntry {
            event_id: None,
            call_id: "tool-call-correctness-1".into(),
            content_preview: Some(r#"{"path":"poem.md","bytes":6}"#.into()),
            is_error: false,
            trace_id: None,
            credential_ref: None,
        }],
    }
}

pub(super) fn make_correctness_security_event(credential_ref: &str) -> SecurityRuleEvent {
    let rule_json = r#"{"name":"correctness_detect","match":"http.host == 'correctness.example'"}"#;
    SecurityRuleEvent::new(
        1_771_100_003,
        "111111111111",
        "http.request",
        "profiles.rules.correctness_detect",
        rule_json,
        CORRECTNESS_SECURITY_PAYLOAD,
    )
    .with_rule_action(SecurityRuleAction::Allow)
    .with_detection_level(SecurityDetectionLevel::Informational)
    .with_trace_id("trace-correctness-1")
    .with_turn_id("trace-correctness-1")
    .with_credential_ref(credential_ref.to_string())
}

async fn correctness_snapshot(db: &DbHandle) -> serde_json::Value {
    let net_security = query_json(
        &db.query(
            "SELECT n.event_id, n.domain, n.decision, n.trace_id, n.turn_id,
                    n.credential_ref, n.policy_rule, s.rule_id, s.rule_action,
                    s.detection_level, s.credential_ref
             FROM net_events n
             JOIN security_rule_events s ON s.event_id = n.event_id
             WHERE n.event_id = ?
             ORDER BY n.event_id, s.rule_id",
            &[json!("111111111111")],
        )
        .await
        .expect("query correctness net/security rows"),
    );
    let model_tool = query_json(
        &db.query(
            "SELECT emit.event_id, consume.event_id, emit.provider, emit.protocol,
                    emit.model, emit.trace_id, emit.turn_id, emit.credential_ref,
                    emit.input_tokens, emit.output_tokens,
                    tc.call_id, tc.tool_name, tc.arguments, tc.origin, tc.trace_id,
                    tc.turn_id, tc.credential_ref,
                    tr.call_id, tr.content_preview, tr.trace_id, tr.turn_id,
                    tr.credential_ref
             FROM model_calls emit
             JOIN tool_calls tc ON tc.model_call_id = emit.id
             JOIN tool_responses tr ON tr.call_id = tc.call_id AND tr.turn_id = tc.turn_id
             JOIN model_calls consume ON consume.id = tr.model_call_id
             WHERE emit.event_id = ?
             ORDER BY tc.call_id",
            &[json!("222222222222")],
        )
        .await
        .expect("query correctness model/tool rows"),
    );
    let model_items = query_json(
        &db.query(
            "SELECT m.event_id, mi.kind, mi.item_index, mi.call_id, mi.tool_name, mi.arguments, mi.content,
                    mi.trace_id, mi.turn_id, mi.credential_ref
             FROM model_items mi
             JOIN model_calls m ON m.id = mi.model_call_id
             WHERE m.event_id IN (?, ?)
             ORDER BY m.event_id, mi.item_index, mi.kind",
            &[json!("222222222222"), json!("333333333333")],
        )
        .await
        .expect("query correctness model items"),
    );
    let blobs = query_json(
        &db.query(
            "SELECT event_id, event_type, source_table, direction, content_type,
                    original_bytes, stored_bytes, truncated, body_hash, trace_id, turn_id
             FROM event_body_blobs
             WHERE event_id IN (?, ?, ?)
             ORDER BY event_id, direction",
            &[json!("111111111111"), json!("222222222222"), json!("333333333333")],
        )
        .await
        .expect("query correctness body blobs"),
    );
    json!({
        "net_security": net_security,
        "model_tool": model_tool,
        "model_items": model_items,
        "blobs": blobs,
    })
}

#[tokio::test]
async fn db_correctness_db_query_exact_after_flush_and_restart() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("correctness-before-after-reopen");
    let credential_ref = credential_reference("openai", "this_is_not_a_real_key");
    let net_request = r#"{"prompt":"ledger","nonce":"net-before-after"}"#;
    let net_response = r#"{"ok":true,"nonce":"net-response-before-after"}"#;
    let model_request = r#"{"input":"write file","nonce":"model-request-before-after"}"#;
    let model_response = r#"{"output_text":"I wrote the file.","nonce":"model-response-before-after"}"#;
    let model_tool_response_request =
        r#"{"tool_result":"write_file","nonce":"model-tool-response-request-before-after"}"#;
    let model_tool_response_body =
        r#"{"output_text":"The tool result was accepted.","nonce":"model-tool-response-output"}"#;

    let before_flush = {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
        db.write(WriteOp::NetEvent(make_correctness_net_event(&credential_ref)))
            .await
            .expect("write correctness net event");
        db.write(WriteOp::ModelCall(make_correctness_tool_emit_model_call(
            &credential_ref,
        )))
        .await
        .expect("write correctness model/tool emit event");
        db.write(WriteOp::ModelCall(make_correctness_tool_response_model_call(
            &credential_ref,
        )))
        .await
        .expect("write correctness model/tool response event");
        db.write(WriteOp::SecurityRuleEvent(make_correctness_security_event(
            &credential_ref,
        )))
        .await
        .expect("write correctness security event");
        db.flush_for_tests().await;

        let snapshot = correctness_snapshot(&db).await;

        assert_eq!(
            snapshot["net_security"]["rows"],
            json!([[
                "111111111111",
                "correctness.example",
                "allowed",
                "trace-correctness-1",
                "trace-correctness-1",
                credential_ref,
                "profiles.rules.default_http",
                "profiles.rules.correctness_detect",
                "allow",
                "informational",
                credential_ref
            ]]),
            "after the DB-owned flush barrier, db.query() must see exact joined protocol/security truth. {DB_BOUNDARY_RATIONALE}"
        );
        assert_eq!(
            snapshot["model_tool"]["rows"],
            json!([[
                "222222222222",
                "333333333333",
                "openai",
                "openai",
                "gpt-ledger-test",
                "trace-correctness-1",
                "trace-correctness-1",
                credential_ref,
                17,
                19,
                "tool-call-correctness-1",
                "write_file",
                r#"{"path":"poem.md","content":"ledger"}"#,
                "native",
                "trace-correctness-1",
                "trace-correctness-1",
                credential_ref,
                "tool-call-correctness-1",
                r#"{"path":"poem.md","bytes":6}"#,
                "trace-correctness-1",
                "trace-correctness-1",
                credential_ref
            ]]),
            "before flush, one model_call_id must own the tool call and response with the same tool_call_id. {DB_BOUNDARY_RATIONALE}"
        );
        assert_eq!(
            snapshot["model_items"]["rows"],
            json!([
                [
                    "222222222222",
                    "request",
                    1,
                    "",
                    null,
                    null,
                    model_request,
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "222222222222",
                    "reasoning",
                    2,
                    "",
                    null,
                    null,
                    "Need to call write_file.",
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "222222222222",
                    "response",
                    3,
                    "",
                    null,
                    null,
                    "I wrote the file.",
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "222222222222",
                    "tool_call",
                    4,
                    "tool-call-correctness-1",
                    "write_file",
                    r#"{"path":"poem.md","content":"ledger"}"#,
                    r#"{"path":"poem.md","content":"ledger"}"#,
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "333333333333",
                    "reasoning",
                    1,
                    "",
                    null,
                    null,
                    "Need to summarize the tool result.",
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "333333333333",
                    "response",
                    2,
                    "",
                    null,
                    null,
                    "The tool result was accepted.",
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ],
                [
                    "333333333333",
                    "tool_response",
                    3,
                    "tool-call-correctness-1",
                    null,
                    null,
                    r#"{"path":"poem.md","bytes":6}"#,
                    "trace-correctness-1",
                    "trace-correctness-1",
                    credential_ref
                ]
            ]),
            "model_items must preserve ordered request/reasoning/response/tool_call/tool_response truth before flush. {DB_BOUNDARY_RATIONALE}"
        );
        assert_eq!(
            snapshot["blobs"]["rows"],
            json!([
                [
                    "111111111111",
                    "security.rule",
                    "security_rule_events",
                    "payload",
                    "application/json",
                    CORRECTNESS_SECURITY_PAYLOAD.len(),
                    CORRECTNESS_SECURITY_PAYLOAD.len(),
                    0,
                    blake3_test_ref(CORRECTNESS_SECURITY_PAYLOAD),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "111111111111",
                    "http.request",
                    "net_events",
                    "request",
                    "application/json",
                    net_request.len(),
                    net_request.len(),
                    0,
                    blake3_test_ref(net_request),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "111111111111",
                    "http.request",
                    "net_events",
                    "response",
                    "application/json",
                    net_response.len(),
                    net_response.len(),
                    0,
                    blake3_test_ref(net_response),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "222222222222",
                    "model.call",
                    "model_calls",
                    "request",
                    "application/json",
                    model_request.len(),
                    model_request.len(),
                    0,
                    blake3_test_ref(model_request),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "222222222222",
                    "model.call",
                    "model_calls",
                    "response",
                    null,
                    model_response.len(),
                    model_response.len(),
                    0,
                    blake3_test_ref(model_response),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "333333333333",
                    "model.call",
                    "model_calls",
                    "request",
                    "application/json",
                    model_tool_response_request.len(),
                    model_tool_response_request.len(),
                    0,
                    blake3_test_ref(model_tool_response_request),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ],
                [
                    "333333333333",
                    "model.call",
                    "model_calls",
                    "response",
                    null,
                    model_tool_response_body.len(),
                    model_tool_response_body.len(),
                    0,
                    blake3_test_ref(model_tool_response_body),
                    "trace-correctness-1",
                    "trace-correctness-1"
                ]
            ]),
            "body blob references must be exact and joinable by event_id/hash before flush. {DB_BOUNDARY_RATIONALE}"
        );

        db.flush_for_tests().await;
        let after_flush = correctness_snapshot(&db).await;
        assert_eq!(
            snapshot, after_flush,
            "the same db.query() results must match exactly before and after DB-owned flush. {DB_BOUNDARY_RATIONALE}"
        );
        snapshot
    };

    let db = DbHandle::open(&p).expect("reopen correctness DB handle");
    db.ready()
        .await
        .expect("ready must rehydrate flushed rows before route reads");
    let after_reopen = correctness_snapshot(&db).await;
    assert_eq!(
        before_flush, after_reopen,
        "the same db.query() results must match exactly after restart/rehydration. {DB_BOUNDARY_RATIONALE}"
    );
}
