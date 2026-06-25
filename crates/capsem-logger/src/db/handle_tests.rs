use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::json;

use super::*;
use crate::events::{
    credential_reference, Decision, NetEvent, SecurityDetectionLevel, SecurityRuleAction,
    SecurityRuleEvent,
};
use crate::WriteOp;

const DB_BOUNDARY_RATIONALE: &str = "DB boundary contract: capsem-logger owns DB execution/storage; callers own query intent only. See AGENTS.md and skills/dev-testing/SKILL.md.";

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

fn disk_net_event_count(path: &std::path::Path, domain: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("open disk verifier");
    conn.query_row(
        "SELECT COUNT(*) FROM main.net_events WHERE domain = ?1",
        [domain],
        |row| row.get(0),
    )
    .expect("count disk net events")
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
async fn db_write_acknowledges_memory_before_disk_flush() {
    let p = temp_db_path("memory-before-disk-flush");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "memory-first.example",
        Decision::Allowed,
    )))
    .await
    .expect("write must acknowledge after DB-owned memory commit");

    assert_eq!(
        disk_net_event_count(&p, "memory-first.example"),
        0,
        "db.write() must not force a disk flush. S06-003 contract: acknowledged writes are memory-visible, and disk batching remains DB-owned."
    );

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
        "query() must observe the memory table immediately after write(). {DB_BOUNDARY_RATIONALE}"
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
async fn db_write_is_immediately_queryable() {
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
            (0, 0),
            "test must prove query() reads the DB-owned memory truth before disk flush"
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
        "acknowledged protocol writes must be immediately visible through db.query(), independent of disk flush timing. {DB_BOUNDARY_RATIONALE}"
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
        "acknowledged security writes must be immediately visible through db.query(), independent of disk flush timing. {DB_BOUNDARY_RATIONALE}"
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
