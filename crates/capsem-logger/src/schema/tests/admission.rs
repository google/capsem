use super::*;
use std::path::{Path, PathBuf};

fn existing(sql: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch(sql).unwrap();
    conn.close().unwrap();
    (root, path)
}

fn unchanged_read_only(path: &Path) -> rusqlite::Result<()> {
    let before = std::fs::read(path).unwrap();
    let names = || {
        let mut names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let entries = names();
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(conn.is_readonly(rusqlite::DatabaseName::Main).unwrap());
    assert!(conn.is_autocommit());
    let result = validate_existing_host_schema(&conn);
    assert_eq!(conn.total_changes(), 0);
    assert!(conn.is_autocommit(), "admission must not retain a transaction");
    conn.close().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before, "admission changed original bytes");
    assert_eq!(names(), entries, "admission created or removed a sidecar");
    result
}

#[test]
fn existing_host_admission_accepts_current_without_any_writes_or_retired_table_cleanup() {
    let (_root, path) = existing(
        "INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail)
         VALUES(1,'service_started','fixture',x'80');
         CREATE TABLE ledger_commitments(sentinel TEXT);
         INSERT INTO ledger_commitments VALUES('retain original'); PRAGMA user_version=123;",
    );
    unchanged_read_only(&path).unwrap();
    unchanged_read_only(&path).unwrap();
}

#[test]
fn existing_host_admission_and_writer_refuse_the_same_obsolete_hash_columns() {
    for column in ["hash", "prev_hash"] {
        let (_root, path) = existing(&format!("ALTER TABLE host_events ADD COLUMN {column} BLOB"));
        let admitted = unchanged_read_only(&path).unwrap_err().to_string();
        assert!(admitted.contains("obsolete host hash-chain schema"), "{admitted}");
        let conn = Connection::open(&path).unwrap();
        let writer = create_tables(&conn).unwrap_err().to_string();
        assert_eq!(admitted, writer);
        assert!(table_column_names(&conn, "main", "host_events")
            .unwrap()
            .iter()
            .any(|value| value == column));
        assert!(conn.is_autocommit());
    }
}

#[test]
fn existing_host_admission_refuses_missing_table_and_column_without_repair() {
    for sql in [
        "DROP TABLE transport_events",
        "ALTER TABLE transport_events RENAME COLUMN event_type TO lost_type",
        "ALTER TABLE host_events RENAME COLUMN kind TO lost_kind",
    ] {
        let (_root, path) = existing(sql);
        assert!(unchanged_read_only(&path).is_err(), "admitted {sql}");
    }
}

#[test]
fn existing_host_admission_uses_current_transport_and_archive_singleton_validators() {
    for (sql, expected) in [
        ("UPDATE transport_schema SET version=9", "transport schema version"),
        ("DELETE FROM archive_state", "exactly one row"),
        (
            "PRAGMA ignore_check_constraints=ON; UPDATE archive_state SET format_version=3",
            "format version",
        ),
        ("UPDATE archive_state SET generation_id=zeroblob(16)", "UUID"),
    ] {
        let (_root, path) = existing(sql);
        let error = unchanged_read_only(&path).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn existing_host_admission_refuses_malformed_schema_and_empty_existing_database() {
    let (_root, path) = existing(
        "PRAGMA writable_schema=ON;
         UPDATE sqlite_master SET sql='malformed table declaration' WHERE name='host_events';
         PRAGMA writable_schema=OFF;",
    );
    assert!(unchanged_read_only(&path).is_err());
    let empty = path.with_file_name("empty.db");
    std::fs::write(&empty, []).unwrap();
    assert!(
        unchanged_read_only(&empty).is_err(),
        "existing empty is not fresh absence"
    );
}

#[test]
fn existing_host_admission_and_writer_share_security_and_integrity_constraint_refusal() {
    for (table, expected) in [
        ("security_rule_events", "security ledgers"),
        ("net_events", "integrity checks"),
    ] {
        let (_root, path) = existing("");
        let conn = Connection::open(&path).unwrap();
        let canonical: String = conn
            .query_row("SELECT sql FROM sqlite_master WHERE name=?1", [table], |row| row.get(0))
            .unwrap();
        let stale = if table == "net_events" {
            canonical.replace(&"[0-9a-f]".repeat(64), "[0-9a-f]*")
        } else {
            canonical.replace(", 'network.connect'", "")
        };
        assert_ne!(canonical, stale);
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        conn.execute_batch(&stale).unwrap();
        conn.close().unwrap();
        let admitted = unchanged_read_only(&path).unwrap_err().to_string();
        assert!(admitted.contains(expected), "{admitted}");
        let conn = Connection::open(&path).unwrap();
        let writer = create_tables(&conn).unwrap_err().to_string();
        assert_eq!(admitted, writer);
    }
}
