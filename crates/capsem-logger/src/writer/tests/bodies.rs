//! What the writer puts in the body archive, and what it leaves in the row.
//!
//! One net event with an oversized request and an oversized response: the
//! display columns keep a preview, the index accounts for the original bytes,
//! and the archive holds exactly what the index says it holds.

use capsem_archive::ArchiveError;

use super::*;
use crate::writer::bodies::takes_the_archive_out_of_service;

#[test]
fn net_event_stores_bounded_body_blobs_and_small_previews() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("body-blobs.db");
    let event_id = "abc123def456".to_string();
    let trace_id = "trace-body-blob".to_string();
    let request_body = format!("{{\"prompt\":\"{}\"}}", "r".repeat(MAX_FIELD_BYTES + 1024));
    let request_preview = "{\"prompt\":\"short\"}".to_string();
    let response_body = format!("event: message\ndata: {}\n\n", "s".repeat(MAX_BODY_BLOB_BYTES + 128));
    let response_preview = "event: message\ndata: short\n\n".to_string();
    // The hash covers what the archive stores, which for this oversized body
    // is its first MAX_BODY_BLOB_BYTES and not the whole thing: a hash of
    // bytes nobody kept could never be checked against anything.
    let response_hash = blake3_bytes_ref(&response_body.as_bytes()[..MAX_BODY_BLOB_BYTES]);

    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            writer
                .write(WriteOp::NetEvent(crate::events::NetEvent {
                    event_id: Some(event_id.clone()),
                    timestamp: std::time::SystemTime::now(),
                    domain: "daily-cloudcode-pa.googleapis.com".into(),
                    port: 443,
                    decision: crate::events::Decision::Allowed,
                    process_name: Some("agy".into()),
                    pid: Some(1234),
                    method: Some("POST".into()),
                    path: Some("/v1internal:streamGenerateContent".into()),
                    query: None,
                    status_code: Some(200),
                    bytes_sent: request_body.len() as u64,
                    bytes_received: response_body.len() as u64,
                    duration_ms: 42,
                    matched_rule: Some("profiles.rules.ai_google_http_googleapis".into()),
                    request_headers: Some("content-type: application/json".into()),
                    response_headers: Some("content-type: text/event-stream".into()),
                    request_body_preview: Some(request_preview.clone()),
                    response_body_preview: Some(response_preview.clone()),
                    request_body_full: Some(request_body.clone()),
                    response_body_full: Some(response_body.clone()),
                    conn_type: Some("https-mitm".into()),
                    policy_mode: None,
                    policy_action: Some("allow".into()),
                    policy_rule: Some("profiles.rules.ai_google_http_googleapis".into()),
                    policy_reason: None,
                    trace_id: Some(trace_id.clone()),
                    credential_ref: None,
                }))
                .await;
            writer.flush().await;
        });
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (stored_request_preview, stored_response_preview): (String, String) = conn
        .query_row(
            "SELECT request_body_preview, response_body_preview FROM net_events WHERE event_id = ?1",
            [&event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_request_preview, request_preview);
    assert_eq!(stored_response_preview, response_preview);

    struct StoredBlob {
        direction: String,
        event_type: String,
        content_type: String,
        original_bytes: i64,
        stored_bytes: i64,
        truncated: i64,
        body_hash: String,
        body: Vec<u8>,
        trace_id: String,
    }

    // The bytes are in the archive; SQLite only says where. Reading them back
    // through the index is what proves the two halves agree.
    let archive = capsem_archive::BodyLogReader::open(&archive_path_for_db(&db_path)).unwrap();
    let blobs: Vec<StoredBlob> = conn
        .prepare(
            "SELECT direction, event_type, content_type, original_bytes, stored_bytes,
                    truncated, body_hash, block_offset, body_offset, body_len, trace_id
             FROM event_body_blobs
             WHERE event_id = ?1
             ORDER BY direction",
        )
        .unwrap()
        .query_map([&event_id], |row| {
            let reference = capsem_archive::BodyRef {
                block_offset: row.get::<_, i64>(7)? as u64,
                offset: row.get::<_, i64>(8)? as u32,
                len: row.get::<_, i64>(9)? as u32,
            };
            Ok(StoredBlob {
                direction: row.get(0)?,
                event_type: row.get(1)?,
                content_type: row.get(2)?,
                original_bytes: row.get(3)?,
                stored_bytes: row.get(4)?,
                truncated: row.get(5)?,
                body_hash: row.get(6)?,
                body: archive.read(reference).expect("archived body"),
                trace_id: row.get(10)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blobs.len(), 2);

    let request = blobs.iter().find(|blob| blob.direction == "request").unwrap();
    assert_eq!(request.event_type, "http.request");
    assert_eq!(request.content_type, "application/json");
    assert_eq!(request.original_bytes, request_body.len() as i64);
    assert_eq!(request.stored_bytes, request_body.len() as i64);
    assert_eq!(request.truncated, 0);
    assert_eq!(request.body_hash, blake3_bytes_ref(request_body.as_bytes()));
    assert_eq!(request.body, request_body.as_bytes());
    assert_eq!(request.trace_id, trace_id);

    let response = blobs.iter().find(|blob| blob.direction == "response").unwrap();
    assert_eq!(response.event_type, "http.request");
    assert_eq!(response.content_type, "text/event-stream");
    assert_eq!(response.original_bytes, response_body.len() as i64);
    assert_eq!(response.stored_bytes, MAX_BODY_BLOB_BYTES as i64);
    assert_eq!(response.truncated, 1);
    assert_eq!(response.body_hash, response_hash);
    assert_ne!(
        response.body_hash,
        blake3_bytes_ref(response_body.as_bytes()),
        "the row must not carry a hash of bytes the archive did not keep"
    );
    assert_eq!(response.body.len(), MAX_BODY_BLOB_BYTES);
    assert_eq!(&response.body, &response_body.as_bytes()[..MAX_BODY_BLOB_BYTES]);
    assert_eq!(response.trace_id, trace_id);
}

fn body_blob<'a>(event_id: &'a str, body: &'a str) -> EventBodyBlob<'a> {
    EventBodyBlob {
        event_id,
        event_type: "http.request",
        source_table: "net_events",
        direction: "request",
        content_type: None,
        body: Some(body),
        original_bytes: None,
        trace_id: None,
        turn_id: None,
    }
}

/// The archive is flushed to the device before the index rows that name its
/// blocks are inserted. A crash between the two must cost bodies, never
/// produce rows pointing at bytes the disk never received.
#[test]
fn appended_blocks_are_flushed_before_their_index_rows_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("flush-order.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open(Some(&db_path));
    archive.stage(body_blob("0f1f2f3f4f5f", "a body worth flushing"));
    archive.seal_pending();
    archive.commit_index_rows(&conn).unwrap();

    assert_eq!(
        archive.steps_for_tests(),
        ["sync", "commit"],
        "the blocks reach the device before the rows that name them"
    );
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    archive.sync();
}

/// A body that cannot fit beside what is already pending seals the block and
/// retries, rather than failing or panicking. Two 10 MiB bodies cannot share
/// a 16 MiB block, so they land in two.
#[test]
fn a_body_that_does_not_fit_the_pending_block_seals_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("seal-and-retry.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let big = "z".repeat(MAX_BODY_BLOB_BYTES);
    let mut archive = BodyArchive::open(Some(&db_path));
    archive.stage(body_blob("aaaaaaaaaaa1", &big));
    archive.stage(body_blob("aaaaaaaaaaa2", &big));
    archive.seal_pending();
    archive.commit_index_rows(&conn).unwrap();
    archive.sync();

    let blocks: i64 = conn
        .query_row("SELECT COUNT(*) FROM body_blocks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(blocks, 2, "10 MiB twice cannot share one 16 MiB block");

    // Both bodies come back whole, which is what seal-and-retry has to
    // preserve: the retry restarts the offsets in a new block.
    let reader = capsem_archive::BodyLogReader::open(&archive_path_for_db(&db_path)).unwrap();
    let mut statement = conn
        .prepare("SELECT block_offset, body_offset, body_len FROM event_body_blobs ORDER BY event_id")
        .unwrap();
    let bodies: Vec<Vec<u8>> = statement
        .query_map([], |row| {
            Ok(capsem_archive::BodyRef {
                block_offset: row.get::<_, i64>(0)? as u64,
                offset: row.get::<_, i64>(1)? as u32,
                len: row.get::<_, i64>(2)? as u32,
            })
        })
        .unwrap()
        .map(|reference| reader.read(reference.unwrap()).unwrap())
        .collect();
    assert_eq!(bodies.len(), 2);
    for body in bodies {
        assert_eq!(body, big.as_bytes());
    }
}

/// A failed append takes the writer out of service and leaves the blocks it
/// had already appended holding their rows. Those rows can never be vouched
/// for, so the next commit drops them -- and, just as importantly, stops
/// counting them as work: otherwise every flush tick for the rest of the
/// session opens a transaction to do nothing in.
#[test]
fn a_poisoned_archive_drops_its_uncommitted_blocks_instead_of_retrying_forever() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("poisoned-commit.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open(Some(&db_path));
    archive.stage(body_blob("0b0b0b0b0b0b", "a body whose writer dies after it"));
    archive.seal_pending();
    assert_eq!(archive.appended_len_for_tests(), 1, "the block is appended");

    archive.poison_writer_for_tests();
    archive.commit_index_rows(&conn).unwrap();

    assert_eq!(
        archive.appended_len_for_tests(),
        0,
        "a block nothing can vouch for is dropped, not kept for a retry that cannot work"
    );
    assert!(
        !archive.has_work(),
        "a poisoned archive must stop asking every flush for a transaction"
    );
    assert!(
        archive.steps_for_tests().is_empty(),
        "no flush happened, so none is recorded: {:?}",
        archive.steps_for_tests()
    );
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "and no row claims bytes the archive cannot stand behind");
}

/// A poisoned writer cannot place bytes any more, so rows staged against its
/// pending block are unplaceable too and must not survive to be inserted.
/// This is the half of that rule that decides; the half that acts is below.
#[test]
fn only_a_poisoned_writer_takes_the_archive_out_of_service() {
    assert!(takes_the_archive_out_of_service(&ArchiveError::Poisoned));
    assert!(
        !takes_the_archive_out_of_service(&ArchiveError::BodyTooLarge { len: 1, max: 0 }),
        "one body too large for a block is a refusal of that body, not of the archive"
    );
    assert!(
        !takes_the_archive_out_of_service(&ArchiveError::RefOutOfRange),
        "a writer that can still place bytes stays in service"
    );
}

/// And the half that acts: everything staged goes, the writer goes with it,
/// and nothing staged afterwards is accepted -- so no later flush can insert
/// a row naming bytes the file never received.
#[test]
fn giving_up_drops_the_rows_that_can_no_longer_be_placed() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("give-up.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open(Some(&db_path));
    archive.stage(body_blob("0c0c0c0c0c0c", "a body staged before the writer died"));
    assert!(archive.has_work(), "the row and its bytes are pending");

    archive.give_up("stage");

    assert!(!archive.has_work(), "nothing may be left for a flush to insert or seal");
    archive.stage(body_blob("0d0d0d0d0d0d", "a body staged after"));
    assert!(!archive.has_work(), "and nothing new is accepted either");

    archive.commit_index_rows(&conn).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

/// The body in hand when `seal_pending` fails its append has no index row yet,
/// so the count that seal takes cannot include it. It used to fall through the
/// `?` on the retry and disappear: no row, no bytes, and no counter anywhere
/// saying a body was lost. That counter is the only sign a session's archive
/// gave up, so a body it cannot see is a body nobody can notice.
#[test]
fn the_body_lost_to_a_failed_seal_is_counted() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("seal-failure.db");
    // Two bodies at the logger's cap overflow a 16 MiB block, which is the
    // only thing that makes `stage` seal and retry.
    let body = "b".repeat(MAX_BODY_BLOB_BYTES);

    metrics::with_local_recorder(&recorder, || {
        let mut archive = BodyArchive::open(Some(&db_path));
        archive.stage(body_blob("0e0e0e0e0e0e", &body));
        archive.fail_next_append_for_tests();
        archive.stage(body_blob("0f0f0f0f0f0f", &body));
    });

    let dropped: Vec<(String, u64)> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == DB_ARCHIVE_BODIES_DROPPED_TOTAL)
        .map(|(key, _, _, value)| {
            let reason = key
                .key()
                .labels()
                .find(|label| label.key() == "reason")
                .expect("every drop names a reason")
                .value()
                .to_string();
            let DebugValue::Counter(count) = value else {
                panic!("dropped bodies are a counter, not {value:?}");
            };
            (reason, count)
        })
        .collect();

    assert!(
        dropped.contains(&("append".to_string(), 1)),
        "the sealed block's own row is counted where the append failed: {dropped:?}"
    );
    assert!(
        dropped.contains(&("stage_after_seal".to_string(), 1)),
        "and the body being staged when it failed is counted too: {dropped:?}"
    );
}
