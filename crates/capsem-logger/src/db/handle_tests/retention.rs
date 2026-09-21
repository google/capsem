//! Retention drops archived bodies by age and leaves the rest readable.
//!
//! What these hold is the pair of properties the compaction rests on: a body
//! whose block survived reads back byte for byte at its new offset, and a body
//! whose block went away leaves no row behind pointing at where it used to be.

use super::*;
use crate::db::BodyDirection;
use crate::writer::archive_path_for_db;
use crate::writer::{fail_retention_for_path_for_tests, RetentionFault};

use super::correctness::make_correctness_security_event;

/// Write one security payload and flush, which -- with every test here
/// closing its block at each flush -- is exactly one block. What these tests
/// hold is what retention does with separate blocks, not when one closes.
async fn write_block(db: &DbHandle, event_id: &str, payload: &str) {
    let mut event = make_correctness_security_event(&credential_reference("test", "not-a-real-secret"));
    event.event_id = event_id.to_string();
    event.event_json = payload.to_string();
    db.write(WriteOp::SecurityRuleEvent(event))
        .await
        .expect("write security rule event");
    db.flush().await.expect("flush");
}

/// `(block_offset, sealed_at)` for every block the index holds, oldest first.
async fn blocks(db: &DbHandle) -> Vec<(i64, String)> {
    let value = query_json(
        &db.query(
            "SELECT block_offset, sealed_at FROM body_blocks ORDER BY block_offset",
            &[],
        )
        .await
        .expect("read body_blocks"),
    );
    value["rows"]
        .as_array()
        .expect("block rows")
        .iter()
        .map(|row| {
            (
                row[0].as_i64().expect("block offset"),
                row[1].as_str().expect("sealed at").to_string(),
            )
        })
        .collect()
}

fn archive_len(db_path: &std::path::Path) -> u64 {
    std::fs::metadata(super::bodies::archive_path(db_path))
        .expect("the session archive exists")
        .len()
}

async fn foreign_key_violations(db: &DbHandle) -> usize {
    let value = query_json(
        &db.query("SELECT * FROM pragma_foreign_key_check", &[])
            .await
            .expect("run the foreign key check"),
    );
    value["rows"].as_array().expect("check rows").len()
}

async fn wait_for_capture(reached: std::sync::mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .expect("capture wait task")
        .expect("capture reached the generation-pinned point");
}

/// The SQL snapshot and open descriptor remain a matched G pair after H is
/// published and G is unlinked. This is the schedule that path reopening
/// cannot make safe.
#[tokio::test]
async fn a_pinned_body_capture_survives_publication_and_unlink() {
    let p = temp_db_path("retention-pinned-body");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let writer = DbHandle::open(&p).expect("open writer");
    write_block(&writer, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&writer, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&writer).await;
    let reader = DbHandle::open_external_reader(&p).expect("open reader");
    let (reached, resume) = crate::db::bodies::pause_next_archive_capture_for_tests(&p);
    let read = tokio::spawn(async move {
        reader
            .read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
            .await
    });
    wait_for_capture(reached).await;
    writer.retain_bodies_since(&sealed[1].1).await.expect("publish H");
    resume.send(()).expect("resume G capture");
    let body = read
        .await
        .expect("read task")
        .expect("captured read")
        .expect("survivor");
    assert_eq!(body.bytes, br#"{"new":2}"#);
}

/// Parameter chunking stays inside one snapshot. Publication between capture
/// and materialization cannot make later chunks switch to H.
#[tokio::test]
async fn a_chunked_capture_uses_one_generation_snapshot() {
    let p = temp_db_path("retention-pinned-chunks");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let writer = DbHandle::open(&p).expect("open writer");
    write_block(&writer, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&writer, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&writer).await;
    let reader = DbHandle::open_external_reader(&p).expect("open reader");
    let (reached, resume) = crate::db::bodies::pause_next_archive_capture_for_tests(&p);
    let read = tokio::spawn(async move {
        let mut ids = (0..998).map(|i| format!("{i:012x}")).collect::<Vec<_>>();
        ids.push("0000000000cd".into());
        let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
        reader
            .read_bodies_for_events(&refs, "security_rule_events", BodyDirection::Payload, 1024)
            .await
    });
    wait_for_capture(reached).await;
    writer.retain_bodies_since(&sealed[1].1).await.expect("publish H");
    resume.send(()).expect("resume chunked capture");
    let archived = read.await.expect("read task").expect("captured read");
    assert_eq!(archived.truncated_rows, 0);
    assert_eq!(archived.bodies.len(), 3, "each query chunk is materialized from G");
    let mut payloads = archived.bodies.into_iter().map(|body| body.bytes).collect::<Vec<_>>();
    payloads.sort();
    assert_eq!(
        payloads,
        vec![
            br#"{"new":2}"#.to_vec(),
            br#"{"new":2}"#.to_vec(),
            br#"{"old":1}"#.to_vec()
        ]
    );
}

/// WARC metadata is fully captured from G before output begins. Publishing H
/// and unlinking G while the capture is paused induces no skip and no hash
/// mismatch.
#[tokio::test]
async fn a_pinned_warc_capture_survives_publication_and_unlink() {
    let p = temp_db_path("retention-pinned-warc");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let writer = DbHandle::open(&p).expect("open writer");
    write_block(&writer, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&writer, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&writer).await;
    let reader = DbHandle::open_external_reader(&p).expect("open reader");
    let reader_path = p.clone();
    let (reached, resume) = crate::db::bodies::pause_next_archive_capture_for_tests(&p);
    let export = tokio::spawn(async move { super::warc_export::export_to_bytes(&reader, &reader_path).await });
    wait_for_capture(reached).await;
    writer.retain_bodies_since(&sealed[1].1).await.expect("publish H");
    resume.send(()).expect("resume WARC capture");
    let (summary, bytes) = export.await.expect("export task");
    assert!(summary.skipped.is_empty(), "publication must induce no WARC skips");
    let bodies = super::warc_export::body_members(&bytes);
    assert_eq!(bodies.len(), 2);
    assert_eq!(super::warc_export::block(&bodies[1]), br#"{"new":2}"#);
}

/// A handle that reads a ledger another process writes must not rewrite its
/// archive: `capsem-process` owns both halves, and two processes compacting
/// one file is the failure the single-writer rule exists to prevent.
#[tokio::test]
async fn external_reader_cannot_retain_bodies() {
    let p = temp_db_path("retention-external-reader");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let writer = DbHandle::open(&p).expect("open handle");
    write_block(&writer, "0000000000ab", r#"{"keep":true}"#).await;

    let reader = DbHandle::open_external_reader(&p).expect("open external reader");
    let error = reader
        .retain_bodies_since("2999-01-01T00:00:00Z")
        .await
        .expect_err("an external reader owns no writer and may not retain");

    assert!(
        error.contains("read-only") && error.contains("owning process"),
        "the refusal must name what this handle is and who may do it instead: {error}"
    );
    assert_eq!(
        blocks(&writer).await.len(),
        1,
        "the refusal must not have dropped anything"
    );
}

/// A cutoff past every block drops the whole archive: the file goes back to
/// its header and the index keeps no row naming bytes that are gone.
#[tokio::test]
async fn a_cutoff_past_everything_empties_the_archive() {
    let p = temp_db_path("retention-drops-everything");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"older":2}"#).await;
    let before = archive_len(&p);

    let outcome = db
        .retain_bodies_since("2999-01-01T00:00:00Z")
        .await
        .expect("retain bodies");

    assert_eq!(outcome.blocks_dropped, 2);
    assert_eq!(outcome.blocks_kept, 0);
    assert_eq!(outcome.rows_dropped, 2);
    assert_eq!(outcome.bytes_reclaimed, before - archive_len(&p));
    assert_eq!(
        archive_len(&p),
        capsem_archive::FILE_HEADER_BYTES as u64,
        "an archive with nothing left is its file header and nothing else"
    );
    assert!(blocks(&db).await.is_empty(), "no block row survives its bytes");
    assert!(
        db.read_body("0000000000ab", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read the dropped body")
            .is_none(),
        "a dropped body is absent, not an error and not someone else's bytes"
    );
    assert_eq!(foreign_key_violations(&db).await, 0);
}

/// The ordinary case: one block ages out, the other does not, and the bodies
/// in the survivor still read back through their moved offsets.
#[tokio::test]
async fn a_cutoff_between_two_blocks_keeps_the_newer_one() {
    let p = temp_db_path("retention-keeps-the-newer");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    assert_eq!(sealed.len(), 2, "each flush closes its own block here");
    assert_ne!(
        sealed[0].1, sealed[1].1,
        "the two flushes must be distinguishable in time for a cutoff to sit between them"
    );

    let outcome = db.retain_bodies_since(&sealed[1].1).await.expect("retain bodies");

    assert_eq!(outcome.blocks_dropped, 1);
    assert_eq!(outcome.blocks_kept, 1);
    assert_eq!(outcome.rows_dropped, 1);
    assert!(outcome.bytes_reclaimed > 0, "the dropped block's bytes are reclaimed");

    let survivors = blocks(&db).await;
    assert_eq!(
        survivors,
        vec![(capsem_archive::FILE_HEADER_BYTES as i64, sealed[1].1.clone())],
        "the surviving block moved up behind the file header and kept its seal time"
    );
    // `read_body` verifies blake3 over the span the index row named, so this
    // passing is the remap being right and not merely in range.
    let kept = db
        .read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the kept body")
        .expect("the newer body survives");
    assert_eq!(kept.bytes, br#"{"new":2}"#);
    assert!(db
        .read_body("0000000000ab", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the dropped body")
        .is_none());
    assert_eq!(foreign_key_violations(&db).await, 0);
    db.ready().await.expect("a retained ledger is still a ready ledger");
}

/// Retention is not the end of the session: the writer reopens the compacted
/// file, so a body written afterwards lands past the survivors and reads back.
#[tokio::test]
async fn writes_after_retention_append_to_the_compacted_archive() {
    let p = temp_db_path("retention-then-writes");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    db.retain_bodies_since(&sealed[1].1).await.expect("retain bodies");

    write_block(&db, "0000000000ef", r#"{"later":3}"#).await;

    let after = blocks(&db).await;
    assert_eq!(after.len(), 2, "the new block joins the survivor");
    assert!(
        after[1].0 > after[0].0,
        "the reopened writer appends after the compacted end rather than over it"
    );
    for (event_id, payload) in [("0000000000cd", r#"{"new":2}"#), ("0000000000ef", r#"{"later":3}"#)] {
        let body = db
            .read_body(event_id, "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read a body")
            .expect("both bodies are archived");
        assert_eq!(body.bytes, payload.as_bytes(), "{event_id} reads back byte for byte");
    }
    assert_eq!(foreign_key_violations(&db).await, 0);
}

/// A compaction that cannot even start must leave the ledger exactly as it
/// was. The index is what makes a body findable, so a half-retained state that
/// deleted rows for bytes still in the file would lose evidence the file still
/// holds.
#[tokio::test]
async fn a_failed_compaction_leaves_the_index_and_the_bodies_alone() {
    let p = temp_db_path("retention-sync-failure");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    let before = archive_len(&p);

    fail_retention_for_path_for_tests(&p, RetentionFault::CandidateSync);
    let error = db
        .retain_bodies_since(&sealed[1].1)
        .await
        .expect_err("a candidate that did not sync must not publish");

    assert!(
        error.contains("candidate-sync") && error.contains("not published"),
        "the failure must name the pre-publication phase: {error}"
    );
    assert_eq!(blocks(&db).await, sealed, "no index row moved or went away");
    assert_eq!(archive_len(&p), before, "and no byte left the archive");
    let body = db
        .read_body("0000000000ab", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the oldest body")
        .expect("it is still archived");
    assert_eq!(body.bytes, br#"{"old":1}"#);
}

/// The whole reason retention stages the file and renames it last: a failure
/// in the index transaction must leave the archive exactly as it was, with
/// every body still findable through the index that still names it.
#[tokio::test]
async fn a_failed_index_transaction_leaves_the_archive_and_the_index_untouched() {
    let p = temp_db_path("retention-index-failure");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    let archive_before = std::fs::read(super::bodies::archive_path(&p)).expect("read the archive");

    fail_retention_for_path_for_tests(&p, RetentionFault::IndexTransaction);
    let error = db
        .retain_bodies_since(&sealed[1].1)
        .await
        .expect_err("the injected failure must fail the retention");

    assert!(
        error.contains("sqlite-transaction") && error.contains("not published"),
        "the failure must say publication did not happen: {error}"
    );
    assert_eq!(
        std::fs::read(super::bodies::archive_path(&p)).expect("read the archive"),
        archive_before,
        "the archive is still the pre-retention file, byte for byte"
    );
    assert_eq!(blocks(&db).await, sealed, "and no index row moved or went away");
    for (event_id, payload) in [("0000000000ab", r#"{"old":1}"#), ("0000000000cd", r#"{"new":2}"#)] {
        let body = db
            .read_body(event_id, "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read a body")
            .expect("every body is still archived");
        assert_eq!(body.bytes, payload.as_bytes(), "{event_id} still reads");
    }
    assert_eq!(
        std::fs::read_dir(archive_path_for_db(&p))
            .expect("list generations")
            .count(),
        1,
        "the unpublished candidate is removed"
    );
}

/// The service reads session ledgers `capsem-process` writes, and holds its
/// archive reader open across a persistent VM's stop. Retention renames a
/// compacted file over the archive, so that reader is left on an inode where
/// every surviving block has moved.
#[tokio::test]
async fn an_external_reader_follows_the_archive_across_a_retention() {
    let p = temp_db_path("retention-external-reader-follows");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let writer = DbHandle::open(&p).expect("open handle");
    write_block(&writer, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&writer, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&writer).await;

    let reader = DbHandle::open_external_reader(&p).expect("open external reader");
    // Opens the archive and caches both the descriptor and the block.
    assert_eq!(
        reader
            .read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read a body")
            .expect("the body is archived")
            .bytes,
        br#"{"new":2}"#
    );

    writer.retain_bodies_since(&sealed[1].1).await.expect("retain bodies");

    let kept = reader
        .read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("a reader that notices the archive moved does not fail here")
        .expect("the surviving body is still archived");
    assert_eq!(
        kept.bytes, br#"{"new":2}"#,
        "the same read returns the same bytes from the compacted file"
    );
    assert!(
        reader
            .read_body("0000000000ab", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read the dropped body")
            .is_none(),
        "and a dropped body is absent rather than stale bytes from the old inode"
    );
}

/// `pragma_foreign_key_check` is only evidence if it can fail, and the other
/// retention tests never plant a violation for it to find. This one does --
/// an index row naming a block that is not there -- proves the check reports
/// it, removes it, and only then asserts that a retention leaves none.
///
/// What it cannot show is the remap surviving enforcement, because a test on
/// this side cannot see the transaction from inside. `writer/retention/tests.rs`
/// does that against `reindex` directly.
#[tokio::test]
async fn a_retention_leaves_no_orphan_index_rows() {
    let p = temp_db_path("retention-foreign-keys");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;

    // The check is a check: plant an orphan and watch it be found. Enforcement
    // is off for the insert, because with it on the insert is simply refused
    // -- which is the other half of the same proof.
    {
        let conn = rusqlite::Connection::open(&p).expect("open disk verifier");
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .expect("the planted orphan needs enforcement out of the way");
        conn.execute(
            "INSERT INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash,
                block_offset, body_offset, body_len, trace_id, turn_id, created_at
             ) VALUES ('deadbeef0000', 'security.rule', 'security_rule_events', 'payload', NULL,
                       1, 1, 0,
                       'blake3:0000000000000000000000000000000000000000000000000000000000000000',
                       999999, 0, 1, NULL, NULL, '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("an orphan row goes in while nothing is enforcing the reference");
        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
            .expect("check keys");
        assert_eq!(violations, 1, "the check must actually find a planted orphan");
        conn.execute("DELETE FROM event_body_blobs WHERE event_id = 'deadbeef0000'", [])
            .expect("remove the orphan");
    }

    db.retain_bodies_since(&sealed[1].1).await.expect("retain bodies");

    assert_eq!(
        foreign_key_violations(&db).await,
        0,
        "no index row may be left naming a block the remap moved or deleted"
    );
    assert_eq!(
        db.read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read the kept body")
            .expect("the newer body survives")
            .bytes,
        br#"{"new":2}"#
    );
}

/// Reopening a ledger whose index outran its archive.
///
/// This is what a crash inside retention's one-syscall window leaves behind,
/// and it is the state the writer used to reopen and append into: the index
/// names a block past the end of the file, so its rows resolve against bytes
/// that are not there or, worse, against a later block's.
#[tokio::test]
async fn an_index_naming_bytes_past_the_archive_refuses_to_open() {
    let p = temp_db_path("retention-index-past-eof");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    db.flush().await.expect("flush");
    drop(db);

    // The file as a failed retention would leave it: compacted away, with the
    // index still describing what used to be there.
    std::fs::OpenOptions::new()
        .write(true)
        .open(super::bodies::archive_path(&p))
        .expect("open the archive")
        .set_len(capsem_archive::FILE_HEADER_BYTES as u64)
        .expect("truncate to the header");

    let error = DbHandle::open(&p)
        .err()
        .expect("a writer must fail closed before readiness");
    assert!(
        error.to_string().contains("shorter") || error.to_string().contains("generation"),
        "the refusal names the invalid authoritative generation: {error}"
    );
}
