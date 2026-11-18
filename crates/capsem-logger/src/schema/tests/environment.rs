use super::*;

const ENVIRONMENT_TABLES: &[&str] = &[
    "environment_catalog_revocations",
    "environment_revision_purges",
    "environment_request_purges",
    "environment_variable_purges",
    "environment_revision_references",
    "environment_applications",
    "environment_request_decisions",
    "environment_request_invalidations",
    "environment_requests",
    "environment_revisions",
    "environment_schema",
];

fn remove_environment_format(conn: &Connection) {
    for table in ENVIRONMENT_TABLES {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}")).unwrap();
    }
}

#[test]
fn fresh_machine_ledger_has_explicit_environment_format() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    for table in ENVIRONMENT_TABLES {
        assert!(!columns_for_schema(&conn, "main", table).is_empty(), "missing {table}");
        assert_eq!(
            is_disk_only_table(table),
            !matches!(*table, "environment_revisions" | "environment_requests"),
            "snapshot-backed metadata stages with its archive; other state is disk-only"
        );
    }
    assert_eq!(
        conn.query_row("SELECT version FROM environment_schema WHERE id=1", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn older_environment_formats_are_refused_before_ddl_without_modifying_history() {
    for version in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.db");
        let conn = Connection::open(&path).unwrap();
        create_tables(&conn).unwrap();
        conn.execute("UPDATE environment_schema SET version=?1", [version])
            .unwrap();
        conn.close().unwrap();
        let before = std::fs::read(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert!(
            validate_existing_host_schema(&conn).is_err(),
            "older layouts lack reviewed pins or durable known revocations"
        );
        assert!(create_tables(&conn).is_err());
        conn.close().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn pre_environment_ledger_is_refused_before_ddl_and_remains_byte_identical() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    create_tables(&conn).unwrap();
    remove_environment_format(&conn);
    conn.close().unwrap();
    let before = std::fs::read(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    assert!(validate_existing_host_schema(&conn).is_err());
    assert!(create_tables(&conn).is_err(), "existing history was silently upgraded");
    conn.close().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn torn_environment_tables_are_not_recreated_as_empty_history() {
    for table in ENVIRONMENT_TABLES {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        let before: i64 = conn.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
        assert!(create_tables(&conn).is_err(), "recreated missing {table}");
        assert_eq!(
            conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            before
        );
    }
}

fn revision(
    conn: &Connection,
    environment: &str,
    id: &str,
    previous: Option<&str>,
    state: &str,
    payload: &str,
) -> rusqlite::Result<usize> {
    conn.execute("INSERT INTO environment_revisions(environment_id,revision_id,previous_revision_id,timestamp_unix_ms,actor,reason,state,payload_event_id) VALUES(?1,?2,?3,1,'operator','approved',?4,?5)", rusqlite::params![environment,id,previous,state,payload])
}

#[test]
fn environment_schema_serializes_commits_and_preserves_reference_relationships() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // The staging ledger has its own FK policy; this exercises the canonical
    // disk-state declarations independently of typed writer admission checks.
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    let environment = "7aa9f198-006d-4b46-8a76-1880a29d6ea2";
    let first = "6b89b1da-acfc-45ec-a3b5-945c3ae3c9bc";
    let second = "d8aef1e7-3ce5-44e2-8ef9-39713bff8597";
    let third = "763d7165-d0a2-44be-826b-04fdb983bca1";
    revision(&conn, environment, first, None, "committed", "aaaaaaaaaaaa").unwrap();
    revision(&conn, environment, second, Some(first), "committing", "bbbbbbbbbbbb").unwrap();
    assert!(revision(&conn, environment, third, Some(first), "committing", "cccccccccccc").is_err());
    conn.execute(
        "UPDATE environment_revisions SET state='committed' WHERE revision_id=?1",
        [second],
    )
    .unwrap();
    revision(&conn, environment, third, Some(second), "committing", "cccccccccccc").unwrap();
    assert!(revision(
        &conn,
        environment,
        "missing-parent",
        Some("absent"),
        "committed",
        "dddddddddddd"
    )
    .is_err());
    assert!(conn.execute("INSERT INTO environment_revision_references(owner_kind,owner_id,owner_generation,environment_id,revision_id) VALUES('current_environment',?1,'',?1,'absent')", [environment]).is_err());
    conn.execute("INSERT INTO environment_revision_references(owner_kind,owner_id,owner_generation,environment_id,revision_id) VALUES('current_environment',?1,'',?1,?2)", rusqlite::params![environment,second]).unwrap();
    assert!(conn
        .execute("DELETE FROM environment_revisions WHERE revision_id=?1", [first])
        .is_err());
}

#[test]
fn unknown_environment_versions_and_missing_commit_indexes_fail_closed() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute("UPDATE environment_schema SET version=4", []).unwrap();
    assert!(create_tables(&conn).is_err());
    conn.execute("UPDATE environment_schema SET version=3", []).unwrap();
    conn.execute_batch("DROP INDEX idx_environment_revisions_inflight")
        .unwrap();
    assert!(create_tables(&conn).is_err());
    assert!(validate_ready_schema(&conn).is_err());
}

#[test]
fn narrow_environment_payload_checks_are_refused_before_ddl() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let current: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='event_body_blobs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let old = current
        .replace(", 'environment.revision', 'environment.request'", "")
        .replace(", 'environment_revisions', 'environment_requests'", "");
    assert_ne!(old, current);
    conn.execute_batch("DROP TABLE event_body_blobs").unwrap();
    conn.execute_batch(&old).unwrap();
    let version: i64 = conn.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
    assert!(create_tables(&conn).is_err());
    assert!(validate_ready_schema(&conn).is_err());
    assert_eq!(
        conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        version
    );
}

#[test]
fn missing_environment_body_index_is_named_and_never_recreated() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch("DROP TABLE event_body_blobs").unwrap();
    assert!(create_tables(&conn)
        .unwrap_err()
        .to_string()
        .contains("event_body_blobs"));
    assert!(!table_exists(&conn, "main", "event_body_blobs").unwrap());
}

#[test]
fn environment_staging_retains_checks_without_cross_database_foreign_keys() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let uri = memory_uri_for_path(&directory.path().join("host.db"));
    create_memory_tables(&conn, &uri).unwrap();
    let durable_keys: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('environment_revisions','main')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let staged_keys: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('environment_revisions','mem')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(durable_keys > 0);
    assert_eq!(staged_keys, 0);
    assert!(conn.execute("INSERT INTO mem.environment_revisions(environment_id,revision_id,timestamp_unix_ms,actor,reason,state,payload_event_id) VALUES('env','rev',1,'operator','reason','unknown-state','aaaaaaaaaaaa')", []).is_err());
}
