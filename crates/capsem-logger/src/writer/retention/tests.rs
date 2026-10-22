//! The SQL half of generation publication, exercised on a real ledger.

use super::*;

const FIRST: i64 = capsem_archive::FILE_HEADER_BYTES as i64;
const SECOND: i64 = FIRST + 84; // An unreferenced gap separates the source blocks.
const PACKED_END: u64 = capsem_archive::FILE_HEADER_BYTES as u64 + 40;

fn ledger_with_two_blocks(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("open");
    crate::schema::create_tables(&conn).expect("schema");
    conn.execute_batch("PRAGMA foreign_keys = ON").expect("enforce keys");
    for (offset, sealed_at, event_id) in [
        (FIRST, "2026-01-01T00:00:00Z", "aaaaaaaaaaaa"),
        (SECOND, "2026-06-01T00:00:00Z", "bbbbbbbbbbbb"),
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
                       'xxh3-128:00000000000000000000000000000000',
                       ?2, 0, 4, NULL, NULL, ?3)",
            params![event_id, offset, sealed_at],
        )
        .expect("index row");
    }
    conn.execute("UPDATE archive_state SET committed_end = ?1", [SECOND + 40])
        .unwrap();
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
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect()
}

#[test]
fn publication_remaps_and_switches_generation_in_one_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let conn = ledger_with_two_blocks(&dir.path().join("session.db"));
    let old = crate::schema::archive_state(&conn).unwrap();
    let next = GenerationId::generate();
    let tx = conn.unchecked_transaction().unwrap();
    let counts = publish_inside_transaction(&tx, "2026-03-01T00:00:00Z", old, next, PACKED_END, 1, dir.path()).unwrap();
    tx.commit().unwrap();

    assert_eq!(counts, (1, 1));
    assert_eq!(offsets(&conn), vec![(FIRST, FIRST)]);
    let state = crate::schema::archive_state(&conn).unwrap();
    assert_eq!(state.header.generation_id, next);
    assert_eq!(state.committed_end, PACKED_END);
    assert_eq!(state.revision, old.revision + 1);
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn publication_failure_rolls_back_rows_offsets_and_identity_together() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let conn = ledger_with_two_blocks(&path);
    let old = crate::schema::archive_state(&conn).unwrap();
    super::super::retention_faults::fail_retention_for_path_for_tests(&path, RetentionFault::IndexTransaction);
    let tx = conn.unchecked_transaction().unwrap();
    assert!(publish_inside_transaction(
        &tx,
        "2026-03-01T00:00:00Z",
        old,
        GenerationId::generate(),
        PACKED_END,
        1,
        &crate::writer::archive_path_for_db(&path),
    )
    .is_err());
    drop(tx);

    assert_eq!(offsets(&conn), vec![(FIRST, FIRST), (SECOND, SECOND)]);
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), old);
}

#[test]
fn candidate_count_or_extent_disagreement_refuses_before_commit() {
    let dir = tempfile::tempdir().unwrap();
    let conn = ledger_with_two_blocks(&dir.path().join("session.db"));
    let old = crate::schema::archive_state(&conn).unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let error = publish_inside_transaction(
        &tx,
        "2026-03-01T00:00:00Z",
        old,
        GenerationId::generate(),
        PACKED_END + 1,
        2,
        dir.path(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("candidate copied 2 blocks"));
    drop(tx);
    assert_eq!(offsets(&conn), vec![(FIRST, FIRST), (SECOND, SECOND)]);
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), old);
}

#[test]
fn uncertain_commit_is_typed_for_either_authoritative_generation() {
    for (fault, published) in [
        (RetentionFault::CommitUnknownBefore, false),
        (RetentionFault::CommitUnknownAfter, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        let conn = ledger_with_two_blocks(&path);
        let old = crate::schema::archive_state(&conn).unwrap();
        let next = GenerationId::generate();
        super::super::retention_faults::fail_retention_for_path_for_tests(&path, fault);
        let error = publish_index(
            &conn,
            "2026-03-01T00:00:00Z",
            old,
            next,
            PACKED_END,
            1,
            &crate::writer::archive_path_for_db(&path),
        )
        .unwrap_err();
        assert!(matches!(error, PublishError::CommitUnknown(_)));
        let state = crate::schema::archive_state(&conn).unwrap();
        assert_eq!(state.header.generation_id == next, published);
        assert_eq!(offsets(&conn) == vec![(FIRST, FIRST)], published);
    }
}
