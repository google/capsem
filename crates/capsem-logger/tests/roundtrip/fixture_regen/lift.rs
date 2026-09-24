//! Read a v3 source ledger through today's reader, for the replay only.

use rusqlite::Connection;

use super::{archive_lock_path, replace_directory};

/// A scratch copy of a v3 source ledger that today's reader will open, or
/// `None` when the source is already current.
pub(super) fn lift_v3_source(db_path: &std::path::Path, conn: &Connection) -> Option<tempfile::TempDir> {
    use std::os::unix::fs::PermissionsExt as _;

    let version: i64 = conn
        .query_row(
            "SELECT format_version FROM archive_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("read the source format version");
    if version != 3 {
        return None;
    }
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("test.db");
    // The checked-in source is checkpointed, so the file is the whole ledger.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    std::fs::copy(db_path, &copy).expect("copy the source ledger");
    replace_directory(&db_path.with_extension("bodies"), &copy.with_extension("bodies"));
    let lock = archive_lock_path(&copy);
    std::fs::File::create(&lock).unwrap();
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
    let lifted = Connection::open(&copy).unwrap();
    lifted
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE archive_state SET format_version = 4;
             CREATE TABLE ledger_counters (singleton INTEGER PRIMARY KEY, counters BLOB NOT NULL);
             INSERT INTO ledger_counters VALUES (1, x'80');",
        )
        .expect("lift the scratch copy's marker");
    Some(scratch)
}
