//! Retention drops archived bodies by age and leaves the rest readable.
//!
//! What these hold is the pair of properties the compaction rests on: a body
//! whose block survived reads back byte for byte at its new offset, and a body
//! whose block went away leaves no row behind pointing at where it used to be.

use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::db::BodyDirection;
use crate::writer::archive_path_for_db;

use super::correctness::make_correctness_security_event;

/// Write one security payload and flush, which seals exactly one block.
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
        db.read_body("0000000000ab", BodyDirection::Payload)
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
    let db = DbHandle::open(&p).expect("open handle");
    write_block(&db, "0000000000ab", r#"{"old":1}"#).await;
    write_block(&db, "0000000000cd", r#"{"new":2}"#).await;
    let sealed = blocks(&db).await;
    assert_eq!(sealed.len(), 2, "each flush seals its own block");
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
        .read_body("0000000000cd", BodyDirection::Payload)
        .await
        .expect("read the kept body")
        .expect("the newer body survives");
    assert_eq!(kept.bytes, br#"{"new":2}"#);
    assert!(db
        .read_body("0000000000ab", BodyDirection::Payload)
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
            .read_body(event_id, BodyDirection::Payload)
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
        .read_body("0000000000ab", BodyDirection::Payload)
        .await
        .expect("read the oldest body")
        .expect("it is still archived");
    assert_eq!(body.bytes, br#"{"old":1}"#);
}
