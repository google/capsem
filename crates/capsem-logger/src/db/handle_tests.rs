use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::SystemTime;

use serde_json::json;

use super::*;
use crate::events::{
    credential_reference, Decision, ModelCall, NetEvent, SecurityDetectionLevel, SecurityRuleAction, SecurityRuleEvent,
    ToolCallEntry, ToolResponseEntry,
};
use crate::WriteOp;

const DB_BOUNDARY_RATIONALE: &str = "DB boundary contract: capsem-logger owns DB execution/storage; callers own query intent only. See AGENTS.md and skills/dev-testing/SKILL.md.";
static DB_FLUSH_FAILURE_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

mod bodies;
mod correctness;
mod external_reader;
mod query;

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

pub(super) fn temp_db_path(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("capsem-test-db-handle-{name}-{}.db", std::process::id()));
    for extension in ["db", "db-wal", "db-shm", "bodies"] {
        let _ = std::fs::remove_file(p.with_extension(extension));
    }
    p
}

pub(super) fn make_net_event(domain: &str, decision: Decision) -> NetEvent {
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
        request_body: None,
        response_body: None,
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

    assert_eq!(value["columns"], json!(["domain", "decision", "bytes_sent"]));
    assert_eq!(value["rows"], json!([["db-handle.example", "allowed", 11]]));
}

#[tokio::test]
async fn db_write_accepts_without_disk_flush_and_barrier_makes_durable() {
    let p = temp_db_path("memory-before-disk-flush");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");

    db.write(WriteOp::NetEvent(make_net_event(
        "memory-first.example",
        Decision::Allowed,
    )))
    .await
    .expect("write must acknowledge after the DB-owned queue accepts the event");

    assert_eq!(
        disk_net_event_count(&p, "memory-first.example"),
        0,
        "db.write() must not force a disk flush; the explicit barrier controls durability"
    );

    db.flush_for_tests().await;

    assert_eq!(
        disk_net_event_count(&p, "memory-first.example"),
        1,
        "the DB-owned flush barrier must make every earlier accepted write durable"
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
        "query() must observe rows after the DB-owned flush barrier. {DB_BOUNDARY_RATIONALE}"
    );
}

#[tokio::test]
async fn db_external_reader_observes_rows_flushed_by_process_writer() {
    let p = temp_db_path("external-reader-process-writer");
    let writer = crate::writer::DbWriter::open(&p, 16).expect("process-owned writer opens schema");
    let db = DbHandle::open_external_reader(&p).expect("service route external reader opens");
    db.ready()
        .await
        .expect("external reader must validate the schema before route reads");

    writer
        .write(WriteOp::SecurityRuleEvent(
            SecurityRuleEvent::new(
                1_789_000_555_000,
                "abc555def000",
                "file.import",
                "profiles.rules.default_file_import",
                r#"{"name":"default_file_import"}"#,
                r#"{"event_type":"file.import","plugin_executions":[{"plugin_id":"dummy_post_allow","stage":"postprocess","applied":true,"duration_us":31}]}"#,
            )
            .with_rule_action(SecurityRuleAction::Allow)
            .with_detection_level(SecurityDetectionLevel::Low)
            .with_trace_id("trace-external-reader")
            .with_turn_id("turn-external-reader"),
        ))
        .await;
    writer.flush().await;

    let raw = db
        .query(
            "SELECT event_id, event_type, rule_id, detection_level, trace_id, turn_id
             FROM security_rule_events WHERE event_id = ?",
            &[json!("abc555def000")],
        )
        .await
        .expect("external reader must see rows flushed by the process writer");
    let value = query_json(&raw);
    assert_eq!(
        value["rows"],
        json!([[
            "abc555def000",
            "file.import",
            "profiles.rules.default_file_import",
            "low",
            "trace-external-reader",
            "turn-external-reader"
        ]]),
        "service route DB handles read ledgers written by capsem-process; DB-owned external readers must sync disk truth before query. {DB_BOUNDARY_RATIONALE}"
    );

    let error = db
        .write(WriteOp::NetEvent(make_net_event(
            "must-not-write-from-route.example",
            Decision::Allowed,
        )))
        .await
        .expect_err("external route reader must reject writes");
    assert!(
        error.contains("read-only"),
        "external route readers must fail loudly if a caller tries to create a second writer rail: {error}"
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
    reopened.ready().await.expect("rehydrate after recovery flush");
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
                    detection_level, rule_json, trace_id, turn_id,
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
    let protocol: serde_json::Value = serde_json::from_str(&protocol_raw).expect("protocol query JSON");
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
    let security: serde_json::Value = serde_json::from_str(&security_raw).expect("security query JSON");
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
async fn external_reader_recovers_when_writer_finishes_schema_after_registration() {
    let p = temp_db_path("external-reader-partial-schema-race");
    {
        let conn = rusqlite::Connection::open(&p).expect("open fixture DB");
        crate::schema::create_tables(&conn).expect("create canonical fixture schema");
        conn.execute("DROP TABLE model_calls", [])
            .expect("simulate writer between canonical DDL statements");
    }

    let db = DbHandle::open_external_reader(&p).expect("register external reader early");
    let first_error = db
        .ready()
        .await
        .expect_err("partial disk schema must still fail loudly");
    assert!(
        first_error.contains("main.model_calls"),
        "readiness must name the genuinely missing disk table: {first_error}"
    );

    {
        let conn = rusqlite::Connection::open(&p).expect("reopen writer fixture");
        crate::schema::create_tables(&conn).expect("writer finishes canonical schema");
    }

    db.ready()
        .await
        .expect("the same external handle must reconcile after canonical DDL finishes");
    let raw = db
        .query("SELECT COUNT(*) AS count FROM model_calls", &[])
        .await
        .expect("reconciled memory view must serve the newly-created table");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("query JSON");
    assert_eq!(value["rows"], json!([[0]]));
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
    db.ready().await.expect("migrated schema must satisfy readiness");
    let raw = db
        .query("SELECT model_call_id, call_id, tool_name, turn_id FROM tool_calls", &[])
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
        error.contains("read-only") || error.contains("only SELECT") || error.contains("not allowed"),
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

// A flush barrier answered `()` whether or not the disk flush it forced
// succeeded, so `DbHandle::flush` returned Ok after a failed flush. Same-
// process readers were fine (rows live in the shared memory schema); a
// service route reading a process-owned session.db from disk was told the
// rows were there when they were not.

#[tokio::test]
async fn db_handle_flush_reports_a_failed_disk_flush() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("flush-reports-failure");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("db ready");
    db.write(WriteOp::NetEvent(make_net_event(
        "flush-reported.example",
        Decision::Allowed,
    )))
    .await
    .expect("write accepted");

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    let error = db
        .flush()
        .await
        .expect_err("a flush the disk refused must not report success");
    assert!(error.contains("flush failed"), "{error}");
    assert_eq!(disk_net_event_count(&p, "flush-reported.example"), 0);

    db.flush().await.expect("the next flush succeeds");
    assert_eq!(disk_net_event_count(&p, "flush-reported.example"), 1);
    crate::writer::fail_disk_flushes_for_tests(0);
}
