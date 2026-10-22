use super::*;
use std::cell::Cell;

type IndexMetadata = (
    i64,
    String,
    String,
    Option<String>,
    i64,
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    String,
);

thread_local! { static INDEX_STATEMENTS: Cell<usize> = const { Cell::new(0) }; }
fn count_index_statements(sql: &str) {
    if sql.trim_start().starts_with("INSERT OR REPLACE INTO event_body_blobs") {
        INDEX_STATEMENTS.with(|count| count.set(count.get() + 1));
    }
}
fn clock() -> SystemTime {
    SystemTime::UNIX_EPOCH
}

#[test]
fn archive_index_flush_batches_actual_sql_and_preserves_order_and_metadata() {
    for size in [0usize, 1, 63, 64, 65, 129] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("batch.db");
        let mut conn = Connection::open(&path).unwrap();
        crate::schema::create_tables(&conn).unwrap();
        let mut archive = BodyArchive::open_for_tests(Some(&path), clock, &conn);
        for index in 0..size {
            let id = format!("{index:012x}");
            let body = format!("index-body-{index}");
            let mut blob = body_blob(&id, &body);
            blob.direction = if index % 2 == 0 { "request" } else { "response" };
            blob.content_type = (index % 2 == 0).then_some("text/plain");
            blob.original_bytes = Some(body.len() as u64 + 10);
            blob.trace_id = (index % 2 == 0).then_some("fixture-trace");
            blob.turn_id = (index % 3 == 0).then_some("fixture-turn");
            archive.stage(&conn, blob);
        }
        archive.flush_segment();
        INDEX_STATEMENTS.with(|count| count.set(0));
        conn.trace(Some(count_index_statements));
        commit(&mut archive, &conn);
        conn.trace(None);
        assert_eq!(INDEX_STATEMENTS.with(Cell::get), size.div_ceil(64));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row
                .get::<_, usize>(0))
                .unwrap(),
            size
        );
        for index in 0..size {
            let id = format!("{index:012x}");
            let body = format!("index-body-{index}");
            let actual: IndexMetadata = conn.query_row(
                "SELECT id, event_type, direction, content_type, original_bytes, stored_bytes, truncated, body_hash, trace_id, turn_id, created_at FROM event_body_blobs WHERE event_id = ?1", [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?)),
            ).unwrap();
            assert_eq!(actual.0, index as i64 + 1);
            assert_eq!(actual.1, "http.request");
            assert_eq!(actual.2, if index % 2 == 0 { "request" } else { "response" });
            assert_eq!(actual.3.as_deref(), (index % 2 == 0).then_some("text/plain"));
            assert_eq!(
                (actual.4, actual.5, actual.6),
                (body.len() as i64 + 10, body.len() as i64, 1)
            );
            assert_eq!(actual.7, content_digest(body.as_bytes()));
            assert_eq!(actual.8.as_deref(), (index % 2 == 0).then_some("fixture-trace"));
            assert_eq!(actual.9.as_deref(), (index % 3 == 0).then_some("fixture-turn"));
            assert_eq!(actual.10, format_timestamp(clock()));
            assert_eq!(read_indexed(&conn, &path, &id), body.as_bytes());
        }
    }
}

#[test]
fn archive_index_batch_rolls_back_and_retries_all_original_rows() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.db");
    let conn = Connection::open(&path).unwrap();
    crate::schema::create_tables(&conn).unwrap();
    let mut archive = BodyArchive::open_for_tests(Some(&path), clock, &conn);
    for index in 0..130 {
        archive.stage(&conn, body_blob(&format!("{index:012x}"), "valid bytes"));
    }
    let mark = archive.staged_mark();
    let mut bad = body_blob("ffffffffffff", "unreferenced invalid row");
    bad.event_type = "invalid-event-type";
    archive.stage(&conn, bad);
    archive.flush_segment();
    let tx = conn.unchecked_transaction().unwrap();
    assert!(
        archive.commit_index_rows(&tx).is_err(),
        "SQL constraints still reject malformed rows"
    );
    drop(tx);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    archive.rollback_staged(mark);
    let tx = conn.unchecked_transaction().unwrap();
    archive.commit_index_rows(&tx).unwrap();
    drop(tx);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    commit(&mut archive, &conn);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        130
    );
    for index in 0..130 {
        assert_eq!(read_indexed(&conn, &path, &format!("{index:012x}")), b"valid bytes");
    }
    archive.sync();
}
