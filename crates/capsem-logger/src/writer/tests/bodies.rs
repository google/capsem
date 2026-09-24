//! What the writer puts in the body archive, and what it leaves in the row.
//!
//! One net event with an oversized request and an oversized response: the
//! display columns keep a preview, the index accounts for the original bytes,
//! and the archive holds exactly what the index says it holds.

use capsem_archive::ArchiveError;

use super::*;
use crate::writer::bodies::takes_the_archive_out_of_service;

fn active_generation_path(conn: &rusqlite::Connection, db_path: &std::path::Path) -> std::path::PathBuf {
    let state = crate::schema::archive_state(conn).unwrap();
    archive_path_for_db(db_path).join(state.header.generation_id.file_name())
}

fn archive_reader(conn: &rusqlite::Connection, db_path: &std::path::Path) -> capsem_archive::BodyLogReader {
    let state = crate::schema::archive_state(conn).unwrap();
    let directory = capsem_foundation::unix::contained::ContainedDir::open_root(&archive_path_for_db(db_path)).unwrap();
    capsem_archive::BodyLogReader::open_generation(&directory, state.header, state.committed_end).unwrap()
}

#[test]
fn recovered_writer_fences_then_collects_only_owned_orphan_generations() {
    use std::ffi::OsStr;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("recovery-gc.db");
    let writer = DbWriter::open(&db_path, 1).unwrap();
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let before = crate::schema::archive_state(&conn).unwrap();
    let archive_dir = archive_path_for_db(&db_path);
    let directory = capsem_foundation::unix::contained::ContainedDir::open_root(&archive_dir).unwrap();
    let orphan = capsem_archive::GenerationId::new_v4();
    let mut candidate =
        capsem_archive::BodyLogWriter::create_generation(&directory, before.header.archive_id, orphan).unwrap();
    candidate.sync().unwrap();
    drop(candidate);
    drop(directory.create_new_private_file(OsStr::new("operator-note")).unwrap());
    directory.sync().unwrap();

    drop(BodyArchive::open_existing(&db_path, SystemTime::now, &conn).unwrap());

    let after = crate::schema::archive_state(&conn).unwrap();
    assert_eq!(
        after.header, before.header,
        "recovery never elects a generation by scanning names"
    );
    assert_eq!(
        after.revision,
        before.revision + 1,
        "recovery commits a real revision fence"
    );
    assert!(
        !archive_dir.join(orphan.file_name()).exists(),
        "an exact unreferenced generation name is retryable garbage"
    );
    assert!(
        archive_dir.join("operator-note").exists(),
        "unknown entries are reported and retained"
    );
}

#[test]
fn net_event_stores_bounded_body_blobs_and_small_previews() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("body-blobs.db");
    let event_id = "abc123def456".to_string();
    let trace_id = "trace-body-blob".to_string();
    let request_body = format!("{{\"prompt\":\"{}\"}}", "r".repeat(MAX_FIELD_BYTES + 1024));
    let response_body = format!("event: message\ndata: {}\n\n", "s".repeat(MAX_BODY_BLOB_BYTES + 128));
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
                    request_body: Some(request_body.clone().into_bytes()),
                    response_body: Some(response_body.clone().into_bytes()),
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
    // The display columns are derived from the bodies staged below, not
    // supplied beside them: whatever the archive holds, the preview is its
    // first PREVIEW_BYTES and cannot disagree with it.
    assert_eq!(stored_request_preview, request_body[..PREVIEW_BYTES]);
    assert_eq!(stored_response_preview, response_body[..PREVIEW_BYTES]);

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
    let archive = archive_reader(&conn, &db_path);
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

#[test]
fn model_items_keep_previews_while_the_archive_keeps_full_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("model-body-previews.db");
    let prefix = "p".repeat(PREVIEW_BYTES);
    let requests = [
        format!("{prefix}{}", "a".repeat(4096)),
        format!("{prefix}{}", "b".repeat(4096)),
    ];
    let responses = [
        format!("{prefix}{}", "c".repeat(4096)),
        format!("{prefix}{}", "d".repeat(4096)),
    ];

    let writer = DbWriter::open(&db_path, 64).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        for (index, (request, response)) in requests.iter().zip(&responses).enumerate() {
            let WriteOp::ModelCall(mut call) = super::minimal_model_call("trace-body-previews") else {
                unreachable!()
            };
            call.event_id = Some(format!("abc123def45{index}"));
            call.request_bytes = request.len() as u64;
            call.request_body = Some(request.as_bytes().to_vec());
            call.response_bytes = response.len() as u64;
            call.response_body = Some(response.as_bytes().to_vec());
            call.text_content = Some(response.clone());
            writer.write(WriteOp::ModelCall(call)).await;
        }
        writer.flush().await;
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let items = conn
        .prepare(
            "SELECT kind, content, content_hash FROM model_items
             WHERE trace_id = 'trace-body-previews' AND kind IN ('request', 'response')
             ORDER BY model_call_id, kind",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(items.len(), 4);
    for (_, content, _) in &items {
        assert_eq!(content, &prefix, "SQLite keeps only the display preview");
    }
    assert_ne!(items[0].2, items[2].2, "request hashes cover bytes after the preview");
    assert_ne!(items[1].2, items[3].2, "response hashes cover bytes after the preview");

    let archive = archive_reader(&conn, &db_path);
    for (index, (request, response)) in requests.iter().zip(&responses).enumerate() {
        let event_id = format!("abc123def45{index}");
        for (direction, expected) in [("request", request), ("response", response)] {
            let reference = conn
                .query_row(
                    "SELECT block_offset, body_offset, body_len FROM event_body_blobs
                     WHERE event_id = ?1 AND direction = ?2",
                    rusqlite::params![event_id, direction],
                    |row| {
                        Ok(capsem_archive::BodyRef {
                            block_offset: row.get::<_, i64>(0)? as u64,
                            offset: row.get::<_, i64>(1)? as u32,
                            len: row.get::<_, i64>(2)? as u32,
                        })
                    },
                )
                .unwrap();
            assert_eq!(archive.read(reference).unwrap(), expected.as_bytes());
        }
    }
}

fn body_blob<'a>(event_id: &'a str, body: &'a str) -> EventBodyBlob<'a> {
    EventBodyBlob {
        event_id,
        event_type: "http.request",
        source_table: "net_events",
        direction: "request",
        content_type: None,
        body: Some(body.as_bytes()),
        original_bytes: None,
        trace_id: None,
        turn_id: None,
    }
}

/// The archive is flushed to the device before the index rows that name its
/// segments are inserted. A crash between the two must cost bodies, never
/// produce rows pointing at bytes the disk never received.
#[test]
fn appended_blocks_are_flushed_before_their_index_rows_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("flush-order.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("0f1f2f3f4f5f", "a body worth flushing"));
    archive.flush_segment();
    archive.commit_index_rows(&conn).unwrap();

    assert_eq!(
        archive.steps_for_tests(),
        ["sync", "commit"],
        "the segments reach the device before the rows that name them"
    );
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    archive.sync();
}

/// A body that cannot fit beside what the open block holds closes the block
/// and retries, rather than failing or panicking. Two 10 MiB bodies cannot
/// share a 16 MiB block, so they land in two.
#[test]
fn a_body_that_does_not_fit_the_open_block_closes_it_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("close-and-retry.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    // Different bytes: identical ones would be one body the open block
    // already holds, and never reach the close this test is about.
    let first = "y".repeat(MAX_BODY_BLOB_BYTES);
    let second = "z".repeat(MAX_BODY_BLOB_BYTES);
    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("aaaaaaaaaaa1", &first));
    archive.stage(&conn, body_blob("aaaaaaaaaaa2", &second));
    archive.flush_segment();
    archive.commit_index_rows(&conn).unwrap();
    archive.sync();

    let blocks: i64 = conn
        .query_row("SELECT COUNT(*) FROM body_blocks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(blocks, 2, "10 MiB twice cannot share one 16 MiB block");

    // Both bodies come back whole, which is what close-and-retry has to
    // preserve: the retry restarts the offsets in a new block.
    let reader = archive_reader(&conn, &db_path);
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
    assert_eq!(bodies, vec![first.into_bytes(), second.into_bytes()]);
}

/// A failed write takes the writer out of service and leaves the segments it
/// had already written holding their rows. Those rows can never be vouched
/// for, so the next commit drops them -- and, just as importantly, stops
/// counting them as work: otherwise every flush tick for the rest of the
/// session opens a transaction to do nothing in.
#[test]
fn a_poisoned_archive_drops_its_uncommitted_blocks_instead_of_retrying_forever() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("poisoned-commit.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("0b0b0b0b0b0b", "a body whose writer dies after it"));
    archive.flush_segment();
    assert_eq!(archive.appended_len_for_tests(), 1, "the segment is written");

    archive.poison_writer_for_tests();
    archive.commit_index_rows(&conn).unwrap();

    assert_eq!(
        archive.appended_len_for_tests(),
        0,
        "a segment nothing can vouch for is dropped, not kept for a retry that cannot work"
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
/// open block are unplaceable too and must not survive to be inserted.
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

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("0c0c0c0c0c0c", "a body staged before the writer died"));
    assert!(archive.has_work(), "the row and its bytes are pending");

    archive.give_up("stage");

    assert!(!archive.has_work(), "nothing may be left for a flush to insert or seal");
    archive.stage(&conn, body_blob("0d0d0d0d0d0d", "a body staged after"));
    assert!(!archive.has_work(), "and nothing new is accepted either");

    archive.commit_index_rows(&conn).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

/// The body in hand when the close before its retry fails has no index row
/// yet, so the count that close takes cannot include it. It used to fall through the
/// `?` on the retry and disappear: no row, no bytes, and no counter anywhere
/// saying a body was lost. That counter is the only sign a session's archive
/// gave up, so a body it cannot see is a body nobody can notice.
#[test]
fn the_body_lost_to_a_failed_close_is_counted() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("close-failure.db");
    // Two bodies at the logger's cap overflow a 16 MiB block, which is the
    // only thing that makes `stage` close and retry.
    // Two different bodies: a repeat would be served from the open block
    // and never force the close whose failure this test is about.
    let body = "b".repeat(MAX_BODY_BLOB_BYTES);
    let other = "c".repeat(MAX_BODY_BLOB_BYTES);
    // The archive checks its index against the file before opening, so it
    // needs a ledger to check even when the test never writes an index row.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    metrics::with_local_recorder(&recorder, || {
        let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
        archive.stage(&conn, body_blob("0e0e0e0e0e0e", &body));
        archive.fail_next_append_for_tests();
        archive.stage(&conn, body_blob("0f0f0f0f0f0f", &other));
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
        "the closed block's own row is counted where the write failed: {dropped:?}"
    );
    assert!(
        dropped.contains(&("stage_after_close".to_string(), 1)),
        "and the body being staged when it failed is counted too: {dropped:?}"
    );
}

/// The span map, driven directly: a repeat inside the open block appends
/// nothing and is counted -- across the segments of that block too, since a
/// flush leaves it open; bytes of the same length but different content are
/// not a repeat; and once the block closes, the same bytes are appended again,
/// into the next block, rather than pointed back across.
#[test]
fn identical_bytes_share_a_span_only_inside_the_open_block() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dedup-spans.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    metrics::with_local_recorder(&recorder, || {
        let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
        let payload = r#"{"event":"matched by three rules"}"#;

        archive.stage(&conn, body_blob("0e0e0e0e0e01", payload));
        let one_copy = archive.pending_bytes();
        assert_eq!(one_copy, payload.len());
        for event_id in ["0e0e0e0e0e02", "0e0e0e0e0e03"] {
            archive.stage(&conn, body_blob(event_id, payload));
        }
        assert_eq!(archive.pending_bytes(), one_copy, "repeats append nothing");

        // Same length, different bytes: the hash says so, and a length or a
        // prefix would not.
        let same_length: String = payload.chars().rev().collect();
        assert_eq!(same_length.len(), payload.len());
        archive.stage(&conn, body_blob("0e0e0e0e0e04", &same_length));
        assert_eq!(archive.pending_bytes(), one_copy * 2, "different content is stored");

        archive.flush_segment();
        assert_eq!(archive.pending_bytes(), 0);
        archive.stage(&conn, body_blob("0e0e0e0e0e05", payload));
        assert_eq!(
            archive.pending_bytes(),
            0,
            "a flush leaves the block open, and its spans reusable"
        );
        archive.close_block();
        archive.stage(&conn, body_blob("0e0e0e0e0e06", payload));
        assert_eq!(
            archive.pending_bytes(),
            payload.len(),
            "a closed block's span is never reused: the next block gets its own copy"
        );
        archive.flush_segment();
    });

    let deduplicated: Vec<u64> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == crate::writer::DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL)
        .map(|(_, _, _, value)| match value {
            DebugValue::Counter(count) => count,
            other => panic!("deduplicated bodies are a counter, not {other:?}"),
        })
        .collect();
    assert_eq!(
        deduplicated,
        vec![3],
        "the three repeats are counted, and nothing else is"
    );
}

/// Read one indexed body straight out of the archive, the way the handle
/// resolves a row.
fn read_indexed(conn: &rusqlite::Connection, db_path: &std::path::Path, event_id: &str) -> Vec<u8> {
    let reference = conn
        .query_row(
            "SELECT block_offset, body_offset, body_len FROM event_body_blobs WHERE event_id = ?1",
            [event_id],
            |row| {
                Ok(capsem_archive::BodyRef {
                    block_offset: row.get::<_, i64>(0)? as u64,
                    offset: row.get::<_, i64>(1)? as u32,
                    len: row.get::<_, i64>(2)? as u32,
                })
            },
        )
        .unwrap();
    archive_reader(conn, db_path).read(reference).unwrap()
}

fn commit(archive: &mut BodyArchive, conn: &rusqlite::Connection) {
    let tx = conn.unchecked_transaction().unwrap();
    archive.commit_index_rows(&tx).unwrap();
    tx.commit().unwrap();
    archive.index_rows_committed();
}

const CRASH_CASE_ENV: &str = "CAPSEM_ARCHIVE_PUBLICATION_CRASH_CASE";
const CRASH_DB_ENV: &str = "CAPSEM_ARCHIVE_PUBLICATION_CRASH_DB";

fn build_survivor_candidate(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
    generation: capsem_archive::GenerationId,
) -> (capsem_archive::BodyLogWriter, i64, i64) {
    let state = crate::schema::archive_state(conn).unwrap();
    let directory = capsem_foundation::unix::contained::ContainedDir::open_root(&archive_path_for_db(db_path)).unwrap();
    let (old_offset, disk_len): (i64, i64) = conn
        .query_row(
            "SELECT block_offset, disk_len FROM body_blocks ORDER BY block_offset DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mut source = std::fs::File::open(active_generation_path(conn, db_path)).unwrap();
    let mut candidate =
        capsem_archive::BodyLogWriter::create_generation(&directory, state.header.archive_id, generation).unwrap();
    candidate
        .copy_block_from(&mut source, old_offset as u64, disk_len as u64)
        .unwrap();
    candidate.sync().unwrap();
    directory.sync().unwrap();
    (candidate, old_offset, disk_len)
}

#[test]
fn publication_crash_child() {
    use std::io::Write as _;

    let Ok(case) = std::env::var(CRASH_CASE_ENV) else {
        return;
    };
    let db_path = std::path::PathBuf::from(std::env::var_os(CRASH_DB_ENV).expect("crash DB path"));
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let state = crate::schema::archive_state(&conn).unwrap();
    let directory =
        capsem_foundation::unix::contained::ContainedDir::open_root(&archive_path_for_db(&db_path)).unwrap();
    let generation = capsem_archive::GenerationId::new_v4();

    if case == "candidate-partial" {
        let mut file = directory
            .create_new_private_file(std::ffi::OsStr::new(&generation.file_name()))
            .unwrap();
        file.write_all(b"crash-partial").unwrap();
        file.sync_all().unwrap();
        directory.sync().unwrap();
        std::process::exit(73);
    }

    let (candidate, old_offset, disk_len) = build_survivor_candidate(&conn, &db_path, generation);
    drop(candidate);
    if matches!(
        case.as_str(),
        "candidate-durable" | "transaction-rolled-back" | "commit-unknown-g"
    ) {
        std::process::exit(73);
    }

    let tx = conn.unchecked_transaction().unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys = ON").unwrap();
    tx.execute("DELETE FROM event_body_blobs WHERE event_id = 'aa0000000001'", [])
        .unwrap();
    tx.execute("DELETE FROM body_blocks WHERE block_offset <> ?1", [old_offset])
        .unwrap();
    tx.execute(
        "UPDATE body_blocks SET block_offset = ?1 WHERE block_offset = ?2",
        rusqlite::params![capsem_archive::FILE_HEADER_BYTES as i64, old_offset],
    )
    .unwrap();
    tx.execute(
        "UPDATE event_body_blobs SET block_offset = ?1 WHERE block_offset = ?2",
        rusqlite::params![capsem_archive::FILE_HEADER_BYTES as i64, old_offset],
    )
    .unwrap();
    tx.execute(
        "UPDATE archive_state SET generation_id = ?1, committed_end = ?2, revision = revision + 1
         WHERE singleton = 1",
        rusqlite::params![
            generation.as_bytes().as_slice(),
            capsem_archive::FILE_HEADER_BYTES as i64 + disk_len
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    if case == "during-gc" {
        directory
            .remove_private_file(std::ffi::OsStr::new(&state.header.generation_id.file_name()))
            .unwrap();
        directory.sync().unwrap();
    }
    std::process::exit(73);
}

/// Seven subprocess deaths cover the distinct recoverable publication states:
/// partial and durable candidates, rollback, both possible uncertain-COMMIT
/// elections, committed-before-adoption, and interrupted GC. Each recovered
/// ledger verifies exact old/survivor bytes and appends to the SQL-elected
/// generation before the next case begins.
#[test]
fn publication_crash_boundaries_recover_and_append_to_the_elected_generation() {
    let executable = std::env::current_exe().unwrap();
    for (case, published) in [
        ("candidate-partial", false),
        ("candidate-durable", false),
        ("transaction-rolled-back", false),
        ("commit-unknown-g", false),
        ("commit-unknown-h", true),
        ("published-before-adopt", true),
        ("during-gc", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join(format!("{case}.db"));
        let writer = DbWriter::open(&db_path, 1).unwrap();
        writer.shutdown_blocking();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut archive = BodyArchive::open_existing(&db_path, SystemTime::now, &conn).unwrap();
        archive.stage(&conn, body_blob("aa0000000001", "old bytes"));
        archive.close_block();
        commit(&mut archive, &conn);
        archive.stage(&conn, body_blob("aa0000000002", "survivor bytes"));
        archive.close_block();
        commit(&mut archive, &conn);
        let initial = crate::schema::archive_state(&conn).unwrap();
        drop(archive);
        drop(conn);

        let status = std::process::Command::new(&executable)
            .args([
                "writer::tests::bodies::publication_crash_child",
                "--exact",
                "--nocapture",
            ])
            .env(CRASH_CASE_ENV, case)
            .env(CRASH_DB_ENV, &db_path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{case} reached its crash boundary");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let elected = crate::schema::archive_state(&conn).unwrap();
        assert_eq!(
            elected.header.generation_id != initial.header.generation_id,
            published,
            "{case}"
        );
        let mut archive = BodyArchive::open_existing(&db_path, SystemTime::now, &conn).unwrap();
        let fenced = crate::schema::archive_state(&conn).unwrap();
        assert_eq!(fenced.revision, elected.revision + 1, "{case} recovery fence");

        if published {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM event_body_blobs WHERE event_id = 'aa0000000001'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                0,
                "{case} keeps the published deletion"
            );
        } else {
            assert_eq!(read_indexed(&conn, &db_path, "aa0000000001"), b"old bytes", "{case}");
        }
        assert_eq!(
            read_indexed(&conn, &db_path, "aa0000000002"),
            b"survivor bytes",
            "{case}"
        );
        archive.stage(&conn, body_blob("aa0000000003", "appended after recovery"));
        archive.close_block();
        commit(&mut archive, &conn);
        assert_eq!(
            read_indexed(&conn, &db_path, "aa0000000003"),
            b"appended after recovery",
            "{case} appends to the elected generation"
        );
        let hash: String = conn
            .query_row(
                "SELECT body_hash FROM event_body_blobs WHERE event_id = 'aa0000000002'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hash, blake3_bytes_ref(b"survivor bytes"), "{case} exact retained hash");
    }
}

/// A crash after a segment reached the file and before its rows committed:
/// the process is gone, the bytes are unreferenced, and the next writer to
/// open the ledger finds an index that still fits the file, starts a new
/// block after the stranded one, and every committed body still reads.
#[test]
fn a_crash_between_a_segment_and_its_commit_strands_bytes_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash-before-commit.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("0a0a0a0a0a01", "committed before the crash"));
    archive.flush_segment();
    commit(&mut archive, &conn);
    archive.stage(&conn, body_blob("0a0a0a0a0a02", "written, never committed"));
    archive.flush_segment();
    // The crash: the segment is in the file, its row never commits.
    drop(archive);
    let stranded_end = std::fs::metadata(active_generation_path(&conn, &db_path))
        .unwrap()
        .len();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), SystemTime::now, &conn);
    archive.stage(&conn, body_blob("0a0a0a0a0a03", "after the restart"));
    archive.flush_segment();
    commit(&mut archive, &conn);

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2, "the uncommitted body has no row");
    let new_block: i64 = conn
        .query_row(
            "SELECT block_offset FROM event_body_blobs WHERE event_id = '0a0a0a0a0a03'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(new_block as u64, stranded_end, "a new block, after the stranded bytes");
    assert_eq!(
        read_indexed(&conn, &db_path, "0a0a0a0a0a01"),
        b"committed before the crash"
    );
    assert_eq!(read_indexed(&conn, &db_path, "0a0a0a0a0a03"), b"after the restart");
}

/// Seconds since the epoch the test clock reads, so a test can move time.
static TEST_CLOCK_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_800_000_000);

fn test_clock() -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(TEST_CLOCK_SECS.load(std::sync::atomic::Ordering::SeqCst))
}

fn advance_test_clock(by: std::time::Duration) {
    TEST_CLOCK_SECS.fetch_add(by.as_secs(), std::sync::atomic::Ordering::SeqCst);
}

/// A quiet session must not keep one block open for days: retention drops
/// whole blocks by their last write, so a block that never closed would keep
/// every body in it for as long as anything new joined it. A block older
/// than `MAX_BLOCK_AGE` closes, and the next body starts a new one.
#[test]
fn a_block_open_longer_than_its_age_limit_closes_before_the_next_body() {
    use crate::writer::bodies::MAX_BLOCK_AGE;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("block-age.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::schema::create_tables(&conn).unwrap();

    let mut archive = BodyArchive::open_for_tests(Some(&db_path), test_clock, &conn);
    archive.stage(&conn, body_blob("0a0a0a0a0b01", "opens the block"));
    archive.flush_segment();
    advance_test_clock(MAX_BLOCK_AGE.saturating_sub(std::time::Duration::from_secs(60)));
    archive.stage(&conn, body_blob("0a0a0a0a0b02", "still inside the age limit"));
    archive.flush_segment();
    advance_test_clock(std::time::Duration::from_secs(120));
    archive.stage(&conn, body_blob("0a0a0a0a0b03", "past it"));
    archive.flush_segment();
    commit(&mut archive, &conn);

    let blocks: Vec<i64> = conn
        .prepare("SELECT block_offset FROM event_body_blobs ORDER BY event_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blocks[0], blocks[1], "inside the limit, one block");
    assert!(blocks[2] > blocks[1], "past it, a new block: {blocks:?}");
    assert_eq!(read_indexed(&conn, &db_path, "0a0a0a0a0b03"), b"past it");

    // And an idle block closes at the next flush, not only at the next body.
    advance_test_clock(MAX_BLOCK_AGE);
    archive.flush_segment();
    assert_eq!(archive.appended_len_for_tests(), 1, "the stale block closed");
    commit(&mut archive, &conn);
}
