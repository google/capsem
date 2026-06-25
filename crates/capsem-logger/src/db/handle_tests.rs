use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::json;

use super::*;
use crate::events::{Decision, NetEvent};
use crate::WriteOp;

#[test]
fn db_handle_contract_names_db_ownership_and_schema_failures() {
    assert!(
        DB_HANDLE_CONTRACT.contains("caller owns query intent"),
        "DB handle docs must keep route SQL/query intent separate from DB execution ownership"
    );
    assert!(
        DB_HANDLE_CONTRACT.contains("db owns execution and storage"),
        "DB handle docs must say the logger DB object owns execution/storage mechanics"
    );
    assert!(
        DB_HANDLE_CONTRACT.contains("missing schema fails loudly"),
        "DB handle docs must preserve the no-fallback missing-schema invariant"
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
        "unexpected write-SQL error: {error}"
    );

    let error = db
        .query("SELECT definitely_missing FROM net_events", &[])
        .await
        .expect_err("broken schema/query must fail loudly");
    assert!(
        error.contains("definitely_missing"),
        "unexpected broken-query error: {error}"
    );
}
