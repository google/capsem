use super::*;

#[test]
fn body_index_uses_existing_keys_without_redundant_write_indexes() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let mut stmt = conn.prepare("PRAGMA index_list(event_body_blobs)").unwrap();
    let names: BTreeSet<String> = stmt
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for redundant in [
        "idx_event_body_blobs_event_id",
        "idx_event_body_blobs_block",
        "idx_event_body_blobs_trace_id",
        "idx_event_body_blobs_turn_id",
    ] {
        assert!(
            !names.contains(redundant),
            "{redundant} duplicates an existing access path or has no reader"
        );
    }
    for (query, index) in [
        (
            "EXPLAIN QUERY PLAN SELECT * FROM event_body_blobs WHERE event_id = 'abcdef123456'",
            "sqlite_autoindex_event_body_blobs_1",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT 1 FROM event_body_blobs WHERE block_offset = 80 LIMIT 1",
            "idx_event_body_blobs_archive_order",
        ),
    ] {
        let plan: String = conn.query_row(query, [], |row| row.get(3)).unwrap();
        assert!(plan.contains(index), "{query}: {plan}");
    }
}

#[test]
fn archive_state_is_a_typed_required_v4_singleton() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();

    let state = archive_state(&conn).expect("decode archive state");
    assert_eq!(state.format_version, 4);
    assert_eq!(state.committed_end, capsem_archive::FILE_HEADER_BYTES as u64);
    assert_eq!(state.revision, 1);
    let storage: (String, i64, String, i64, String, String, String) = conn
        .query_row(
            "SELECT typeof(singleton), singleton, typeof(archive_id), length(archive_id),
                    typeof(generation_id), typeof(committed_end), typeof(revision)
             FROM archive_state",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        storage,
        (
            "integer".into(),
            1,
            "blob".into(),
            16,
            "blob".into(),
            "integer".into(),
            "integer".into()
        )
    );
}

#[test]
fn an_existing_schema_without_archive_identity_is_refused_not_upgraded() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE body_blocks(block_offset INTEGER PRIMARY KEY)")
        .unwrap();

    let error = create_tables(&conn).expect_err("v2 ledger must not be upgraded implicitly");
    assert!(error.to_string().contains("archive_state"), "{error}");
    assert!(!table_exists(&conn, "main", "archive_state").unwrap());
}

#[test]
fn archive_state_rejects_missing_extra_and_wrongly_typed_rows() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute("DELETE FROM archive_state", []).unwrap();
    assert!(archive_state(&conn).unwrap_err().to_string().contains("exactly one"));

    conn.execute(
        "INSERT INTO archive_state(singleton,archive_id,generation_id,format_version,committed_end,revision)
         VALUES(1,zeroblob(16),zeroblob(16),4,80,1)",
        [],
    )
    .unwrap();
    assert!(archive_state(&conn).unwrap_err().to_string().contains("UUIDv4"));
}

/// A ledger written before generation publication has no archive identity.
/// Nothing migrates it implicitly: the open fails at the version marker
/// rather than filling in columns and adopting an unrelated v2 file.
#[test]
fn a_pre_archive_body_table_fails_to_open_by_name() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE event_body_blobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                source_table TEXT NOT NULL,
                direction TEXT NOT NULL,
                content_type TEXT,
                original_bytes INTEGER NOT NULL,
                stored_bytes INTEGER NOT NULL,
                truncated INTEGER NOT NULL,
                body_hash TEXT NOT NULL,
                body BLOB NOT NULL,
                trace_id TEXT,
                created_at TEXT NOT NULL
            );",
    )
    .unwrap();

    let error = create_tables(&conn).expect_err("a pre-archive ledger cannot be opened");
    let error = error.to_string();
    assert!(
        error.contains("archive_state") && error.contains("v2"),
        "opening a pre-generation ledger must name the missing identity contract: {error}"
    );
}

/// A v3 ledger carries no counter snapshot, and nothing rebuilds one by
/// scanning its rows: it is refused by version, not upgraded.
#[test]
fn a_v3_ledger_is_refused_rather_than_upgraded() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Recreate the singleton as v3 wrote it, around the v4 CHECK.
    conn.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
         UPDATE archive_state SET format_version = 3;
         PRAGMA ignore_check_constraints = OFF;",
    )
    .unwrap();
    let error = archive_state(&conn).unwrap_err().to_string();
    assert!(error.contains("format version 3 is unsupported; expected 4"), "{error}");
    assert!(
        create_tables(&conn).is_err(),
        "a writer must not adopt a v3 ledger either"
    );
}
