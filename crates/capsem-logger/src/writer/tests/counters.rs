//! Where the writer's counts land relative to the rows they count.

use super::*;
use crate::counters::fixtures::{exec, exec_done};
use capsem_telemetry::db::DB_WRITE_OPS_TOTAL;

/// A ledger file with the writer's schemas, and nothing else running on it.
fn ledger(path: &Path) -> (Connection, BodyArchive, LedgerTally) {
    let conn = Connection::open(path).unwrap();
    schema::apply_pragmas(&conn).unwrap();
    schema::create_tables(&conn).unwrap();
    schema::create_memory_tables(&conn, &schema::memory_uri_for_path(path)).unwrap();
    schema::seed_memory_sequences(&conn, schema::hot_ledger_tables()).unwrap();
    let tally = LedgerTally::restored(crate::counters::load(&conn).unwrap());
    (conn, BodyArchive::disabled(SystemTime::now), tally)
}

fn on_disk(path: &Path) -> (LedgerCounters, i64) {
    let disk = Connection::open(path).unwrap();
    let completed = disk
        .query_row("SELECT COUNT(exit_code) FROM main.exec_events", [], |row| row.get(0))
        .unwrap();
    (crate::counters::load(&disk).unwrap(), completed)
}

#[test]
fn completing_a_flushed_exec_commits_its_count_with_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let (conn, mut bodies, mut tally) = ledger(&path);

    execute_memory_batch(&conn, &[exec(3)], &mut bodies, 0, &mut tally).unwrap();
    flush_dirty_tables_to_disk(
        &conn,
        &mut BTreeSet::from(["exec_events"]),
        &mut schema::MemoryFlushWatermarks::new(),
        None,
        &mut bodies,
        tally.counters(),
    )
    .unwrap();
    assert_eq!(on_disk(&path).0.exec.started, 1);

    // The start row is on disk now, so the completion updates it there, and
    // that update is durable the moment this batch commits -- no flush follows.
    execute_memory_batch(&conn, &[exec_done(3)], &mut bodies, 0, &mut tally).unwrap();
    let (snapshot, completed_rows) = on_disk(&path);
    assert_eq!(completed_rows, 1);
    assert_eq!(
        snapshot.exec.completed, 1,
        "a crash here must not leave the row uncounted"
    );
    assert_eq!(snapshot, *tally.counters());

    // A duplicate completion rewrites the row and counts nothing.
    execute_memory_batch(&conn, &[exec_done(3)], &mut bodies, 0, &mut tally).unwrap();
    assert_eq!(tally.counters().exec.completed, 1);
}

#[test]
fn a_rejected_op_is_not_counted_and_its_batch_neighbours_are() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let (conn, mut bodies, mut tally) = ledger(&path);
    // "observed" fails the substitution outcome CHECK and takes its batch down.
    let refused = crate::counters::fixtures::substitution(
        &format!("credential:blake3:{:064x}", 1),
        "observed",
        None,
        1_700_000_000.0,
    );
    let batch = [exec(1), refused, exec(2)];
    assert!(execute_memory_batch(&conn, &batch, &mut bodies, 0, &mut tally).is_err());
    assert_eq!(
        tally.counters(),
        &LedgerCounters::default(),
        "a rolled-back batch counts nothing"
    );

    let salvaged = retry_batch_ops_individually(&conn, &batch, &mut bodies, 0, &mut tally);
    assert_eq!(salvaged.written, 2);
    assert_eq!(tally.counters().exec.started, 2);
    assert!(tally.counters().credentials.is_empty());
}

#[test]
fn mcp_protocol_only_event_does_not_claim_tool_storage() {
    use metrics_util::debugging::DebuggingRecorder;

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let event = WriteOp::McpCall(crate::events::McpCall {
        event_id: Some("abcdef123456".into()),
        timestamp: std::time::SystemTime::now(),
        server_name: "github".into(),
        method: "tools/list".into(),
        tool_name: None,
        request_id: Some("r1".into()),
        request_preview: Some("{}".into()),
        response_preview: Some(r#"{"tools":[]}"#.into()),
        decision: "allowed".into(),
        duration_ms: 1,
        error_message: None,
        process_name: Some("agent".into()),
        bytes_sent: 2,
        bytes_received: 12,
        transport: "vsock_frame".into(),
        policy_mode: Some("security_event".into()),
        policy_action: Some("allow".into()),
        policy_rule: Some("profiles.rules.default_mcp".into()),
        policy_reason: None,
        trace_id: Some("trace-list".into()),
        credential_ref: None,
    });

    let mut bodies = BodyArchive::open_for_tests(None, SystemTime::now, &conn);
    let outcome = metrics::with_local_recorder(&recorder, || {
        execute_memory_batch(
            &conn,
            &[event],
            &mut bodies,
            0,
            &mut crate::counters::LedgerTally::default(),
        )
        .unwrap()
    });
    let snapshot = snapshotter.snapshot().into_vec();

    assert!(
        outcome.tables.is_empty(),
        "protocol-only MCP evidence must not dirty the user tool ledger"
    );
    assert_eq!(outcome.written, 0);
    assert!(
        snapshot
            .iter()
            .all(|(key, _, _, _)| key.key().name() != DB_WRITE_OPS_TOTAL),
        "a no-op protocol event must not count as a persisted logger write"
    );
}
