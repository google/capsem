//! Retention drops archived bodies by age and leaves the rest readable.
//!
//! What these hold is the pair of properties the compaction rests on: a body
//! whose block survived reads back byte for byte at its new offset, and a body
//! whose block went away leaves no row behind pointing at where it used to be.

use std::os::unix::fs::PermissionsExt;

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
    std::fs::metadata(archive_path_for_db(db_path))
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
    // Its own directory, because this test makes the directory unwritable and
    // the shared temp directory is not this test's to seal.
    let directory = tempfile::tempdir().expect("a private directory");
    let p = directory.path().join("session.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    let before = archive_len(&p);

    // The compaction writes its replacement as a sibling of the archive, so a
    // directory it cannot create one in fails it before anything is touched.
    let sealed_directory = directory.path();
    let original = std::fs::metadata(sealed_directory)
        .expect("stat the directory")
        .permissions();
    std::fs::set_permissions(sealed_directory, std::fs::Permissions::from_mode(0o500)).expect("seal the directory");
    let error = db.retain_bodies_since(&sealed[1].1).await.expect_err(
        "a compaction that cannot write its replacement must fail; \
         if this passes, the test is running with rights that ignore the mode",
    );
    std::fs::set_permissions(sealed_directory, original).expect("restore the directory");

    assert!(
        error.contains("could not be compacted"),
        "the failure must say the archive was not rewritten: {error}"
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
    let archive_before = std::fs::read(archive_path_for_db(&p)).expect("read the archive");

    fail_retention_for_path_for_tests(&p, RetentionFault::IndexTransaction);
    let error = db
        .retain_bodies_since(&sealed[1].1)
        .await
        .expect_err("the injected failure must fail the retention");

    assert!(
        error.contains("left the archive") && error.contains("untouched"),
        "the failure must say the archive was not replaced: {error}"
    );
    assert_eq!(
        std::fs::read(archive_path_for_db(&p)).expect("read the archive"),
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
    // This ledger's own staging only: the temp directory is shared, and other
    // tests have their own retentions in flight.
    let staging_prefix = format!(
        ".{}",
        archive_path_for_db(&p)
            .file_name()
            .expect("the archive has a name")
            .to_string_lossy()
    );
    assert!(
        std::fs::read_dir(p.parent().expect("a parent"))
            .expect("list the directory")
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(&staging_prefix)),
        "the staged replacement is removed with the staging"
    );
}

/// The one syscall the ordering cannot make atomic. The index has committed
/// the new offsets and the rename did not happen, so the file still holds the
/// old ones -- and the remap is reversed, which puts the ledger back to
/// readable rather than merely loud.
#[tokio::test]
async fn a_failed_rename_puts_the_old_offsets_back_and_every_body_still_reads() {
    let p = temp_db_path("retention-rename-failure");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    let archive_before = std::fs::read(archive_path_for_db(&p)).expect("read the archive");

    fail_retention_for_path_for_tests(&p, RetentionFault::Rename);
    let error = db
        .retain_bodies_since(&sealed[1].1)
        .await
        .expect_err("the injected failure must fail the retention");

    assert!(
        error.contains("the index was put back"),
        "the failure must say the ledger was restored, not merely that it broke: {error}"
    );
    assert_eq!(
        std::fs::read(archive_path_for_db(&p)).expect("read the archive"),
        archive_before,
        "the rename never happened, so the archive is the old file"
    );
    assert_eq!(
        blocks(&db).await,
        vec![sealed[1].clone()],
        "the surviving block is back at the offset the unreplaced file still holds it at"
    );
    // The point of the restore: this reads, rather than failing its hash check
    // against bytes that belong to the block the compaction would have dropped.
    let kept = db
        .read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the kept body")
        .expect("the newer body is still archived");
    assert_eq!(kept.bytes, br#"{"new":2}"#);
    // Its rows are gone and its bytes are unreferenced in the file, which is
    // the archive's documented cost -- and it was the body retention was asked
    // to forget, so this is the intended outcome reached by an unintended road.
    assert!(db
        .read_body("0000000000ab", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the dropped body")
        .is_none());
    assert_eq!(foreign_key_violations(&db).await, 0);
    db.ready().await.expect("a restored ledger is still a ready ledger");
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

/// The unrecoverable state, and the only response to it: stop archiving.
///
/// The rename failed *and* the index could not be put back, so the ledger
/// names offsets the file does not have. Appending more bodies into that
/// archive would add rows to a ledger whose existing ones already lie, and a
/// later reopen would carry the disagreement forward.
#[tokio::test]
async fn an_unrecoverable_retention_takes_the_archive_out_of_service() {
    let p = temp_db_path("retention-unrecoverable");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;

    fail_retention_for_path_for_tests(&p, RetentionFault::Rename);
    fail_retention_for_path_for_tests(&p, RetentionFault::Restore);
    let error = db
        .retain_bodies_since(&sealed[1].1)
        .await
        .expect_err("a retention that cannot be undone must fail");
    assert!(
        error.contains("no further bodies will be stored"),
        "the failure must say the archive is out of service: {error}"
    );

    // Written after the archive gave up: accepted as a ledger row, with no
    // body archived, rather than appended into a file the index disagrees with.
    write_block(&db, "0000000000ef", r#"{"after":3}"#).await;
    assert!(
        db.read_body("0000000000ef", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read the later body")
            .is_none(),
        "a retired archive stores no bodies, and says so through the drop counter"
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
        .open(archive_path_for_db(&p))
        .expect("open the archive")
        .set_len(capsem_archive::FILE_HEADER_BYTES as u64)
        .expect("truncate to the header");

    let db = DbHandle::open(&p).expect("the ledger still opens; it is the archive that is refused");
    write_block(&db, "0000000000cd", r#"{"after":2}"#).await;

    assert!(
        db.read_body("0000000000cd", "security_rule_events", BodyDirection::Payload)
            .await
            .expect("read the new body")
            .is_none(),
        "a writer that cannot trust the archive must not append into it"
    );
    assert_eq!(
        blocks(&db).await.len(),
        1,
        "and must not add a block row beside the one it refused to believe"
    );
}
