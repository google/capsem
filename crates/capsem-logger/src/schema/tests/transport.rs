use crate::schema::create_tables;
use rusqlite::Connection;

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

#[tokio::test]
async fn an_external_reader_can_upgrade_a_retained_session_without_booting_its_vm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).unwrap();
    // The previous schema has all legacy tables but no primary transport ledger.
    let legacy = &crate::schema::CREATE_SCHEMA[crate::schema::CREATE_SCHEMA
        .find("    CREATE TABLE IF NOT EXISTS net_events")
        .unwrap()..];
    conn.execute_batch(legacy).unwrap();
    conn.execute("INSERT INTO security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json,event_json) VALUES(1,'abcdef123456','http.request','retained','allow','{}','{}')", []).unwrap();
    drop(conn);
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    reader.ready().await.unwrap();
    let rows = reader
        .query("SELECT rule_id FROM security_rule_events", &[])
        .await
        .unwrap();
    assert!(rows.contains("retained"));
    assert!(reader.query("SELECT COUNT(*) FROM transport_events", &[]).await.is_ok());
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
