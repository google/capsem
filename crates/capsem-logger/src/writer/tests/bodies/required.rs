use super::*;
use crate::writer::bodies::EventBodyBlob;

fn required<'a>(event_id: &'a str, bytes: &'a [u8]) -> EventBodyBlob<'a> {
    EventBodyBlob {
        event_id,
        event_type: "environment.revision",
        source_table: "environment_revisions",
        direction: "payload",
        content_type: Some("application/msgpack"),
        body: Some(bytes),
        original_bytes: None,
        trace_id: None,
        turn_id: None,
    }
}

fn flush(
    conn: &Connection,
    path: &Path,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<super::super::super::flush::FlushTimings> {
    crate::writer::flush::flush_dirty_tables_to_disk(
        conn,
        &mut BTreeSet::new(),
        &mut Default::default(),
        Some(path),
        bodies,
        &crate::counters::LedgerCounters::default(),
        true,
    )
}

#[test]
fn required_configuration_payloads_refuse_truncation_and_unavailable_archives() {
    let conn = Connection::open_in_memory().unwrap();
    schema::create_tables(&conn).unwrap();
    let mut disabled = BodyArchive::disabled(SystemTime::now);
    disabled.stage(&conn, required("aaaaaaaaaaaa", b"approved configuration"));
    assert!(
        disabled.required_status().is_err(),
        "missing required snapshot was accepted"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    bodies.stage(&conn, required("bbbbbbbbbbbb", &vec![42; MAX_BODY_BLOB_BYTES + 1]));
    assert!(bodies.required_status().is_err(), "required snapshot was truncated");
    assert!(flush(&conn, &path, &mut bodies).is_err());
}

#[test]
fn required_archive_append_failure_is_reported_by_the_flush_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    bodies.stage(&conn, required("cccccccccccc", b"approved configuration"));
    bodies.fail_next_append_for_tests();
    assert!(
        flush(&conn, &path, &mut bodies).is_err(),
        "lost required bytes acknowledged durable"
    );
    assert!(
        flush(&conn, &path, &mut bodies).is_err(),
        "later empty flush forgot required data loss"
    );
}

#[test]
fn required_payload_roundtrips_complete_binary_bytes_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    let bytes: Vec<u8> = (0u8..=250).cycle().take(1024 * 1024).collect();
    bodies.stage(&conn, required("dddddddddddd", &bytes));
    flush(&conn, &path, &mut bodies).unwrap();
    assert_eq!(read_indexed(&conn, &path, "dddddddddddd"), bytes);
    assert_eq!(
        conn.query_row(
            "SELECT original_bytes,stored_bytes,truncated FROM event_body_blobs WHERE event_id='dddddddddddd'",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?))
        )
        .unwrap(),
        (
            i64::try_from(bytes.len()).unwrap(),
            i64::try_from(bytes.len()).unwrap(),
            false
        )
    );
    drop(bodies);
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    let _recovered =
        BodyArchive::open_existing(&path, SystemTime::now, &conn, capsem_archive::ArchiveCodecs::default()).unwrap();
    assert_eq!(read_indexed(&conn, &path, "dddddddddddd"), bytes);
}

#[test]
fn required_payload_does_not_reuse_a_colliding_committed_fingerprint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    bodies.stage(&conn, required("eeeeeeeeeeee", &[b'A'; 1024]));
    flush(&conn, &path, &mut bodies).unwrap();
    let expected = [b'B'; 1024];
    // Simulate the fast-fingerprint lookup returning an unequal old span.
    conn.execute(
        "UPDATE event_body_blobs SET body_hash=?1 WHERE event_id='eeeeeeeeeeee'",
        [content_digest(&expected)],
    )
    .unwrap();
    bodies.stage(&conn, required("ffffffffffff", &expected));
    flush(&conn, &path, &mut bodies).unwrap();
    assert_eq!(read_indexed(&conn, &path, "ffffffffffff"), expected);
    let offsets: Vec<(i64, i64)> = conn
        .prepare("SELECT block_offset,body_offset FROM event_body_blobs ORDER BY event_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(offsets.len(), 2);
    assert_ne!(offsets[0], offsets[1]);
}

#[tokio::test]
async fn required_index_transaction_failure_is_retryable_without_false_durability() {
    let _fault = DISK_FLUSH_FAULT_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    let bytes = b"approved revision survives an index rollback";
    bodies.stage(&conn, required("111111111111", bytes));
    fail_disk_flushes_for_path_for_tests(&path, 1);
    assert!(flush(&conn, &path, &mut bodies).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    flush(&conn, &path, &mut bodies).unwrap();
    assert_eq!(read_indexed(&conn, &path, "111111111111"), bytes);
}

#[test]
fn required_sync_failure_is_reported_after_worker_completion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let conn = Connection::open(&path).unwrap();
    schema::create_tables(&conn).unwrap();
    let mut bodies = BodyArchive::open_for_tests(Some(&path), SystemTime::now, &conn);
    bodies.stage(&conn, required("222222222222", b"approved configuration"));
    bodies.fail_next_sync_for_tests();
    assert!(flush(&conn, &path, &mut bodies).is_err());
    assert!(flush(&conn, &path, &mut bodies).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
