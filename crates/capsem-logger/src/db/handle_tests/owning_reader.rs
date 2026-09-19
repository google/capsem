//! A handle that owns its writer reads the file, like every other reader.
//!
//! It used to read the writer's memory schema through TEMP views, which is
//! what obliged the writer to keep every row of the session in RAM (#213).
//! Now the writer's memory holds only rows it has not flushed, so the owning
//! handle reads `main` and learns that its writer committed the same way an
//! external reader does: from SQLite's `data_version`.

use super::*;
use crate::schema::memory_row_count_for_tests as mem_rows;

#[tokio::test]
async fn owning_handle_attaches_no_memory_schema() {
    let p = temp_db_path("owning-no-memory-schema");
    let db = DbHandle::open(&p).expect("open owning handle");
    db.ready().await.expect("ready");
    let attached = db.attached_schemas_for_tests().await.expect("read attached schemas");
    assert!(
        !attached.iter().any(|name| name == crate::schema::MEMORY_SCHEMA),
        "the owning handle's reader must query the file, not the writer's memory; attached {attached:?}. \
         {DB_BOUNDARY_RATIONALE}"
    );
}

/// The writer's own interval flush is the commit nothing else announces: no
/// `write()` and no handle-level `flush()` invalidates the cache for it. The
/// batch cached before it must still be refused after it.
#[tokio::test]
async fn owning_handle_sees_its_writers_flush_without_another_write() {
    let p = temp_db_path("owning-sees-interval-flush");
    let db = DbHandle::open(&p).expect("open owning handle");
    db.ready().await.expect("ready");
    let batch = || vec![("SELECT COUNT(*) FROM net_events".to_string(), Vec::new())];

    db.write(WriteOp::NetEvent(make_net_event(
        "unflushed.example",
        Decision::Allowed,
    )))
    .await
    .expect("write accepted");
    let before = db.query_many(batch()).await.expect("poll before flush");
    assert_eq!(
        query_json(&before[0])["rows"],
        json!([[0]]),
        "an accepted row is not visible until the writer flushes it. {DB_BOUNDARY_RATIONALE}"
    );

    // The writer flushing on its own, as its interval does; the handle is not told.
    db.inner.writer.as_ref().expect("owning writer").flush().await;

    let after = db.query_many(batch()).await.expect("poll after flush");
    assert_eq!(
        query_json(&after[0])["rows"],
        json!([[1]]),
        "the handle must notice its writer's commit instead of serving the batch cached before it. \
         {DB_BOUNDARY_RATIONALE}"
    );
}

/// A flush that rolls back takes its prune with it: the rows stay in memory,
/// the retry copies them once, and their bodies are indexed once.
#[tokio::test]
async fn a_rolled_back_flush_keeps_its_rows_for_the_retry() {
    let _guard = DB_FLUSH_FAILURE_TEST_LOCK.lock().await;
    crate::writer::fail_disk_flushes_for_tests(0);

    let p = temp_db_path("rolled-back-flush-prune");
    let db = DbHandle::open(&p).expect("open handle");
    db.ready().await.expect("ready");
    for idx in 0..3 {
        let mut event = make_net_event(&format!("retry-{idx}.example"), Decision::Allowed);
        event.event_id = Some(format!("{idx:012x}"));
        event.request_body = Some(format!("request body {idx}").into_bytes());
        db.write(WriteOp::NetEvent(event)).await.expect("write accepted");
    }

    crate::writer::fail_disk_flushes_for_path_for_tests(&p, 1);
    db.flush().await.expect_err("the injected flush failure is reported");
    assert_eq!(
        mem_rows(&p, "net_events"),
        3,
        "a failed flush must not drop the rows it did not copy"
    );
    assert_eq!(disk_net_event_count(&p, "retry-0.example"), 0);

    db.flush().await.expect("the retry flushes");
    assert_eq!(mem_rows(&p, "net_events"), 0, "the retry's prune must empty memory");
    let counts = query_json(
        &db.query(
            "SELECT
                (SELECT COUNT(*) FROM net_events WHERE domain LIKE 'retry-%'),
                (SELECT COUNT(DISTINCT event_id) FROM net_events WHERE domain LIKE 'retry-%'),
                (SELECT COUNT(*) FROM event_body_blobs
                 WHERE source_table = 'net_events' AND direction = 'request')",
            &[],
        )
        .await
        .expect("count flushed rows and bodies"),
    );
    assert_eq!(
        counts["rows"],
        json!([[3, 3, 3]]),
        "each event must reach disk exactly once, and each body be indexed exactly once"
    );
    crate::writer::fail_disk_flushes_for_tests(0);
}
