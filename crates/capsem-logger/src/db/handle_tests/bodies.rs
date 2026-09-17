//! Bodies live in `session.bodies`; SQLite keeps the index that finds them.
//!
//! What these tests hold is the pair of invariants the split rests on: a body
//! written through the handle reads back through the handle, byte for byte,
//! and no index row is ever visible before the bytes it names are on disk.

use super::*;
use crate::db::BodyDirection;
use crate::events::{ExecEvent, ExecEventComplete};
use crate::writer::{MAX_BODY_BLOB_BYTES, PREVIEW_BYTES};

fn archive_path(db_path: &std::path::Path) -> std::path::PathBuf {
    db_path.with_extension("bodies")
}

async fn count(db: &DbHandle, sql: &str) -> i64 {
    query_json(&db.query(sql, &[]).await.expect("count query"))["rows"][0][0]
        .as_i64()
        .expect("count column")
}

/// One net event with a response body and nothing else to archive.
fn net_event_with_response(event_id: &str, domain: &str, body: &str) -> NetEvent {
    let mut event = make_net_event(domain, Decision::Allowed);
    event.event_id = Some(event_id.to_string());
    event.response_headers = Some("content-type: application/json".into());
    event.response_body_full = Some(body.to_string());
    event
}

#[tokio::test]
async fn bodies_live_in_the_archive_and_read_back_through_the_handle() {
    let p = temp_db_path("bodies-in-archive");
    let db = DbHandle::open(&p).expect("open handle");
    let body = r#"{"stream":true}"#.repeat(500);

    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "archive.example",
        &body,
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");

    let stored = db
        .read_body("0123456789ab", BodyDirection::Response)
        .await
        .expect("read response body")
        .expect("the response body is archived");
    assert_eq!(stored.bytes, body.as_bytes(), "the archive must return the exact body");
    assert_eq!(stored.original_bytes, body.len() as u64);
    assert!(!stored.truncated);
    assert_eq!(stored.source_table, "net_events");

    let columns = query_json(
        &db.query(
            "SELECT name FROM pragma_table_info('event_body_blobs') ORDER BY name",
            &[],
        )
        .await
        .expect("read index columns"),
    );
    let names: Vec<String> = columns["rows"]
        .as_array()
        .expect("column rows")
        .iter()
        .map(|row| row[0].as_str().expect("column name").to_string())
        .collect();
    assert!(
        !names.iter().any(|name| name == "body"),
        "the index must not carry a second copy of the bytes: {names:?}"
    );
    assert!(
        names.iter().any(|name| name == "block_offset"),
        "the index must name the block its bytes are in: {names:?}"
    );

    assert!(
        archive_path(&p).exists(),
        "the archive file must sit beside the database"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM body_blocks").await, 1);
    assert!(
        db.read_body("0123456789ab", BodyDirection::Request)
            .await
            .expect("read request body")
            .is_none(),
        "an event with no request body must have no request row"
    );
}

#[tokio::test]
async fn read_bodies_returns_every_direction_with_one_inflate() {
    let p = temp_db_path("bodies-one-inflate");
    let db = DbHandle::open(&p).expect("open handle");

    let mut event = net_event_with_response("0123456789ac", "both.example", r#"{"answer":"yes"}"#);
    event.request_headers = Some("content-type: application/json".into());
    event.request_body_full = Some(r#"{"question":"is one inflate enough"}"#.into());
    db.write(WriteOp::NetEvent(event)).await.expect("write event");
    db.flush().await.expect("flush");

    let bodies = db.read_bodies("0123456789ac").await.expect("read both bodies");
    assert_eq!(bodies.len(), 2, "both directions must come back");
    assert_eq!(
        db.archive_blocks_inflated_for_tests(),
        1,
        "the two bodies of one exchange share a block and must cost one inflate"
    );
}

#[tokio::test]
async fn external_reader_reads_bodies_another_process_wrote() {
    let p = temp_db_path("bodies-external-reader");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    let body = "cross-process body ".repeat(64);
    writer
        .write(WriteOp::NetEvent(net_event_with_response(
            "0123456789ad",
            "external.example",
            &body,
        )))
        .await
        .expect("write event");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open external reader");
    reader.ready().await.expect("external reader ready");
    let stored = reader
        .read_body("0123456789ad", BodyDirection::Response)
        .await
        .expect("read body")
        .expect("the body another process wrote is readable");
    assert_eq!(stored.bytes, body.as_bytes());
}

#[tokio::test]
async fn every_index_row_points_inside_a_recorded_block() {
    let p = temp_db_path("bodies-inside-their-block");
    let db = DbHandle::open(&p).expect("open handle");

    let body_of = |i: usize| format!("body-{i}-").repeat(1 + i * 7);
    for i in 0..300 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{i:012x}"),
            "varied.example",
            &body_of(i),
        )))
        .await
        .expect("write event");
        if i % 37 == 0 {
            db.flush().await.expect("interleaved flush");
        }
    }
    db.flush().await.expect("final flush");

    let dangling = count(
        &db,
        "SELECT COUNT(*)
         FROM event_body_blobs AS b
         LEFT JOIN body_blocks AS k ON k.block_offset = b.block_offset
         WHERE k.block_offset IS NULL OR b.body_offset + b.body_len > k.raw_len",
    )
    .await;
    assert_eq!(
        dangling, 0,
        "every index row must name a recorded block and sit inside it"
    );

    for i in [0_usize, 36, 37, 299] {
        let stored = db
            .read_body(&format!("{i:012x}"), BodyDirection::Response)
            .await
            .expect("read body")
            .unwrap_or_else(|| panic!("body {i} is archived"));
        assert_eq!(stored.bytes, body_of(i).as_bytes(), "body {i} read back wrong");
    }
}

#[tokio::test]
async fn a_body_missing_from_the_file_fails_loudly_not_empty() {
    let p = temp_db_path("bodies-missing-file");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ae",
        "missing.example",
        "a body that will not survive",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");

    let reader = DbHandle::open_external_reader(&p).expect("open external reader");
    reader
        .read_body("0123456789ae", BodyDirection::Response)
        .await
        .expect("read body")
        .expect("the body is archived before the file loses it");
    drop(db);

    std::fs::OpenOptions::new()
        .write(true)
        .open(archive_path(&p))
        .expect("open archive")
        .set_len(16)
        .expect("truncate archive to its header");
    // The reader holds the block it just inflated; a file that may have been
    // rewritten under it is exactly what the reset exists for.
    reader.archive_reader_reset();

    let error = reader
        .read_body("0123456789ae", BodyDirection::Response)
        .await
        .expect_err("a body the file cannot produce is a broken ledger, not an empty one");
    assert!(error.contains("archive"), "{error}");
}

#[tokio::test]
async fn a_burst_of_large_bodies_seals_before_the_interval() {
    let p = temp_db_path("bodies-burst-seals");
    let db = DbHandle::open(&p).expect("open handle");

    let body = "b".repeat(200 * 1024);
    for i in 0..8 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{i:012x}"),
            "burst.example",
            &body,
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");

    // A block seals once it reaches 256 KiB of raw bodies, and a body is
    // never split across blocks: 200 KiB bodies therefore pair up, and eight
    // of them are four blocks. The point is that they are on disk at all --
    // the 5 s interval has not elapsed, and 1.6 MB of raw bodies is not
    // sitting in the writer thread.
    let blocks = count(&db, "SELECT COUNT(*) FROM body_blocks").await;
    assert!(
        blocks >= 4,
        "a burst of large bodies must seal without waiting for the flush interval; sealed {blocks}"
    );
}

#[tokio::test]
async fn pending_body_bytes_are_bounded_after_flush() {
    let p = temp_db_path("bodies-pending-bounded");
    let db = DbHandle::open(&p).expect("open handle");

    let body = "p".repeat(100 * 1024);
    for i in 0..500 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{i:012x}"),
            "bounded.example",
            &body,
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");

    let pending = db.pending_body_bytes_for_tests().await;
    assert!(
        pending <= MAX_BODY_BLOB_BYTES as u64,
        "the writer must not hold 50 MB of raw bodies; pending {pending}"
    );

    let longest = count(
        &db,
        "SELECT COALESCE(MAX(length(response_body_preview)), 0) FROM net_events",
    )
    .await;
    assert!(
        longest <= PREVIEW_BYTES as i64,
        "the display preview must stay a preview; longest {longest}"
    );
}

#[tokio::test]
async fn tool_response_and_exec_output_are_archived_and_previewed() {
    let p = temp_db_path("bodies-tool-and-exec");
    let db = DbHandle::open(&p).expect("open handle");
    let big = "t".repeat(64 * 1024);

    let mut call = make_correctness_tool_response_model_call(&credential_reference("test", "bodies-tool-not-a-secret"));
    call.event_id = Some("0123456789b0".into());
    call.tool_responses = vec![ToolResponseEntry {
        call_id: "tool-call-archive-1".into(),
        content_preview: Some(big.clone()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];
    db.write(WriteOp::ModelCall(call)).await.expect("write model call");

    db.write(WriteOp::ExecEvent(ExecEvent {
        event_id: Some("0123456789b1".into()),
        timestamp: SystemTime::now(),
        exec_id: 4242,
        command: "print a lot".into(),
        source: "api".into(),
        trace_id: Some("trace-exec-archive".into()),
        process_name: Some("bash".into()),
        credential_ref: None,
    }))
    .await
    .expect("write exec start");
    db.write(WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 4242,
        exit_code: 0,
        duration_ms: 12,
        stdout_preview: Some(big.clone()),
        stderr_preview: Some(big.clone()),
        stdout_bytes: big.len() as u64,
        stderr_bytes: big.len() as u64,
        pid: Some(99),
    }))
    .await
    .expect("write exec completion");
    db.flush().await.expect("flush");

    let tool_response_event_id = query_json(
        &db.query(
            "SELECT event_id, length(content_preview) FROM tool_responses WHERE call_id = ?",
            &[json!("tool-call-archive-1")],
        )
        .await
        .expect("read tool response row"),
    );
    let tool_event_id = tool_response_event_id["rows"][0][0]
        .as_str()
        .expect("tool response event_id")
        .to_string();
    assert_eq!(
        tool_response_event_id["rows"][0][1],
        json!(PREVIEW_BYTES),
        "the tool response column must keep a preview, not the body"
    );

    let archived = db.read_bodies(&tool_event_id).await.expect("read tool response body");
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].source_table, "tool_responses");
    assert_eq!(archived[0].direction, BodyDirection::Response);
    assert_eq!(archived[0].bytes, big.as_bytes());

    let exec_previews = query_json(
        &db.query(
            "SELECT length(stdout_preview), length(stderr_preview) FROM exec_events WHERE exec_id = 4242",
            &[],
        )
        .await
        .expect("read exec previews"),
    );
    assert_eq!(exec_previews["rows"][0][0], json!(PREVIEW_BYTES));
    assert_eq!(exec_previews["rows"][0][1], json!(PREVIEW_BYTES));

    for direction in [BodyDirection::Stdout, BodyDirection::Stderr] {
        let stored = db
            .read_body("0123456789b1", direction)
            .await
            .expect("read exec output")
            .unwrap_or_else(|| panic!("exec {} is archived", direction.as_str()));
        assert_eq!(stored.source_table, "exec_events");
        assert_eq!(stored.bytes, big.as_bytes());
    }
}

#[tokio::test]
async fn ledger_snapshot_copies_the_archive_the_db_references() {
    let p = temp_db_path("bodies-snapshot");
    let src_dir = p.with_extension("srcdir");
    let dst_dir = p.with_extension("dstdir");
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
    std::fs::create_dir_all(&src_dir).expect("create source session dir");

    let body = "a forked session still owns its bodies ".repeat(32);
    let db = DbHandle::open(&src_dir.join("session.db")).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789af",
        "snapshot.example",
        &body,
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");

    crate::snapshot_session_ledger(&src_dir, &dst_dir).expect("snapshot the ledger");

    let forked = DbHandle::open_external_reader(&dst_dir.join("session.db")).expect("open the forked ledger");
    forked.ready().await.expect("forked ledger ready");
    let stored = forked
        .read_body("0123456789af", BodyDirection::Response)
        .await
        .expect("read forked body")
        .expect("the fork carries the body its index references");
    assert_eq!(stored.bytes, body.as_bytes());

    drop(db);
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
}

/// A ledger whose table is gone is broken, not empty. The readiness contract
/// has to name what is missing, or the route that fails cannot say why.
#[tokio::test]
async fn a_dropped_main_table_fails_ready_loudly() {
    let p = temp_db_path("bodies-dropped-table");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(make_net_event("dropped.example", Decision::Allowed)))
        .await
        .expect("write event");
    db.flush().await.expect("flush");
    drop(db);

    rusqlite::Connection::open(&p)
        .expect("open disk verifier")
        .execute_batch("DROP TABLE body_blocks")
        .expect("drop the block table");

    let reader = DbHandle::open_external_reader(&p).expect("open external reader");
    let error = reader
        .ready()
        .await
        .expect_err("a missing ledger table must fail readiness");
    assert!(
        error.contains("body_blocks"),
        "the readiness failure must name the missing table: {error}"
    );
}

/// The ordering that makes a crash survivable: bytes first, index second.
#[tokio::test]
async fn index_rows_are_committed_only_after_their_block_is_appended() {
    let p = temp_db_path("bodies-bytes-before-index");
    let db = DbHandle::open(&p).expect("open handle");
    for i in 0..40 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{i:012x}"),
            "ordering.example",
            &"o".repeat(16 * 1024),
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");

    let archive_len = std::fs::metadata(archive_path(&p)).expect("stat archive").len();
    let blocks = query_json(
        &db.query(
            "SELECT DISTINCT k.block_offset, k.comp_len
             FROM event_body_blobs AS b
             JOIN body_blocks AS k ON k.block_offset = b.block_offset",
            &[],
        )
        .await
        .expect("read referenced blocks"),
    );
    let rows = blocks["rows"].as_array().expect("block rows");
    assert!(!rows.is_empty(), "the flush must have committed index rows");
    for row in rows {
        let block_offset = row[0].as_u64().expect("block offset");
        let comp_len = row[1].as_u64().expect("compressed length");
        let block_end = block_offset + capsem_archive::BLOCK_HEADER_BYTES as u64 + comp_len;
        assert!(
            block_end <= archive_len,
            "an index row points past the end of the archive: block {block_offset} ends at \
             {block_end}, file is {archive_len} bytes"
        );
    }
}

// net_events.request_body_preview / response_body_preview are documented as
// "compact display field only" (writer.rs), yet were capped at 256 KB and in
// a real 10-day session averaged 28 KB per row -- duplicating bytes already
// stored in full in the archive. This proves the writer now caps the display
// preview at PREVIEW_BYTES while the index still accounts for the exact
// original body.
#[tokio::test]
async fn previews_are_capped_but_blobs_keep_the_full_body() {
    let p = temp_db_path("previews-capped-blobs-full");
    let db = DbHandle::open(&p).expect("open handle");

    let big = "x".repeat(64 * 1024);
    let mut event = make_net_event("preview-cap.example", Decision::Allowed);
    event.event_id = Some("0123456789ab".into());
    event.response_body_preview = Some(big.clone());
    event.response_body_full = Some(big.clone());
    db.write(WriteOp::NetEvent(event)).await.expect("write event");
    db.flush().await.expect("flush");

    let preview_rows = query_json(
        &db.query(
            "SELECT length(response_body_preview) FROM net_events WHERE event_id = ?",
            &[json!("0123456789ab")],
        )
        .await
        .expect("query preview length"),
    );
    let n = preview_rows["rows"][0][0].as_i64().expect("preview length column");
    // The input is plain ASCII 'x' bytes, so the cap is exact, not just an
    // upper bound.
    assert_eq!(
        n, PREVIEW_BYTES as i64,
        "preview must be capped to exactly PREVIEW_BYTES ({})",
        PREVIEW_BYTES
    );

    let blob_rows = query_json(
        &db.query(
            "SELECT original_bytes, stored_bytes FROM event_body_blobs WHERE event_id = ? AND direction = 'response'",
            &[json!("0123456789ab")],
        )
        .await
        .expect("query blob sizes"),
    );
    assert_eq!(
        blob_rows["rows"][0][0],
        json!(64 * 1024),
        "original_bytes must be the full body size"
    );
    assert_eq!(
        blob_rows["rows"][0][1],
        json!(64 * 1024),
        "stored_bytes must be the full body size, not the capped preview"
    );
}
