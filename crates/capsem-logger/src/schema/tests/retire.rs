use super::*;
use rusqlite::Connection;

fn legacy_ledger(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE net_events (id INTEGER PRIMARY KEY)")
        .unwrap();
}

#[test]
fn a_ledger_that_predates_the_archive_format_is_moved_with_its_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    legacy_ledger(&path);
    std::fs::write(dir.path().join("main.db-wal"), b"wal").unwrap();
    std::fs::write(dir.path().join("main.db-shm"), b"shm").unwrap();

    let retired = retire_predating_ledger(&path)
        .unwrap()
        .expect("a legacy ledger is moved");

    assert!(
        !path.exists(),
        "the legacy ledger no longer sits where the service opens"
    );
    assert!(retired.is_file());
    let name = retired.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("main.db.retired-"), "{name}");
    for sidecar in ["-wal", "-shm"] {
        assert!(!dir.path().join(format!("main.db{sidecar}")).exists());
        assert!(dir.path().join(format!("{name}{sidecar}")).is_file());
    }
}

#[test]
fn a_missing_ledger_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(retire_predating_ledger(&dir.path().join("main.db")).unwrap(), None);
}

#[test]
fn a_current_ledger_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let conn = Connection::open(&path).unwrap();
    create_tables_with_archive_header(&conn, None).unwrap();
    drop(conn);

    assert_eq!(retire_predating_ledger(&path).unwrap(), None);
    assert!(path.is_file());
}

#[test]
fn a_broken_ledger_is_an_error_not_a_move() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let conn = Connection::open(&path).unwrap();
    create_tables_with_archive_header(&conn, None).unwrap();
    conn.execute_batch("DELETE FROM archive_state").unwrap();
    drop(conn);

    assert!(retire_predating_ledger(&path).is_err());
    assert!(path.is_file(), "a ledger in the current format is never moved aside");
}
