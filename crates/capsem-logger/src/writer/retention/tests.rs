//! The index half of retention, exercised directly.
//!
//! These reach `reindex` and `restore_offsets` on a connection this test owns,
//! which is what lets them assert the remap under enforced foreign keys and
//! watch the transaction roll back. Enforcement is not hypothetical here --
//! see `the_writer_connection_enforces_foreign_keys` -- so `defer_foreign_keys`
//! is load-bearing on the live path, not insurance.

use super::*;

/// A ledger with two one-block bodies, on a connection that enforces keys.
fn ledger_with_two_blocks(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("open");
    crate::schema::create_tables(&conn).expect("schema");
    conn.execute_batch("PRAGMA foreign_keys = ON").expect("enforce keys");
    for (offset, sealed_at, event_id) in [
        (16, "2026-01-01T00:00:00Z", "aaaaaaaaaaaa"),
        (100, "2026-06-01T00:00:00Z", "bbbbbbbbbbbb"),
    ] {
        conn.execute(
            "INSERT INTO body_blocks (block_offset, raw_len, disk_len, sealed_at) VALUES (?1, 40, 40, ?2)",
            params![offset, sealed_at],
        )
        .expect("block row");
        conn.execute(
            "INSERT INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash,
                block_offset, body_offset, body_len, trace_id, turn_id, created_at
             ) VALUES (?1, 'security.rule', 'security_rule_events', 'payload', NULL,
                       4, 4, 0,
                       'blake3:0000000000000000000000000000000000000000000000000000000000000000',
                       ?2, 0, 4, NULL, NULL, ?3)",
            params![event_id, offset, sealed_at],
        )
        .expect("index row");
    }
    conn
}

fn offsets(conn: &Connection) -> Vec<(i64, i64)> {
    let mut statement = conn
        .prepare(
            "SELECT b.block_offset, e.block_offset FROM body_blocks b
             JOIN event_body_blobs e ON e.block_offset = b.block_offset
             ORDER BY b.block_offset",
        )
        .expect("prepare");
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query");
    rows.map(|row| row.expect("row")).collect()
}

/// What everything below rests on.
#[test]
fn the_writer_connection_enforces_foreign_keys() {
    // Not an assumption: rusqlite turns enforcement on for every connection it
    // opens, so `defer_foreign_keys` in the remap is load-bearing in
    // production rather than insurance for a configuration nobody uses. If a
    // future rusqlite stops doing that, this says so before the comment that
    // depends on it goes stale.
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("probe.db")).expect("open");
    let enforced: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read the pragma");
    assert_eq!(enforced, 1, "the ledger's connections enforce foreign keys");
}

/// The remap moves a parent and its children one pair at a time, so the two
/// are out of step in the middle of the transaction even though the committed
/// state is not. That is only legal because the transaction defers the check
/// to commit; without `defer_foreign_keys` this fails on the first pair.
#[test]
fn the_remap_commits_under_enforced_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let conn = ledger_with_two_blocks(&dir.path().join("session.db"));
    // What a compaction that dropped the first block would produce.
    let moved = BTreeMap::from([(100_u64, 16_u64)]);

    let dropped = reindex(&conn, "2026-03-01T00:00:00Z", &moved, std::path::Path::new("unused"))
        .expect("the remap must commit with foreign keys enforced");

    assert_eq!((dropped.blocks, dropped.rows), (1, 1));
    assert_eq!(
        offsets(&conn),
        vec![(16, 16)],
        "the survivor moved, and its index row moved with it"
    );
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
        .expect("check keys");
    assert_eq!(violations, 0);
}

/// The reverse pass, for when the rename failed after the remap committed.
/// It runs under enforcement too, and it must put every survivor back at the
/// offset the unreplaced file still holds it at.
#[test]
fn restoring_offsets_puts_every_survivor_back() {
    let dir = tempfile::tempdir().unwrap();
    let conn = ledger_with_two_blocks(&dir.path().join("session.db"));
    let moved = BTreeMap::from([(100_u64, 16_u64)]);
    reindex(&conn, "2026-03-01T00:00:00Z", &moved, std::path::Path::new("unused")).expect("remap");

    restore_offsets(&conn, &moved).expect("the reverse pass must commit too");

    assert_eq!(
        offsets(&conn),
        vec![(100, 100)],
        "the survivor is back where the file still has it"
    );
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
        .expect("check keys");
    assert_eq!(violations, 0);
}

/// The index and the staged file must describe the same set of blocks. They
/// are derived from one `sealed_at` predicate a moment apart, so this is a
/// cross-check rather than an expected case -- and it must refuse rather than
/// leave rows naming blocks the new file does not have.
#[test]
fn a_map_that_disagrees_with_the_index_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let conn = ledger_with_two_blocks(&dir.path().join("session.db"));
    // The file was staged keeping both blocks; the cutoff drops one.
    let moved = BTreeMap::from([(16_u64, 16_u64), (100_u64, 100_u64)]);

    let error = reindex(&conn, "2026-03-01T00:00:00Z", &moved, std::path::Path::new("unused"))
        .expect_err("a staged file and an index that disagree must not both be committed");

    assert!(
        format!("{error}").contains("staged 2 blocks but the index holds 1"),
        "{error}"
    );
    assert_eq!(
        offsets(&conn).len(),
        2,
        "and the transaction rolls back, so nothing was deleted"
    );
}
