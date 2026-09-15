use crate::schema::{create_tables, migrate, CREATE_SCHEMA};
use rusqlite::Connection;

const NETWORK_TYPES: &str = ", 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result'";

fn insert_rule(conn: &Connection, event_type: &str) -> rusqlite::Result<usize> {
    conn.execute("INSERT INTO security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json,event_json) VALUES (1,'abcdef123456',?1,'fixture','allow','{}','{}')", [event_type])
}

#[test]
fn fresh_security_ledgers_accept_network_events_but_reject_unknown_types() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    for event_type in [
        "network.connect",
        "network.connect_result",
        "network.close",
        "network.lifecycle",
        "network.probe",
        "network.probe_result",
    ] {
        insert_rule(&conn, event_type).unwrap();
        conn.execute("INSERT INTO security_decision_events(timestamp_unix_ms,event_id,event_type,stage,actor,previous_decision,requested_decision,effective_decision,event_json) VALUES(1,'abcdef123456',?1,'rule','fixture','allow','block','block','{}')", [event_type]).unwrap();
        conn.execute("INSERT INTO security_ask_events(timestamp_unix_ms,ask_id,event_id,event_type,rule_id,rule_name,status,rule_json,event_json) VALUES(1,'abcdef123456','abcdef123456',?1,'fixture','fixture','pending','{}','{}')", [event_type]).unwrap();
    }
    assert!(insert_rule(&conn, "network.typo").is_err());
}

#[test]
fn network_type_upgrade_preserves_rows_and_indexes_and_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&CREATE_SCHEMA.replace(NETWORK_TYPES, "")).unwrap();
    insert_rule(&conn, "http.request").unwrap();
    conn.execute(
        "UPDATE sqlite_sequence SET seq=40 WHERE name='security_rule_events'",
        [],
    )
    .unwrap();
    assert!(insert_rule(&conn, "network.connect").is_err());
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
    insert_rule(&conn, "network.connect").unwrap();
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id,event_type FROM security_rule_events ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, [(1, "http.request".into()), (41, "network.connect".into())]);
    let indexes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='security_rule_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(indexes >= 4);
    assert!(insert_rule(&conn, "network.typo").is_err());
}

#[test]
fn rejected_legacy_rows_roll_back_the_constraint_upgrade() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&CREATE_SCHEMA.replace(NETWORK_TYPES, "")).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=ON").unwrap();
    insert_rule(&conn, "network.typo").unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=OFF").unwrap();
    assert!(migrate(&conn).is_err());
    let retained: String = conn
        .query_row("SELECT event_type FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "network.typo");
    let temporary: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%before_network_types%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(temporary, 0);
    assert!(insert_rule(&conn, "network.connect").is_err());
}

#[test]
fn an_existing_memory_ledger_upgrades_constraints_without_losing_pending_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(&CREATE_SCHEMA.replace(NETWORK_TYPES, "")).unwrap();
    let uri = crate::schema::memory_uri_for_path(&path);
    crate::schema::create_memory_tables(&conn, &uri).unwrap();
    conn.execute("INSERT INTO mem.security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json,event_json) VALUES(1,'abcdef123456','http.request','pending','allow','{}','{}')", []).unwrap();
    conn.execute(
        "UPDATE mem.sqlite_sequence SET seq=40 WHERE name='security_rule_events'",
        [],
    )
    .unwrap();
    crate::schema::create_memory_read_views(&conn).unwrap();
    // A writer upgrades disk; an already-open reader owns these temporary views.
    let writer = Connection::open(&path).unwrap();
    migrate(&writer).unwrap();
    crate::schema::create_memory_tables(&conn, &uri).unwrap();
    crate::schema::create_memory_read_views(&conn).unwrap();
    conn.execute("INSERT INTO mem.security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json,event_json) VALUES(1,'abcdef123456','network.connect','new','allow','{}','{}')", []).unwrap();
    let rows: Vec<String> = conn
        .prepare("SELECT rule_id FROM mem.security_rule_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, ["pending", "new"]);
    let maximum: i64 = conn
        .query_row("SELECT MAX(id) FROM temp.security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(maximum, 41);
}
