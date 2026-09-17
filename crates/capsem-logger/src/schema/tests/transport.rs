use crate::schema::create_tables;
use rusqlite::Connection;

/// A column an older build lacked is not filled in behind the operator.
///
/// This test used to assert the opposite: `schema::migrate` re-added
/// `dns_events.answer_ip` with a discarded `ALTER TABLE`, and the transport
/// upgrade deliberately waited for it, which is what the old name -- "writer
/// finishes older column migrations before transport upgrade" -- described.
/// Nothing migrates now, so the writer opens the file it was given and
/// readiness names the column that is gone.
#[test]
fn a_writer_does_not_refill_a_column_an_older_build_lacked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch("DROP INDEX IF EXISTS idx_dns_events_answer_ip")
        .unwrap();
    conn.execute_batch("ALTER TABLE dns_events DROP COLUMN answer_ip")
        .unwrap();

    let writer = crate::DbWriter::open(&path, 8).unwrap();
    writer.shutdown_blocking();

    let error = crate::DbReader::open(&path)
        .unwrap()
        .ready()
        .expect_err("readiness must refuse a dns_events without answer_ip");
    assert!(
        error.contains("dns_events") && error.contains("answer_ip"),
        "readiness must name the table and column an older build lacked: {error}"
    );
}

#[test]
fn malformed_or_future_transport_markers_fail_without_reinitializing() {
    for change in [
        "DELETE FROM transport_schema",
        "UPDATE transport_schema SET version=2",
        "ALTER TABLE transport_events DROP COLUMN connection_id",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        let conn = Connection::open(&path).unwrap();
        create_tables(&conn).unwrap();
        // Drop the index before SQLite permits dropping its indexed column.
        conn.execute_batch("DROP INDEX idx_transport_events_connection")
            .unwrap();
        conn.execute_batch(change).unwrap();
        assert!(crate::DbReader::open(&path).is_err());
        assert!(crate::DbWriter::open(&path, 8).is_err());
    }
}

#[test]
fn transport_upgrade_preserves_the_shared_session_index_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let index = crate::SessionIndex::open(&path).unwrap();
    drop(index);
    let conn = Connection::open(&path).unwrap();
    let before: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert!(before > 1);
    let writer = crate::DbWriter::open(&path, 8).unwrap();
    writer.shutdown_blocking();
    let after: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(after, before);
    crate::DbReader::open(&path).unwrap().ready().unwrap();
    crate::SessionIndex::open(&path).unwrap();
}

#[tokio::test]
async fn a_current_database_missing_transport_rows_is_corrupt_and_not_recreated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch("DROP TABLE transport_events").unwrap();
    drop(conn);
    assert!(crate::DbHandle::open_external_reader(&path)
        .err()
        .unwrap()
        .to_string()
        .contains("transport_events"));
    assert!(crate::DbWriter::open(&path, 8)
        .err()
        .unwrap()
        .to_string()
        .contains("transport_events"));
}

/// A reader reads. A retained session from a build without the transport
/// ledger is refused by name, not upgraded in place.
///
/// `upgrade_legacy` used to run from `DbReader::open`: it took an IMMEDIATE
/// transaction on a file another process was writing, created
/// `transport_events`, and stamped the marker, with a retry inside it for the
/// case where a second process got there first. `CREATE_SCHEMA` without the
/// transport batch is exactly that older shape, which is what this fixture
/// builds, and the reader now says what it lacks instead of editing it.
#[tokio::test]
async fn an_external_reader_refuses_a_retained_session_from_an_older_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(crate::schema::CREATE_SCHEMA).unwrap();
    conn.execute("INSERT INTO security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json) VALUES(1,'abcdef123456','http.request','retained','allow','{}')", []).unwrap();
    drop(conn);

    let error = crate::DbHandle::open_external_reader(&path)
        .err()
        .expect("a reader must not adopt a ledger written before the transport rail")
        .to_string();
    assert!(
        error.contains("transport_schema"),
        "the refusal must name what the ledger lacks: {error}"
    );

    // And the file is left exactly as it was found.
    let conn = Connection::open(&path).unwrap();
    assert!(!crate::schema::table_exists(&conn, "main", "transport_events").unwrap());
    assert!(!crate::schema::table_exists(&conn, "main", "transport_schema").unwrap());
}

#[tokio::test]
async fn transport_facts_survive_buffer_flush_and_reopen_with_the_same_identity() {
    use crate::{DbReader, DbWriter, TransportEvent, TransportEventKind, WriteOp};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let facts = serde_json::json!({"decision":"block", "generation":"18446744073709551615"});
    let event = TransportEvent::new(
        "abcdef123456".into(),
        42,
        TransportEventKind::Connect,
        Some(uuid::Uuid::from_u128(1)),
        Some(uuid::Uuid::from_u128(2)),
        &facts,
    )
    .unwrap();
    writer.write_checked(WriteOp::TransportEvent(event)).await.unwrap();
    writer.flush_checked().await.unwrap();
    let reader = DbReader::open(&path).unwrap();
    let rows = reader
        .query_raw_with_params(
            "SELECT event_id,event_type,network_id,connection_id,event_json FROM transport_events ORDER BY id",
            &[],
        )
        .unwrap();
    assert!(rows.contains("abcdef123456"));
    assert!(rows.contains("network.connect"));
    assert!(rows.contains("00000000-0000-0000-0000-000000000001"));
    assert!(rows.contains("00000000-0000-0000-0000-000000000002"));
    assert!(rows.contains("18446744073709551615"));
    writer.shutdown_blocking();
    assert_eq!(
        DbReader::open(&path)
            .unwrap()
            .query_raw_with_params(
                "SELECT event_id,event_type,network_id,connection_id,event_json FROM transport_events ORDER BY id",
                &[],
            )
            .unwrap(),
        rows
    );
}

#[test]
fn invalid_identity_phase_or_oversized_facts_cannot_enter_the_producer_queue() {
    use crate::{TransportEvent, TransportEventKind};
    let connection = Some(uuid::Uuid::from_u128(2));
    for (id, kind, network, flow) in [
        ("invalid", TransportEventKind::Connect, None, connection),
        ("ABCDEF123456", TransportEventKind::Connect, None, connection),
        ("abcdef123456", TransportEventKind::Connect, None, None),
        ("abcdef123456", TransportEventKind::Lifecycle, None, connection),
        (
            "abcdef123456",
            TransportEventKind::Connect,
            Some(uuid::Uuid::nil()),
            connection,
        ),
    ] {
        assert!(TransportEvent::new(id.into(), 1, kind, network, flow, &()).is_err());
    }
    assert!(TransportEvent::new(
        "abcdef123456".into(),
        1,
        TransportEventKind::Connect,
        None,
        connection,
        &"x".repeat(65537)
    )
    .is_err());
}

#[test]
fn primary_transport_rows_exist_without_a_matched_security_rule() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO transport_events(event_id,timestamp_unix_ms,event_type,event_json) VALUES('abcdef123456',1,'network.connect','{}')",
        [],
    ).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM transport_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let rules: i64 = conn
        .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rules, 0);
    assert!(conn.execute("INSERT INTO transport_events(event_id,timestamp_unix_ms,event_type,event_json) VALUES('123456abcdef',1,'network.typo','{}')", []).is_err());
    assert!(conn.execute("INSERT INTO transport_events(event_id,timestamp_unix_ms,event_type,event_json) VALUES('abcdef123456',2,'network.close','{}')", []).is_err());
}
