//! Bodies live in `session.bodies`; SQLite keeps the index that finds them.
//!
//! What these tests hold is the pair of invariants the split rests on: a body
//! written through the handle reads back through the handle, byte for byte,
//! and no index row is ever visible before the bytes it names are on disk.

use super::*;
use crate::db::BodyDirection;
use crate::events::{ExecEvent, ExecEventComplete};
use crate::writer::PREVIEW_BYTES;

use super::correctness::{make_correctness_security_event, make_correctness_tool_response_model_call};

/// A rule match's forensic payload averaged a kilobyte and peaked at 297 KB in
/// one real session, and every one of those bytes sat in the RAM mirror as
/// well as on disk. The row keeps what routes filter on; the payload is a body
/// like any other and belongs in the archive, fetched when someone asks.
#[tokio::test]
async fn security_rule_payload_is_archived_not_inlined() {
    let p = temp_db_path("security-rule-payload");
    let db = DbHandle::open(&p).expect("open handle");
    let mut event = make_correctness_security_event(&credential_reference("test", "not-a-real-secret"));
    event.event_id = "0123456789ab".into();
    event.event_json = r#"{"rule":"x"}"#.repeat(200);

    db.write(WriteOp::SecurityRuleEvent(event))
        .await
        .expect("write security rule event");
    db.flush().await.expect("flush");

    let columns = query_json(
        &db.query(
            "SELECT name FROM pragma_table_info('security_rule_events') ORDER BY name",
            &[],
        )
        .await
        .expect("read security_rule_events columns"),
    );
    let names: Vec<String> = columns["rows"]
        .as_array()
        .expect("column rows")
        .iter()
        .map(|row| row[0].as_str().expect("column name").to_string())
        .collect();
    assert!(
        !names.iter().any(|name| name == "event_json"),
        "the payload must leave SQLite, not sit in every row and every RAM mirror: {names:?}"
    );

    let body = db
        .read_body("0123456789ab", BodyDirection::Payload)
        .await
        .expect("read the archived payload")
        .expect("a rule match's payload is archived");
    assert_eq!(body.source_table, "security_rule_events");
    assert_eq!(body.content_type.as_deref(), Some("application/json"));
    assert!(
        body.bytes.starts_with(br#"{"rule""#),
        "the archived payload must be the payload that was written"
    );
    assert_eq!(body.bytes.len(), 2400, "and all of it, not a preview");
}

/// A route with a page of rules in hand needs a payload per row. Asking event
/// by event is a query and a blocking task each, and inflates the block that
/// holds them once per body; one call reads the page in archive order.
#[tokio::test]
async fn recent_bodies_reads_a_page_of_payloads_in_one_pass() {
    let p = temp_db_path("security-rule-payload-page");
    let db = DbHandle::open(&p).expect("open handle");
    let credential_ref = credential_reference("test", "not-a-real-secret");
    for index in 0..5 {
        let mut event = make_correctness_security_event(&credential_ref);
        event.event_id = format!("{index:012x}");
        event.event_json = format!(r#"{{"match":{index}}}"#);
        db.write(WriteOp::SecurityRuleEvent(event))
            .await
            .expect("write security rule event");
    }
    db.flush().await.expect("flush");

    let payloads = db
        .read_recent_bodies("security_rule_events", BodyDirection::Payload, 3)
        .await
        .expect("read the newest payloads");
    let mut seen: Vec<String> = payloads
        .iter()
        .map(|body| String::from_utf8(body.bytes.clone()).expect("payloads are JSON text"))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![r#"{"match":2}"#, r#"{"match":3}"#, r#"{"match":4}"#],
        "the limit must take the newest matches, not the oldest"
    );
    assert!(payloads.iter().all(|body| body.source_table == "security_rule_events"));
    assert_eq!(
        db.archive_blocks_inflated_for_tests(),
        1,
        "a page of payloads that share a block must cost one inflate"
    );
}

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
    assert_eq!(
        blocks, 4,
        "a burst of large bodies must seal without waiting for the flush interval"
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

    // A flush seals whatever is pending, so the bound after one is exact:
    // nothing at all, let alone the 50 MB of raw bodies that went in.
    let pending = db.pending_body_bytes_for_tests().await;
    assert_eq!(pending, 0, "a flush leaves no body bytes in the writer");

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
    // The load-bearing assertion: with the sync moved after the inserts, or
    // the index committed before its block is appended, this query comes back
    // empty and every bound below is vacuously true.
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

/// A flush that fails after the index rows are written rolls them back with
/// everything else. The blocks are already on the disk, so the retry that
/// exists for the table rows has to bring the bodies with it -- otherwise the
/// bytes are there and nothing names them, for the rest of the session.
#[tokio::test]
async fn a_failed_flush_retries_bodies_with_the_rows() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("bodies-flush-retry");
    let db = DbHandle::open(&p).expect("open handle");
    let body = "a body that survives a failed flush ".repeat(32);
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789c0",
        "retry.example",
        &body,
    )))
    .await
    .expect("write event");

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    db.flush()
        .await
        .expect_err("the injected failure must be reported, not swallowed");
    db.flush().await.expect("the next flush succeeds");
    crate::writer::fail_disk_flushes_for_tests(0);

    let stored = db
        .read_body("0123456789c0", BodyDirection::Response)
        .await
        .expect("read body")
        .expect("the retried flush indexes the body the first one rolled back");
    assert_eq!(stored.bytes, body.as_bytes());
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM event_body_blobs").await,
        1,
        "the retry must not double-index the body"
    );
}

/// An operation that stages a body and then fails its own row insert leaves
/// bytes in the pending block with no row left to name them. The writer still
/// has to let go of that block before it closes: the archive asserts it was
/// not dropped holding one, and the file must stay readable.
#[tokio::test]
async fn a_rejected_op_does_not_leave_the_archive_holding_a_block() {
    let p = temp_db_path("bodies-rejected-op");
    let db = DbHandle::open(&p).expect("open handle");

    // Nothing dirty to flush afterwards: the rejected op is the only work,
    // so only the pending bytes can bring the final flush back.
    db.flush().await.expect("settle the ledger first");

    let mut call = make_correctness_tool_response_model_call(&credential_reference("test", "bodies-reject"));
    call.event_id = Some("0123456789c1".into());
    call.tool_responses = vec![ToolResponseEntry {
        call_id: "tool-call-rejected".into(),
        content_preview: Some("a body staged by an op that will be rejected".repeat(8)),
        is_error: false,
        trace_id: None,
        // Refused by the tool_responses CHECK, after the bodies are staged.
        credential_ref: Some("not-a-credential-reference".into()),
    }];
    db.write(WriteOp::ModelCall(call)).await.expect("write is accepted");
    db.flush().await.expect("flush");
    drop(db);

    let reader = DbHandle::open_external_reader(&p).expect("reopen the ledger");
    reader.ready().await.expect("the ledger is still sound");
    // The block reached the file: a writer that closed still holding it
    // would leave the archive at its bare 16-byte header, and the archive's
    // own drop assertion -- which the writer thread swallows on join -- would
    // be the only other sign.
    let archive = std::fs::metadata(archive_path(&p)).expect("stat archive");
    assert!(
        archive.len() > 16,
        "the rejected op's bytes must be sealed at shutdown, not held; archive is {} bytes",
        archive.len()
    );
}

/// Exec output arrives already cut down: capsem-process caps it at the vsock
/// boundary and sends the true size beside it. The index row has to report
/// the size the output was cut from, or a reader is told a 5 MB build log was
/// 1 KB long and nothing anywhere says otherwise.
#[tokio::test]
async fn exec_output_rows_report_the_size_the_output_was_cut_from() {
    let p = temp_db_path("bodies-exec-true-size");
    let db = DbHandle::open(&p).expect("open handle");
    let excerpt = "x".repeat(1024);

    db.write(WriteOp::ExecEvent(ExecEvent {
        event_id: Some("0123456789c2".into()),
        timestamp: SystemTime::now(),
        exec_id: 90_210,
        command: "build everything".into(),
        source: "api".into(),
        trace_id: None,
        process_name: Some("bash".into()),
        credential_ref: None,
    }))
    .await
    .expect("write exec start");
    db.write(WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 90_210,
        exit_code: 0,
        duration_ms: 1,
        stdout_preview: Some(excerpt.clone()),
        stderr_preview: None,
        stdout_bytes: 5_000_000,
        stderr_bytes: 0,
        pid: Some(11),
    }))
    .await
    .expect("write exec completion");
    db.flush().await.expect("flush");

    let row = query_json(
        &db.query(
            "SELECT original_bytes, stored_bytes, truncated FROM event_body_blobs
             WHERE event_id = ? AND direction = 'stdout'",
            &[json!("0123456789c2")],
        )
        .await
        .expect("read the exec index row"),
    );
    assert_eq!(row["rows"][0][0], json!(5_000_000), "original_bytes is what ran");
    assert_eq!(row["rows"][0][1], json!(1024), "stored_bytes is what arrived");
    assert_eq!(row["rows"][0][2], json!(1), "and the row says so");

    let stored = db
        .read_body("0123456789c2", BodyDirection::Stdout)
        .await
        .expect("read exec stdout")
        .expect("the excerpt is archived");
    assert_eq!(stored.bytes, excerpt.as_bytes());
    assert!(stored.truncated);
    assert_eq!(stored.original_bytes, 5_000_000);
}

/// The archive's own hash covers a whole block, so an index row edited to
/// name a different span of the same valid block still resolves -- to someone
/// else's body. The row's hash is over the bytes it claims, and the read
/// checks it, so that edit surfaces as an error naming the event instead of
/// as a body served under the wrong id.
#[tokio::test]
async fn a_body_that_does_not_match_its_index_hash_fails_the_read() {
    let p = temp_db_path("bodies-hash-mismatch");
    let db = DbHandle::open(&p).expect("open handle");
    for (event_id, body) in [
        ("0123456789d0", "the first body, which is one length"),
        ("0123456789d1", "the second body, quite another length entirely"),
    ] {
        db.write(WriteOp::NetEvent(net_event_with_response(
            event_id,
            "mismatch.example",
            body,
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");
    drop(db);

    // Point the first event's row at the second event's bytes: same block,
    // real offsets, a body that is simply not the one the row names.
    let conn = rusqlite::Connection::open(&p).expect("open disk verifier");
    conn.execute(
        "UPDATE event_body_blobs
         SET body_offset = (SELECT body_offset FROM event_body_blobs WHERE event_id = ?2),
             body_len = (SELECT body_len FROM event_body_blobs WHERE event_id = ?2),
             stored_bytes = (SELECT stored_bytes FROM event_body_blobs WHERE event_id = ?2),
             original_bytes = (SELECT original_bytes FROM event_body_blobs WHERE event_id = ?2)
         WHERE event_id = ?1",
        rusqlite::params!["0123456789d0", "0123456789d1"],
    )
    .expect("repoint the index row");
    drop(conn);

    let reader = DbHandle::open_external_reader(&p).expect("reopen the ledger");
    let error = reader
        .read_body("0123456789d0", BodyDirection::Response)
        .await
        .expect_err("bytes that do not match the row must not be served as the row's body");
    assert!(
        error.contains("0123456789d0") && error.contains("hash"),
        "the failure must name the event and say what was checked: {error}"
    );
}

/// A flush that fails rolls back its block row and its index rows together,
/// and the next flush writes both again. What this holds is that the pair
/// survives the round trip intact: the retried index row still names a block
/// row that exists, which is what `pragma_foreign_key_check` answers -- and it
/// is the check, not the `PRAGMA foreign_keys` setting, that decides, so this
/// is proof and not a setting the writer happened to run under.
#[tokio::test]
async fn the_retry_survives_enforced_foreign_keys() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("bodies-foreign-keys");
    let db = DbHandle::open(&p).expect("open handle");
    let body = "a body indexed twice by a retried flush ".repeat(16);
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789d2",
        "foreign-keys.example",
        &body,
    )))
    .await
    .expect("write event");

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    db.flush().await.expect_err("the injected failure is reported");
    db.flush().await.expect("the retry succeeds");
    crate::writer::fail_disk_flushes_for_tests(0);

    let conn = rusqlite::Connection::open(&p).expect("open disk verifier");
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
        .expect("check keys");
    assert_eq!(violations, 0, "no index row may be left naming a deleted block");
    drop(conn);

    let stored = db
        .read_body("0123456789d2", BodyDirection::Response)
        .await
        .expect("read body")
        .expect("the retried body is still indexed");
    assert_eq!(stored.bytes, body.as_bytes());
}
