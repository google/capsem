//! A block stays open across disk flushes and grows by one segment each.
//!
//! What these hold, through the handle: a body is readable -- by this handle
//! and by an external one in another process's shoes -- the moment its flush
//! returns, while the block it is in keeps growing; the index records that
//! growth under one offset and never shrinks it on a retry; a block closes at
//! its target size and at shutdown; retention closes the open block and
//! writing carries on after it; and an archive of an earlier format is left
//! alone rather than appended to.

use super::bodies::{archive_path, count, net_event_with_response};
use super::*;
use crate::db::BodyDirection;

/// `(block_offset, raw_len, disk_len)` of every block, oldest first.
async fn extents(db: &DbHandle) -> Vec<(u64, u64, u64)> {
    let value = query_json(
        &db.query(
            "SELECT block_offset, raw_len, disk_len FROM body_blocks ORDER BY block_offset",
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
            let field = |index: usize| row[index].as_u64().expect("block column");
            (field(0), field(1), field(2))
        })
        .collect()
}

async fn response(db: &DbHandle, event_id: &str) -> Option<Vec<u8>> {
    db.read_body(event_id, "net_events", BodyDirection::Response)
        .await
        .expect("read body")
        .map(|stored| stored.bytes)
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).expect("stat archive").len()
}

#[tokio::test]
async fn a_body_reads_from_an_external_handle_while_its_block_is_still_open() {
    let p = temp_db_path("open-block-external-read");
    let db = DbHandle::open(&p).expect("open handle");
    let external = DbHandle::open_external_reader(&p).expect("open external reader");
    let mut previous: Option<(u64, u64, u64)> = None;
    for round in 0..4 {
        let event_id = format!("00000000000{round}");
        let body = format!("round {round}: ").repeat(200);
        db.write(WriteOp::NetEvent(net_event_with_response(
            &event_id,
            "open-block.example",
            &body,
        )))
        .await
        .expect("write event");
        db.flush().await.expect("flush");

        assert_eq!(
            response(&external, &event_id).await.as_deref(),
            Some(body.as_bytes()),
            "round {round}: readable from outside as soon as the flush returns"
        );
        let blocks = extents(&db).await;
        assert_eq!(blocks.len(), 1, "round {round}: still one open block");
        let (offset, raw_len, disk_len) = blocks[0];
        assert_eq!(
            offset + disk_len,
            file_len(&archive_path(&p)),
            "the index covers exactly what was written"
        );
        if let Some((old_offset, old_raw, old_disk)) = previous {
            assert_eq!(offset, old_offset, "the block keeps its offset");
            assert!(raw_len > old_raw && disk_len > old_disk, "and grows");
        }
        previous = Some((offset, raw_len, disk_len));
    }
    // Every earlier body still reads from the grown block.
    assert_eq!(
        response(&external, "000000000000").await.as_deref(),
        Some("round 0: ".repeat(200).as_bytes())
    );
}

/// A flush that fails after its segment reached the file rolls back the
/// rows and the block's new extent together; the retry writes the rows and
/// records the extent once, at its full length.
#[tokio::test]
async fn a_retried_flush_records_the_grown_extent_once() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    let p = temp_db_path("open-block-retried-flush");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0000000000a1",
        "retry.example",
        &"first ".repeat(300),
    )))
    .await
    .expect("write first");
    db.flush().await.expect("first flush");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0000000000a2",
        "retry.example",
        &"second ".repeat(300),
    )))
    .await
    .expect("write second");

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    db.flush().await.expect_err("the injected failure is reported");
    db.flush().await.expect("the retry succeeds");
    crate::writer::fail_disk_flushes_for_tests(0);

    let blocks = extents(&db).await;
    assert_eq!(blocks.len(), 1);
    let (offset, _, disk_len) = blocks[0];
    assert_eq!(offset + disk_len, file_len(&archive_path(&p)));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM event_body_blobs").await, 2);
    assert_eq!(
        response(&db, "0000000000a2").await.as_deref(),
        Some("second ".repeat(300).as_bytes())
    );
}

/// A block closes once it holds its target, and the next body opens a new
/// one; at shutdown the open block is closed and that close is indexed.
#[tokio::test]
async fn a_block_closes_at_its_target_and_at_shutdown() {
    let p = temp_db_path("open-block-closes");
    let body_len = capsem_archive::TARGET_BLOCK_BYTES / 2 + 1;
    {
        let db = DbHandle::open(&p).expect("open handle");
        for (index, fill) in ["a", "b", "c"].iter().enumerate() {
            db.write(WriteOp::NetEvent(net_event_with_response(
                &format!("0000000000b{index}"),
                "closes.example",
                &fill.repeat(body_len),
            )))
            .await
            .expect("write event");
            db.flush().await.expect("flush");
        }
        let blocks = extents(&db).await;
        assert_eq!(blocks.len(), 2, "two bodies fill a block; the third opens the next");
    }

    // Reopened after shutdown: the last block ends in a FINAL segment, so a
    // span past its end is out of range rather than bytes not yet written.
    let db = DbHandle::open_external_reader(&p).expect("reopen");
    let blocks = extents(&db).await;
    let (offset, raw_len, disk_len) = *blocks.last().expect("a last block");
    assert_eq!(offset + disk_len, file_len(&archive_path(&p)), "the close is indexed");
    let reader = capsem_archive::BodyLogReader::open(&archive_path(&p)).expect("open archive");
    let past_the_end = capsem_archive::BodyRef {
        block_offset: offset,
        offset: u32::try_from(raw_len).expect("raw length fits"),
        len: 1,
    };
    assert!(matches!(
        reader.read(past_the_end),
        Err(capsem_archive::ArchiveError::RefOutOfRange)
    ));
    assert_eq!(
        response(&db, "0000000000b2").await.as_deref(),
        Some("c".repeat(body_len).as_bytes())
    );
}

/// Retention closes the open block before it compacts, and the writer goes on
/// in a new block at the end of the compacted file.
#[tokio::test]
async fn retention_closes_the_open_block_and_writing_carries_on() {
    let p = temp_db_path("open-block-retention");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0000000000c1",
        "retention.example",
        &"kept ".repeat(100),
    )))
    .await
    .expect("write kept");
    db.flush().await.expect("flush");

    let outcome = db
        .retain_bodies_since("1970-01-01T00:00:00.000000Z")
        .await
        .expect("retain nothing");
    assert_eq!(outcome, crate::writer::RetainOutcome::default());

    db.write(WriteOp::NetEvent(net_event_with_response(
        "0000000000c2",
        "retention.example",
        &"after ".repeat(100),
    )))
    .await
    .expect("write after");
    db.flush().await.expect("flush");

    let blocks = extents(&db).await;
    assert_eq!(blocks.len(), 2, "the retained block was closed; a new one follows it");
    assert_eq!(blocks[1].0, blocks[0].0 + blocks[0].2, "right after the kept extent");
    assert_eq!(
        response(&db, "0000000000c1").await.as_deref(),
        Some("kept ".repeat(100).as_bytes())
    );
    assert_eq!(
        response(&db, "0000000000c2").await.as_deref(),
        Some("after ".repeat(100).as_bytes())
    );
}

/// Retention closes the open block before it asks how old each block is. The
/// close carries no bodies, so it must not make the block look new: a block
/// whose bodies are all older than the cutoff goes, open or not.
#[tokio::test]
async fn closing_an_idle_block_for_retention_does_not_make_it_newer() {
    let p = temp_db_path("open-block-idle-close");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0000000000e1",
        "idle.example",
        &"idle ".repeat(100),
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");
    let written_at = query_json(
        &db.query("SELECT sealed_at FROM body_blocks", &[])
            .await
            .expect("read the block's time"),
    )["rows"][0][0]
        .as_str()
        .expect("sealed at")
        .to_string();
    let mut cutoff = crate::writer::format_ledger_timestamp(SystemTime::now());
    while cutoff <= written_at {
        cutoff = crate::writer::format_ledger_timestamp(SystemTime::now());
    }

    let outcome = db.retain_bodies_since(&cutoff).await.expect("retain");
    assert_eq!(outcome.blocks_dropped, 1, "the block's bodies predate the cutoff");
    assert_eq!(outcome.rows_dropped, 1);
    assert_eq!(response(&db, "0000000000e1").await, None);
}

/// An archive of another version beside the ledger is refused and left alone.
#[tokio::test]
async fn an_archive_of_an_earlier_version_is_left_alone() {
    let p = temp_db_path("open-block-old-version");
    let mut header = capsem_archive::format::encode_file_header(
        capsem_archive::ArchiveId::new_v4(),
        capsem_archive::GenerationId::new_v4(),
    );
    header[8..10].copy_from_slice(&1u16.to_le_bytes());
    std::fs::write(archive_path(&p), header).expect("plant a version 1 header");

    let error = match DbHandle::open(&p) {
        Ok(_) => panic!("v2 archive must not be adopted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("legacy v2 archive"), "{error}");
    assert_eq!(std::fs::read(archive_path(&p)).expect("read archive"), header);
}
