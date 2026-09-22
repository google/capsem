use crate::schema::{create_tables, CREATE_SCHEMA};
use rusqlite::Connection;

/// The event types this build added last. Removing them from `CREATE_SCHEMA`
/// is how these tests build a ledger an older build would have written.
const NETWORK_TYPES: &str = ", 'network.connect', 'network.connect_result', 'network.close', 'network.lifecycle', 'network.probe', 'network.probe_result'";

/// A ledger that is current in every respect except the event-type CHECK.
///
/// `CREATE_SCHEMA` alone is not that: the transport ledger is a separate batch
/// now, and a file without it fails readiness for a different reason entirely.
fn ledger_with_the_older_check(conn: &Connection) {
    conn.execute_batch(&CREATE_SCHEMA.replace(NETWORK_TYPES, "")).unwrap();
    conn.execute_batch(crate::schema::ddl::CREATE_TRANSPORT).unwrap();
}

fn insert_rule(conn: &Connection, event_type: &str) -> rusqlite::Result<usize> {
    conn.execute("INSERT INTO security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json) VALUES (1,'abcdef123456',?1,'fixture','allow','{}')", [event_type])
}

/// The declaration in `ddl.rs` carries the whole current list, and only it.
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
        conn.execute("INSERT INTO security_decision_events(timestamp_unix_ms,event_id,event_type,stage,actor,previous_decision,requested_decision,effective_decision) VALUES(1,'abcdef123456',?1,'rule','fixture','allow','block','block')", [event_type]).unwrap();
        conn.execute("INSERT INTO security_ask_events(timestamp_unix_ms,ask_id,event_id,event_type,rule_id,rule_name,status,rule_json) VALUES(1,'abcdef123456','abcdef123456',?1,'fixture','fixture','pending','{}')", [event_type]).unwrap();
    }
    assert!(insert_rule(&conn, "network.typo").is_err());
}

/// A ledger with an older CHECK is refused, and the refusal names the table.
///
/// This test used to assert the opposite: `network_types::migrate` renamed
/// each stale table to `<name>_before_network_types`, rebuilt it from
/// `CREATE_SCHEMA`, copied the rows over and restored the AUTOINCREMENT
/// sequence, and the test checked that the ids came out unchanged. Three
/// security ledgers rewritten in place on open, with the evidence of the
/// rewrite removed. Detection was the sound half and it is what is left.
#[test]
fn a_ledger_with_an_older_event_type_check_is_refused_by_name() {
    let conn = Connection::open_in_memory().unwrap();
    ledger_with_the_older_check(&conn);
    insert_rule(&conn, "http.request").unwrap();

    let error = super::assert_current(&conn)
        .expect_err("a ledger that cannot record this build's event types must not open")
        .to_string();
    for table in super::TABLES {
        assert!(
            error.contains(table),
            "the refusal must name every stale ledger, not just the first: {error}"
        );
    }
}

/// Refused, and left exactly as it was found.
///
/// The rebuild's own failure path had to be careful to clean up the renamed
/// table it left behind; a check that only reads has no such path to get
/// wrong, and this is what says so.
#[test]
fn a_refused_ledger_is_not_rewritten() {
    let conn = Connection::open_in_memory().unwrap();
    ledger_with_the_older_check(&conn);
    insert_rule(&conn, "http.request").unwrap();

    assert!(super::assert_current(&conn).is_err());
    assert!(super::validate_ready(&conn, "main").is_err());

    let retained: String = conn
        .query_row("SELECT event_type FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "http.request", "the recorded row must survive the refusal");
    let renamed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%before_network_types%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(renamed, 0, "nothing renames a security ledger any more");
    // And the constraint is still the old one: refusing did not widen it.
    assert!(insert_rule(&conn, "network.connect").is_err());
}

/// The writer refuses such a file outright, before it can write to it.
#[test]
fn a_writer_will_not_open_a_ledger_with_an_older_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    crate::DbWriter::open(&path, 8).unwrap().shutdown_blocking();
    let conn = Connection::open(&path).unwrap();
    for table in super::TABLES {
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    ledger_with_the_older_check(&conn);
    drop(conn);

    let error = crate::DbWriter::open(&path, 8)
        .err()
        .expect("the writer must refuse a ledger it cannot fully record into")
        .to_string();
    assert!(error.contains("security_rule_events"), "{error}");
}

/// Readiness catches it too, and it has to look at the declaration to do so:
/// a table with the old CHECK has exactly the columns the new one has.
#[test]
fn readiness_refuses_what_column_names_cannot_see() {
    let conn = Connection::open_in_memory().unwrap();
    ledger_with_the_older_check(&conn);

    let columns_only = crate::schema::validate_ready_schema(&conn);
    assert!(columns_only.is_err(), "readiness must refuse a stale event_type CHECK");
    let error = columns_only.unwrap_err();
    assert!(error.contains("security_rule_events"), "{error}");
    assert!(error.contains("event_type"), "{error}");
}

/// An absent table is readiness's business, not this module's: saying it twice
/// in two vocabularies helps nobody.
#[test]
fn a_missing_security_table_is_left_to_readiness() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch("DROP TABLE security_ask_events").unwrap();

    assert!(super::assert_current(&conn).is_ok());
    let error = crate::schema::validate_ready_schema(&conn).unwrap_err();
    assert!(error.contains("security_ask_events"), "{error}");
}

/// A ledger whose security tables still keep the payload inline, built the way
/// the previous build wrote them: the current declaration plus the column.
fn ledger_with_inline_payloads(conn: &Connection) {
    create_tables(conn).unwrap();
    for table in super::TABLES {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN event_json TEXT NOT NULL DEFAULT '{{}}'"
        ))
        .unwrap();
    }
}

/// The payload moved to the archive, and a ledger that still keeps it inline
/// is refused at open, naming every table that does. Accepting it would fail
/// the session's first security write instead -- a decision made and not
/// recorded, which is the outcome a forensic ledger exists to rule out.
#[test]
fn a_ledger_that_keeps_security_payloads_inline_is_refused_by_name() {
    let conn = Connection::open_in_memory().unwrap();
    ledger_with_inline_payloads(&conn);

    let error = super::assert_current(&conn)
        .expect_err("a ledger that keeps security payloads inline must not open")
        .to_string();
    for table in super::TABLES {
        assert!(error.contains(table), "the refusal must name {table}: {error}");
    }
    assert!(error.contains("event_json"), "and say what is wrong with it: {error}");

    let readiness = crate::schema::validate_ready_schema(&conn)
        .expect_err("readiness must refuse it too, since column presence alone would pass it");
    assert!(readiness.contains("event_json"), "{readiness}");
}

/// The current declaration is not mistaken for the old one.
#[test]
fn a_fresh_ledger_keeps_no_security_payload_inline() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    assert!(super::assert_current(&conn).is_ok());
    assert!(super::inline_payloads(&conn, "main").unwrap().is_empty());
}
