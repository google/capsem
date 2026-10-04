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
    let old = Connection::open(kept).unwrap();
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
