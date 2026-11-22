//! A pre-v4 ledger is kept aside and replaced; nothing else is moved.
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::writer::DbWriter;

fn aside_files(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains(".pre-v4-"))
        .collect();
    found.sort();
    found
}

/// What Capsem 0.6.3 left in `~/.capsem/sessions/main.db`: ledger tables,
/// recorded rows, and no `archive_state`.
fn pre_archive_ledger(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE net_events (id INTEGER PRIMARY KEY, domain TEXT);
         INSERT INTO net_events (domain) VALUES ('example.com');",
    )
    .unwrap();
}

#[test]
fn a_pre_v4_ledger_is_kept_aside_and_a_fresh_one_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    pre_archive_ledger(&path);

    let writer = DbWriter::open(&path, 64).expect("a 0.6.3 ledger no longer stops the open");
    drop(writer);

    let aside = aside_files(dir.path());
    let kept = aside
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("main.db.pre-v4-"))
        })
        .expect("the old ledger is kept");
    let old = Connection::open(kept.join("main.db")).unwrap();
    let domain: String = old
        .query_row("SELECT domain FROM net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(domain, "example.com", "the kept file is the old ledger, intact");

    let fresh = Connection::open(&path).unwrap();
    let version: i64 = fresh
        .query_row("SELECT format_version FROM archive_state", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn a_current_ledger_is_never_moved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    drop(DbWriter::open(&path, 64).unwrap());
    drop(DbWriter::open(&path, 64).unwrap());
    assert!(aside_files(dir.path()).is_empty());
}

/// A ledger that has `archive_state` but is malformed is broken, not old:
/// it still fails loudly and stays where it is.
#[test]
fn a_broken_current_ledger_still_fails_and_stays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    drop(DbWriter::open(&path, 64).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM archive_state", [])
        .unwrap();

    let error = DbWriter::open(&path, 64)
        .err()
        .expect("a ledger with no archive row is refused");
    assert!(error.to_string().contains("exactly one row, found 0"), "{error}");
    assert!(aside_files(dir.path()).is_empty());
}

fn pre_environment_ledger(path: &Path) {
    drop(DbWriter::open(path, 8).unwrap());
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name GLOB 'environment_*'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for table in tables {
        conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    conn.execute(
        "INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail) VALUES(1,'service_started','operator',?1)",
        [b"preserved-history".as_slice()],
    )
    .unwrap();
}

#[test]
fn unrecognized_pre_environment_shape_fails_without_moving_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    pre_environment_ledger(&path);
    let archive = super::archive_path_for_db(&path);
    let canary = archive.join("preserved-cohort-canary");
    std::fs::write(&canary, b"archive must remain exact").unwrap();
    assert!(DbWriter::open(&path, 8).is_err());
    let old = Connection::open(&path).unwrap();
    assert_eq!(
        old.query_row("SELECT detail FROM host_events", [], |row| row.get::<_, Vec<u8>>(0))
            .unwrap(),
        b"preserved-history"
    );
    assert_eq!(std::fs::read(canary).unwrap(), b"archive must remain exact");
}

#[test]
fn missing_old_tables_and_partial_environment_formats_are_not_reset() {
    for damage in [
        "DROP TABLE host_events",
        "CREATE TABLE environment_schema(id INTEGER PRIMARY KEY,version INTEGER)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.db");
        pre_environment_ledger(&path);
        Connection::open(&path).unwrap().execute_batch(damage).unwrap();
        assert!(DbWriter::open(&path, 8).is_err(), "damaged history was reset: {damage}");
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".pre-environment-")));
    }
}

#[test]
fn interrupted_preservation_refuses_a_fresh_writer_even_if_database_already_moved() {
    for database_present in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.db");
        if database_present {
            pre_archive_ledger(&path);
        }
        let pending = super::with_suffix(&path, ".set-aside-pending");
        std::fs::create_dir(&pending).unwrap();
        let error = DbWriter::open(&path, 8)
            .err()
            .expect("interrupted cohort must fail closed");
        assert!(error.to_string().contains("interrupted ledger preservation"));
        assert_eq!(path.exists(), database_present);
    }
}

#[test]
fn preserved_database_reads_committed_history_still_in_its_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    pre_archive_ledger(&path);
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "journal_mode", "WAL").unwrap();
    connection.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    connection
        .execute("INSERT INTO net_events(domain) VALUES ('wal-only-history')", [])
        .unwrap();
    assert!(super::with_suffix(&path, "-wal").is_file());
    let _lock = crate::writer::writer_lock::acquire(&path).unwrap();
    let kept = super::set_aside_pre_archive_ledger(&path).unwrap().unwrap();
    assert!(
        super::with_suffix(&kept, "-wal").is_file(),
        "SQLite must find the preserved WAL beside its database"
    );
    let reader = Connection::open(&kept).unwrap();
    assert_eq!(
        reader
            .query_row("SELECT domain FROM net_events WHERE id=2", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "wal-only-history"
    );
    // Keep the original connection alive until after recovery was proved: its
    // last close would otherwise checkpoint and hide a broken companion name.
    drop(reader);
    drop(connection);
}
