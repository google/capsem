use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::SystemTime;

use serde_json::json;

use super::*;
use crate::events::{
    credential_reference, Decision, ModelCall, NetEvent, SecurityDetectionLevel,
    SecurityRuleAction, SecurityRuleEvent, ToolCallEntry, ToolResponseEntry,
};
use crate::WriteOp;

const DB_BOUNDARY_RATIONALE: &str = "DB boundary contract: capsem-logger owns DB execution/storage; callers own query intent only. See AGENTS.md and skills/dev-testing/SKILL.md.";
static DB_FLUSH_FAILURE_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[test]
fn db_handle_contract_names_db_ownership_and_schema_failures() {
    assert!(
        DB_HANDLE_CONTRACT.contains("caller owns query intent"),
        "DB handle docs must keep route SQL/query intent separate from DB execution ownership. {DB_BOUNDARY_RATIONALE}"
    );
    assert!(
        DB_HANDLE_CONTRACT.contains("db owns execution and storage"),
        "DB handle docs must say the logger DB object owns execution/storage mechanics. {DB_BOUNDARY_RATIONALE}"
    );
    assert!(
        DB_HANDLE_CONTRACT.contains("missing schema fails loudly"),
        "DB handle docs must preserve the no-fallback missing-schema invariant. {DB_BOUNDARY_RATIONALE}"
    );
}

fn temp_db_path(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "capsem-test-db-handle-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&p);
    let _ = std::fs::remove_file(p.with_extension("db-wal"));
    let _ = std::fs::remove_file(p.with_extension("db-shm"));
    p
}

fn make_net_event(domain: &str, decision: Decision) -> NetEvent {
    NetEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        domain: domain.to_string(),
        port: 443,
        decision,
        process_name: Some("db-handle-test".into()),
        pid: Some(7),
        method: Some("GET".into()),
        path: Some("/api".into()),
        query: None,
        status_code: Some(200),
        bytes_sent: 11,
        bytes_received: 22,
        duration_ms: 3,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        request_body_full: None,
        response_body_full: None,
        conn_type: None,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("trace-db-handle".into()),
        credential_ref: None,
    }
}

fn blake3_test_ref(value: &str) -> String {
    format!("blake3:{}", blake3::hash(value.as_bytes()).to_hex())
}

fn query_json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("DbHandle::query JSON")
}

fn disk_net_event_count(path: &std::path::Path, domain: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("open disk verifier");
    conn.query_row(
        "SELECT COUNT(*) FROM main.net_events WHERE domain = ?1",
        [domain],
        |row| row.get(0),
    )
    .expect("count disk net events")
}

fn disk_quick_check(path: &std::path::Path) -> String {
    let conn = rusqlite::Connection::open(path).expect("open disk verifier");
    conn.pragma_query_value(None, "quick_check", |row| row.get(0))
        .expect("disk quick_check")
}

fn make_correctness_net_event(credential_ref: &str) -> NetEvent {
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
        request_body_preview: Some(r#"{"prompt":"ledger"}"#.into()),
        response_body_preview: Some(r#"{"ok":true}"#.into()),
        request_body_full: Some(r#"{"prompt":"ledger","nonce":"net-before-after"}"#.into()),
        response_body_full: Some(r#"{"ok":true,"nonce":"net-response-before-after"}"#.into()),
        conn_type: Some("https".into()),
        policy_mode: Some("default".into()),
        policy_action: Some("allow".into()),
        policy_rule: Some("profiles.rules.default_http".into()),
        policy_reason: Some("default allow".into()),
        trace_id: Some("trace-correctness-1".into()),
        credential_ref: Some(credential_ref.to_string()),
    }
}

fn make_correctness_tool_emit_model_call(credential_ref: &str) -> ModelCall {
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
        request_body_preview: Some(r#"{"input":"write file"}"#.into()),
        request_body_full: Some(
            r#"{"input":"write file","nonce":"model-request-before-after"}"#.into(),
        ),
        message_id: Some("resp_correctness_1".into()),
        status_code: Some(200),
        text_content: Some("I wrote the file.".into()),
        thinking_content: Some("Need to call write_file.".into()),
        response_body_full: Some(
            r#"{"output_text":"I wrote the file.","nonce":"model-response-before-after"}"#.into(),
        ),
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

fn make_correctness_tool_response_model_call(credential_ref: &str) -> ModelCall {
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
        request_body_preview: Some(r#"{"tool_result":"write_file"}"#.into()),
        request_body_full: Some(
            r#"{"tool_result":"write_file","nonce":"model-tool-response-request-before-after"}"#
                .into(),
        ),
        message_id: Some("resp_correctness_2".into()),
        status_code: Some(200),
        text_content: Some("The tool result was accepted.".into()),
        thinking_content: Some("Need to summarize the tool result.".into()),
        response_body_full: Some(
            r#"{"output_text":"The tool result was accepted.","nonce":"model-tool-response-output"}"#
                .into(),
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
            call_id: "tool-call-correctness-1".into(),
            content_preview: Some(r#"{"path":"poem.md","bytes":6}"#.into()),
            is_error: false,
            trace_id: None,
            credential_ref: None,
        }],
    }
}

fn make_correctness_security_event(credential_ref: &str) -> SecurityRuleEvent {
    let rule_json = r#"{"name":"correctness_detect","match":"http.host == 'correctness.example'"}"#;
    let event_json = r#"{"event_type":"http.request","http":{"host":"correctness.example"},"decision":{"effective":"allow"}}"#;
    SecurityRuleEvent::new(
        1_771_100_003,
        "111111111111",
        "http.request",
        "profiles.rules.correctness_detect",
        rule_json,
        event_json,
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
            &[
                json!("111111111111"),
                json!("222222222222"),
                json!("333333333333"),
            ],
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
async fn db_handle_ready_query_write() {
    let p = temp_db_path("ready-query-write");
    let db = DbHandle::open(&p).expect("open handle");

    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "db-handle.example",
        Decision::Allowed,
    )))
    .await
    .expect("write event");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, decision, bytes_sent FROM net_events WHERE domain = ?",
            &[json!("db-handle.example")],
        )
        .await
        .expect("query ledger");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");

    assert_eq!(
        value["columns"],
        json!(["domain", "decision", "bytes_sent"])
    );
    assert_eq!(value["rows"], json!([["db-handle.example", "allowed", 11]]));
}

#[tokio::test]
async fn db_write_accepts_before_flush_and_flush_makes_visible() {
    let p = temp_db_path("memory-before-disk-flush");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "memory-first.example",
        Decision::Allowed,
    )))
    .await
    .expect("write must acknowledge after DB-owned buffer accepts the event");

    assert_eq!(
        disk_net_event_count(&p, "memory-first.example"),
        0,
        "db.write() must not force a disk flush. S08 contract: write accepts into DB-owned batching, and flush controls visibility/durability."
    );
    let buffered_raw = db
        .query(
            "SELECT domain, decision, bytes_sent FROM net_events WHERE domain = ?",
            &[json!("memory-first.example")],
        )
        .await
        .expect("query before flush");
    let buffered_value: serde_json::Value =
        serde_json::from_str(&buffered_raw).expect("query JSON");
    assert_eq!(
        buffered_value["rows"],
        json!([]),
        "write() accepts into the DB-owned batch buffer; query visibility starts at flush(). {DB_BOUNDARY_RATIONALE}"
    );

    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, decision, bytes_sent FROM net_events WHERE domain = ?",
            &[json!("memory-first.example")],
        )
        .await
        .expect("query acknowledged row from memory");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(
        value["rows"],
        json!([["memory-first.example", "allowed", 11]]),
        "query() must observe rows after the DB-owned flush barrier. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_batch_flush_persists_memory_rows_idempotently() {
    let p = temp_db_path("flush-memory-idempotent");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "flush-idempotent.example",
        Decision::Allowed,
    )))
    .await
    .expect("write must acknowledge memory row");

    assert_eq!(disk_net_event_count(&p, "flush-idempotent.example"), 0);
    db.flush_for_tests().await;
    assert_eq!(disk_net_event_count(&p, "flush-idempotent.example"), 1);
    db.flush_for_tests().await;
    assert_eq!(
        disk_net_event_count(&p, "flush-idempotent.example"),
        1,
        "flush must be idempotent; batching cannot duplicate ledger rows"
    );
}

#[tokio::test]
async fn db_correctness_db_interrupted_flush_is_transactional() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("interrupted-flush-transactional");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "flush-committed.example",
        Decision::Allowed,
    )))
    .await
    .expect("baseline write must acknowledge memory row");
    db.flush_for_tests().await;
    assert_eq!(
        disk_net_event_count(&p, "flush-committed.example"),
        1,
        "baseline row must be durably flushed before injecting failure"
    );

    db.write(WriteOp::NetEvent(make_net_event(
        "flush-interrupted.example",
        Decision::Allowed,
    )))
    .await
    .expect("interrupted write must acknowledge memory row before disk flush");
    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    db.flush_for_tests().await;

    let memory_before_failed_flush = query_json(
        &db.query(
            "SELECT domain, decision, trace_id, turn_id
             FROM net_events
             WHERE domain LIKE 'flush-%'
             ORDER BY domain",
            &[],
        )
        .await
        .expect("memory query before failed flush"),
    );
    assert_eq!(
        memory_before_failed_flush["rows"],
        json!([
            [
                "flush-committed.example",
                "allowed",
                "trace-db-handle",
                "trace-db-handle"
            ],
            [
                "flush-interrupted.example",
                "allowed",
                "trace-db-handle",
                "trace-db-handle"
            ]
        ]),
        "rows from an interrupted disk flush must remain visible in DB-owned memory. {DB_BOUNDARY_RATIONALE}"
    );

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 100);
    db.flush_for_tests().await;

    assert_eq!(
        disk_net_event_count(&p, "flush-committed.example"),
        1,
        "failed flush must not roll back previously committed disk truth"
    );
    assert_eq!(
        disk_net_event_count(&p, "flush-interrupted.example"),
        0,
        "failed flush must not expose partially copied memory rows on disk"
    );
    assert_eq!(
        disk_quick_check(&p),
        "ok",
        "failed flush must leave the disk database transactionally valid"
    );

    let memory_after_failed_flush = query_json(
        &db.query(
            "SELECT domain, decision, trace_id, turn_id
             FROM net_events
             WHERE domain LIKE 'flush-%'
             ORDER BY domain",
            &[],
        )
        .await
        .expect("memory query after failed flush"),
    );
    assert_eq!(
        memory_before_failed_flush, memory_after_failed_flush,
        "failed disk flush must not corrupt acknowledged DB-owned memory truth. {DB_BOUNDARY_RATIONALE}"
    );

    crate::writer::fail_disk_flushes_for_tests(0);
    db.flush_for_tests().await;
    assert_eq!(
        disk_net_event_count(&p, "flush-interrupted.example"),
        1,
        "clearing the injected failure must let the dirty memory row flush exactly once"
    );
    drop(db);

    let reopened = DbHandle::open(&p).expect("reopen handle");
    reopened
        .ready()
        .await
        .expect("rehydrate after recovery flush");
    let after_reopen = query_json(
        &reopened
            .query(
                "SELECT domain, decision, trace_id, turn_id
                 FROM net_events
                 WHERE domain LIKE 'flush-%'
                 ORDER BY domain",
                &[],
            )
            .await
            .expect("query after recovery reopen"),
    );
    assert_eq!(
        memory_before_failed_flush, after_reopen,
        "after recovery flush and restart, db.query() must return the same ledger truth. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_shutdown_flushes_dirty_memory_rows_to_disk() {
    let p = temp_db_path("shutdown-flushes-memory");
    {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
        db.write(WriteOp::NetEvent(make_net_event(
            "shutdown-flush.example",
            Decision::Allowed,
        )))
        .await
        .expect("write must acknowledge memory row");
        assert_eq!(disk_net_event_count(&p, "shutdown-flush.example"), 0);
    }

    assert_eq!(
        disk_net_event_count(&p, "shutdown-flush.example"),
        1,
        "dropping the DB handle must shutdown the DB object and drain dirty memory to disk"
    );
}

#[tokio::test]
async fn db_flush_rehydrate_flushed_rows_survive_reopen() {
    let p = temp_db_path("flush-rehydrate-reopen");
    {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
        db.write(WriteOp::NetEvent(make_net_event(
            "flush-rehydrate.example",
            Decision::Allowed,
        )))
        .await
        .expect("write must acknowledge memory row");
        db.flush_for_tests().await;
    }

    let db = DbHandle::open(&p).expect("reopen handle");
    db.ready().await.expect("db ready after reopen");
    let raw = db
        .query(
            "SELECT domain, decision, bytes_sent FROM net_events WHERE domain = ?",
            &[json!("flush-rehydrate.example")],
        )
        .await
        .expect("query flushed row after reopen");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(
        value["rows"],
        json!([["flush-rehydrate.example", "allowed", 11]]),
        "flushed DB-owned memory rows must survive close/reopen through the same query() contract"
    );
}

#[tokio::test]
async fn db_rehydrates_from_disk_before_ready_succeeds() {
    let p = temp_db_path("startup-rehydrate-existing-disk");
    {
        let conn = rusqlite::Connection::open(&p).expect("open disk fixture");
        crate::schema::create_tables(&conn).expect("create disk schema");
        conn.execute(
            "INSERT INTO main.net_events (
                event_id, timestamp, domain, port, decision, process_name, pid,
                method, path, query, status_code,
                bytes_sent, bytes_received, duration_ms, matched_rule,
                request_headers, response_headers, request_body_preview, response_body_preview,
                conn_type, policy_mode, policy_action, policy_rule, policy_reason,
                trace_id, turn_id, credential_ref
             )
             VALUES (
                'abcabcabcabc', '2026-06-25T00:00:00Z', 'startup-rehydrate.example', 443,
                'allowed', 'fixture', 42, 'GET', '/disk', NULL, 200,
                31, 41, 5, NULL, NULL, NULL, NULL, NULL, 'https',
                'default', 'allow', 'profiles.rules.default_http', NULL,
                'trace-startup-rehydrate', 'turn-startup-rehydrate', NULL
             )",
            [],
        )
        .expect("seed disk row before DB handle startup");
    }

    let db = DbHandle::open(&p).expect("open handle over existing disk rows");
    db.ready()
        .await
        .expect("ready() must include DB-owned disk-to-memory rehydration");
    let raw = db
        .query(
            "SELECT domain, decision, bytes_sent, bytes_received, trace_id, turn_id
             FROM net_events WHERE domain = ?",
            &[json!("startup-rehydrate.example")],
        )
        .await
        .expect("query rehydrated disk row from memory view");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(
        value["rows"],
        json!([[
            "startup-rehydrate.example",
            "allowed",
            31,
            41,
            "trace-startup-rehydrate",
            "turn-startup-rehydrate"
        ]]),
        "ready() must not succeed until existing disk rows are visible through the DB-owned memory query path"
    );
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
    let model_response =
        r#"{"output_text":"I wrote the file.","nonce":"model-response-before-after"}"#;
    let model_tool_response_request =
        r#"{"tool_result":"write_file","nonce":"model-tool-response-request-before-after"}"#;
    let model_tool_response_body =
        r#"{"output_text":"The tool result was accepted.","nonce":"model-tool-response-output"}"#;

    let before_flush = {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
        db.write(WriteOp::NetEvent(make_correctness_net_event(
            &credential_ref,
        )))
        .await
        .expect("write correctness net event");
        db.write(WriteOp::ModelCall(make_correctness_tool_emit_model_call(
            &credential_ref,
        )))
        .await
        .expect("write correctness model/tool emit event");
        db.write(WriteOp::ModelCall(
            make_correctness_tool_response_model_call(&credential_ref),
        ))
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
                    r#"{"input":"write file"}"#,
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

#[tokio::test]
async fn db_handle_query_binds_params_and_caps_rows() {
    let p = temp_db_path("query-binds-params-caps-rows");
    {
        let db = DbHandle::open(&p).expect("open handle");
        db.ready().await.expect("db ready");
    }
    {
        let mut conn = rusqlite::Connection::open(&p).expect("open query fixture");
        let tx = conn.transaction().expect("start fixture transaction");
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO net_events (timestamp, domain, decision)
                     VALUES (?1, ?2, 'allowed')",
                )
                .expect("prepare fixture insert");
            for i in 0..10_050 {
                stmt.execute(("2026-01-01T00:00:00Z", format!("bind-{i:05}.example")))
                    .expect("insert fixture row");
            }
        }
        tx.commit().expect("commit fixture rows");
    }

    let db = DbHandle::open(&p).expect("reopen handle");
    let raw = db
        .query(
            "SELECT domain, decision FROM net_events
             WHERE decision = ? AND domain LIKE ?
             ORDER BY domain",
            &[json!("allowed"), json!("bind-%.example")],
        )
        .await
        .expect("query should bind params on DB-owned worker");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    let rows = value["rows"].as_array().expect("rows array");

    assert_eq!(
        value["columns"],
        json!(["domain", "decision"]),
        "query(sql, params) must return deterministic columns. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        rows.len(),
        10_000,
        "query(sql, params) must cap route-visible output at the DB boundary. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        rows.first(),
        Some(&json!(["bind-00000.example", "allowed"]))
    );
    assert_eq!(rows.last(), Some(&json!(["bind-09999.example", "allowed"])));
}

#[tokio::test]
async fn db_handle_query_returns_exact_columns_rows() {
    let p = temp_db_path("query-exact-columns-rows");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "exact.example",
        Decision::Denied,
    )))
    .await
    .expect("write exact fixture");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, port, decision, method, path, bytes_sent, bytes_received
             FROM net_events WHERE domain = ?",
            &[json!("exact.example")],
        )
        .await
        .expect("query exact rows");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");

    assert_eq!(
        value["columns"],
        json!([
            "domain",
            "port",
            "decision",
            "method",
            "path",
            "bytes_sent",
            "bytes_received"
        ]),
        "DbHandle::query must preserve exact column order. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        value["rows"],
        json!([["exact.example", 443, "denied", "GET", "/api", 11, 22]]),
        "DbHandle::query must preserve exact row values. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_records_read_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let p = temp_db_path("query-records-read-metrics");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "metrics.example",
        Decision::Allowed,
    )))
    .await
    .expect("write metric fixture");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, decision FROM net_events WHERE domain = ?",
            &[json!("metrics.example")],
        )
        .await
        .expect("query metric rows");
    assert!(
        raw.contains("metrics.example"),
        "metric test must exercise a real DB query"
    );

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_TOTAL && matches!(value, DebugValue::Counter(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_DURATION_MS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_RESULT_ROWS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_RESULT_BYTES && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_QUERY_PARAMS_COUNT && matches!(value, DebugValue::Histogram(_))
    }));
}

#[tokio::test]
async fn db_handle_query_rejects_mutations() {
    let p = temp_db_path("query-rejects-mutations");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    let error = db
        .query("DELETE FROM net_events WHERE domain = ?", &[json!("x")])
        .await
        .expect_err("query must reject mutation SQL");
    assert!(
        error.contains("DELETE") && error.contains("not allowed"),
        "DbHandle::query must reject mutations before SQLite execution: {error}. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_query_uses_worker_not_runtime_block() {
    let p = temp_db_path("query-worker-not-runtime-block");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    let ticker = tokio::spawn(async {
        let mut ticks = 0;
        for _ in 0..100 {
            tokio::task::yield_now().await;
            ticks += 1;
        }
        ticks
    });

    let (query, ticks) = tokio::join!(
        db.query("SELECT COUNT(*) AS count FROM net_events", &[]),
        ticker
    );

    query.expect("query should complete through DB worker");
    assert_eq!(
        ticks.expect("ticker task should complete"),
        100,
        "DbHandle::query must await a DB-owned worker instead of blocking the tokio runtime. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_write_event_contract() {
    let p = temp_db_path("write-security-event-fields");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    let credential_ref = credential_reference("test", "not-a-real-secret");
    let rule_json = r#"{"name":"db_write_exact","match":"http.host == 'example.com'"}"#;
    let event_json = r#"{"event_type":"http.request","http":{"host":"example.com"}}"#;

    db.write(WriteOp::SecurityRuleEvent(
        SecurityRuleEvent::new(
            1_771_000_001,
            "abcdef123456",
            "http.request",
            "profiles.rules.db_write_exact",
            rule_json,
            event_json,
        )
        .with_rule_action(SecurityRuleAction::Block)
        .with_detection_level(SecurityDetectionLevel::High)
        .with_trace_id("trace-write-security")
        .with_turn_id("turn-write-security")
        .with_credential_ref(credential_ref.clone()),
    ))
    .await
    .expect("write(security event) must use the logger-owned writer path");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT timestamp_unix_ms, event_id, event_type, rule_id, rule_action,
                    detection_level, rule_json, event_json, trace_id, turn_id,
                    credential_ref
             FROM security_rule_events WHERE event_id = ?",
            &[json!("abcdef123456")],
        )
        .await
        .expect("query written security event");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");

    assert_eq!(
        value["rows"],
        json!([[
            1_771_000_001_i64,
            "abcdef123456",
            "http.request",
            "profiles.rules.db_write_exact",
            "block",
            "high",
            rule_json,
            event_json,
            "trace-write-security",
            "turn-write-security",
            credential_ref
        ]]),
        "db.write(event) must persist exact security ledger fields. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_write_then_query_observes_event() {
    let p = temp_db_path("write-then-query-observes-event");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "observed-write.example",
        Decision::Allowed,
    )))
    .await
    .expect("write event through DbHandle");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, decision, trace_id FROM net_events WHERE domain = ?",
            &[json!("observed-write.example")],
        )
        .await
        .expect("query written event through DbHandle");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(
        value["rows"],
        json!([["observed-write.example", "allowed", "trace-db-handle"]]),
        "DbHandle::write must be visible through DbHandle::query without route projections. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_write_flush_then_queryable() {
    let p = temp_db_path("write-immediately-queryable");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    let credential_ref = credential_reference("test", "memory-visible-secret");
    let rule_json = r#"{"name":"memory_visible","match":"http.host == 'memory.example'"}"#;
    let event_json = r#"{"event_type":"http.request","http":{"host":"memory.example"}}"#;

    db.write(WriteOp::NetEvent(make_net_event(
        "memory-visible.example",
        Decision::Allowed,
    )))
    .await
    .expect("acknowledged protocol write");
    db.write(WriteOp::SecurityRuleEvent(
        SecurityRuleEvent::new(
            1_771_000_002,
            "fedcba654321",
            "http.request",
            "profiles.rules.memory_visible",
            rule_json,
            event_json,
        )
        .with_rule_action(SecurityRuleAction::Ask)
        .with_detection_level(SecurityDetectionLevel::Medium)
        .with_trace_id("trace-memory-visible")
        .with_turn_id("turn-memory-visible")
        .with_credential_ref(credential_ref.clone()),
    ))
    .await
    .expect("acknowledged security write");
    db.flush_for_tests().await;

    {
        let conn = rusqlite::Connection::open(&p).expect("open disk verifier");
        let protocol_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM main.net_events WHERE domain = 'memory-visible.example'",
                [],
                |row| row.get(0),
            )
            .expect("count protocol disk rows after acknowledged memory write");
        let security_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM main.security_rule_events WHERE event_id = 'fedcba654321'",
                [],
                |row| row.get(0),
            )
            .expect("count security disk rows after acknowledged memory write");
        assert_eq!(
            (protocol_rows, security_rows),
            (1, 1),
            "flush() is the DB-owned visibility/durability barrier for accepted writes"
        );
    }

    let protocol_raw = db
        .query(
            "SELECT domain, decision, process_name, bytes_sent, bytes_received, trace_id
             FROM net_events WHERE domain = ?",
            &[json!("memory-visible.example")],
        )
        .await
        .expect("query acknowledged protocol row from memory");
    let protocol: serde_json::Value =
        serde_json::from_str(&protocol_raw).expect("protocol query JSON");
    assert_eq!(
        protocol["rows"],
        json!([[
            "memory-visible.example",
            "allowed",
            "db-handle-test",
            11,
            22,
            "trace-db-handle"
        ]]),
        "accepted protocol writes must be visible through db.query() after the DB-owned flush barrier. {DB_BOUNDARY_RATIONALE}"
    );

    let security_raw = db
        .query(
            "SELECT event_id, event_type, rule_id, rule_action, detection_level,
                    trace_id, turn_id, credential_ref
             FROM security_rule_events WHERE event_id = ?",
            &[json!("fedcba654321")],
        )
        .await
        .expect("query acknowledged security row from memory");
    let security: serde_json::Value =
        serde_json::from_str(&security_raw).expect("security query JSON");
    assert_eq!(
        security["rows"],
        json!([[
            "fedcba654321",
            "http.request",
            "profiles.rules.memory_visible",
            "ask",
            "medium",
            "trace-memory-visible",
            "turn-memory-visible",
            credential_ref
        ]]),
        "accepted security writes must be visible through db.query() after the DB-owned flush barrier. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_contract_ready_query_write_exactness() {
    let p = temp_db_path("contract-ready-query-write-exactness");
    let db = DbHandle::open(&p).expect("open handle");

    db.ready()
        .await
        .expect("ready() must validate schema before routes read ledgers. DB boundary contract: capsem-logger owns schema/readiness; callers must not fake empty route data.");
    db.write(WriteOp::NetEvent(make_net_event(
        "contract.example",
        Decision::Allowed,
    )))
    .await
    .expect("write(event) must persist through the logger DB path only. DB boundary contract: no caller-owned SQLite writes.");
    db.flush_for_tests().await;

    let raw = db
        .query(
            "SELECT domain, port, decision, process_name, pid, method, path, status_code,
                    bytes_sent, bytes_received, duration_ms, trace_id
             FROM net_events WHERE domain = ?",
            &[json!("contract.example")],
        )
        .await
        .expect("query(sql, params) must be the DB-owned read path. DB boundary contract: caller owns query intent, logger owns execution/storage.");
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("query() must return deterministic column/row JSON");

    assert_eq!(
        value["columns"],
        json!([
            "domain",
            "port",
            "decision",
            "process_name",
            "pid",
            "method",
            "path",
            "status_code",
            "bytes_sent",
            "bytes_received",
            "duration_ms",
            "trace_id"
        ]),
        "query() columns changed. {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        value["rows"],
        json!([[
            "contract.example",
            443,
            "allowed",
            "db-handle-test",
            7,
            "GET",
            "/api",
            200,
            11,
            22,
            3,
            "trace-db-handle"
        ]]),
        "write(event) did not persist exact route-visible fields. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_ready_valid_schema() {
    let p = temp_db_path("ready-valid-empty");
    let db = DbHandle::open(&p).expect("open handle");

    db.ready().await.expect("valid empty schema must be ready");
}

#[tokio::test]
async fn db_handle_ready_rejects_broken_schema() {
    let p = temp_db_path("ready-broken-schema");
    {
        let conn = rusqlite::Connection::open(&p).expect("open broken fixture");
        conn.execute(
            "CREATE TABLE net_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL
            )",
            [],
        )
        .expect("create intentionally broken net_events table");
    }

    let db = DbHandle::open_existing_for_tests(&p).expect("open existing broken handle");
    let error = db
        .ready()
        .await
        .expect_err("ready must reject missing route-critical columns");
    assert!(
        error.contains("net_events") && error.contains("event_id"),
        "ready error should name the broken table and missing column: {error}. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_handle_ready_preserves_turn_id_through_tool_call_migration() {
    let p = temp_db_path("ready-tool-calls-turn-id-migration");
    {
        let conn = rusqlite::Connection::open(&p).expect("open migration fixture");
        conn.execute(
            "CREATE TABLE tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                model_call_id INTEGER NOT NULL,
                provider TEXT NOT NULL,
                call_index INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments TEXT
            )",
            [],
        )
        .expect("create old tool_calls shape");
        conn.execute(
            "INSERT INTO tool_calls (
                model_call_id, provider, call_index, call_id, tool_name, arguments
            ) VALUES (7, 'test', 0, 'call_1', 'write_file', '{}')",
            [],
        )
        .expect("seed old tool call row");
    }

    let db = DbHandle::open(&p).expect("open and migrate handle");
    db.ready()
        .await
        .expect("migrated schema must satisfy readiness");
    let raw = db
        .query(
            "SELECT model_call_id, call_id, tool_name, turn_id FROM tool_calls",
            &[],
        )
        .await
        .expect("query migrated tool call");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(
        value["columns"],
        json!(["model_call_id", "call_id", "tool_name", "turn_id"])
    );
    assert_eq!(value["rows"], json!([[7, "call_1", "write_file", null]]));
}

#[tokio::test]
async fn db_handle_rejects_write_sql_and_broken_schema() {
    let p = temp_db_path("rejects-write-sql");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    let error = db
        .query("INSERT INTO net_events(domain) VALUES ('evil')", &[])
        .await
        .expect_err("write SQL must be rejected");
    assert!(
        error.contains("read-only")
            || error.contains("only SELECT")
            || error.contains("not allowed"),
        "unexpected write-SQL error: {error}. {DB_BOUNDARY_RATIONALE}"
    );

    let error = db
        .query("SELECT definitely_missing FROM net_events", &[])
        .await
        .expect_err("broken schema/query must fail loudly");
    assert!(
        error.contains("definitely_missing"),
        "unexpected broken-query error: {error}. {DB_BOUNDARY_RATIONALE}"
    );
}
