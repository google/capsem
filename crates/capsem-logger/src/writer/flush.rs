//! Bounded publication of in-memory ledger rows and archive indexes.

use super::*;

pub(in crate::writer) fn flush_dirty_tables_to_disk(
    conn: &Connection,
    dirty_tables: &mut BTreeSet<&'static str>,
    flush_watermarks: &mut schema::MemoryFlushWatermarks,
    db_path: Option<&Path>,
    bodies: &mut BodyArchive,
    counters: &LedgerCounters,
    wait_for_body_sync: bool,
) -> rusqlite::Result<FlushTimings> {
    if dirty_tables.is_empty() && !bodies.has_work() {
        return Ok(FlushTimings::default());
    }
    // Bytes before index: the segment reaches the archive file here, and the
    // rows that name it are inserted in the transaction below. A crash in
    // between costs unreferenced bytes, never a row pointing past EOF.
    let started = Instant::now();
    bodies.flush_segment();
    bodies.prepare_index_rows(wait_for_body_sync);
    let archive_ms = started.elapsed().as_secs_f64() * 1000.0;
    let tables: Vec<&'static str> = dirty_tables.iter().copied().collect();
    let started = Instant::now();
    let tx = conn.unchecked_transaction()?;
    let transaction_begin_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let body_index_limit = if wait_for_body_sync {
        usize::MAX
    } else {
        capsem_proto::LEDGER_ARCHIVE_INDEX_ROWS_PER_FLUSH
    };
    bodies.commit_prepared_index_rows_with_limit(&tx, body_index_limit)?;
    let body_index_rows = bodies.prepared_index_row_count();
    let body_index_ms = started.elapsed().as_secs_f64() * 1000.0;
    // Injected where a real flush failure lands: inside the transaction, with
    // the archive's index rows already written into it. Everything here rolls
    // back together, and the retry has to bring the bodies back with the rows.
    if take_disk_flush_failure_for_tests(db_path) {
        return Err(rusqlite::Error::InvalidParameterName(
            "injected disk flush failure before copy".to_string(),
        ));
    }
    let started = Instant::now();
    let advanced_watermarks = schema::with_memory_schema_lock(|| {
        schema::flush_memory_tables_to_disk(&tx, tables.iter().copied(), flush_watermarks)
    })?;
    let table_copy_ms = started.elapsed().as_secs_f64() * 1000.0;
    // The snapshot lands with the rows it counts, or neither does.
    let started = Instant::now();
    counters::store(&tx, counters)?;
    let counter_snapshot_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    tx.commit()?;
    let commit_ms = started.elapsed().as_secs_f64() * 1000.0;
    // Only now are the written segments indexed: anything above this line rolls
    // the transaction back, and the next flush retries both halves together.
    bodies.index_rows_committed();
    flush_watermarks.extend(advanced_watermarks);
    dirty_tables.clear();
    Ok(FlushTimings {
        archive_ms,
        transaction_begin_ms,
        body_index_rows,
        body_index_ms,
        table_copy_ms,
        counter_snapshot_ms,
        commit_ms,
    })
}

#[derive(Default)]
pub(in crate::writer) struct FlushTimings {
    archive_ms: f64,
    transaction_begin_ms: f64,
    body_index_rows: usize,
    body_index_ms: f64,
    table_copy_ms: f64,
    counter_snapshot_ms: f64,
    commit_ms: f64,
}

#[derive(Clone, Copy)]
pub(super) struct FlushRequest {
    pub(super) pending_ops: usize,
    pub(super) wait_for_body_sync: bool,
}

pub(super) fn flush_dirty_tables_to_disk_recorded(
    conn: &Connection,
    dirty_tables: &mut BTreeSet<&'static str>,
    flush_watermarks: &mut schema::MemoryFlushWatermarks,
    db_path: Option<&Path>,
    bodies: &mut BodyArchive,
    counters: &LedgerCounters,
    request: FlushRequest,
) -> rusqlite::Result<()> {
    let table_count = dirty_tables.len();
    let started = Instant::now();
    let result = flush_dirty_tables_to_disk(
        conn,
        dirty_tables,
        flush_watermarks,
        db_path,
        bodies,
        counters,
        request.wait_for_body_sync,
    );
    let elapsed = started.elapsed();
    let status = if result.is_ok() { "ok" } else { "error" };
    let timings = result.as_ref().ok();
    let archive_ms = timings.map_or(0.0, |timings| timings.archive_ms);
    let transaction_begin_ms = timings.map_or(0.0, |timings| timings.transaction_begin_ms);
    let body_index_rows = timings.map_or(0, |timings| timings.body_index_rows);
    let body_index_ms = timings.map_or(0.0, |timings| timings.body_index_ms);
    let table_copy_ms = timings.map_or(0.0, |timings| timings.table_copy_ms);
    let counter_snapshot_ms = timings.map_or(0.0, |timings| timings.counter_snapshot_ms);
    let commit_ms = timings.map_or(0.0, |timings| timings.commit_ms);
    tracing::debug!(
        target: "capsem.db",
        duration_ms = elapsed.as_secs_f64() * 1000.0,
        archive_ms,
        transaction_begin_ms,
        body_index_rows,
        body_index_ms,
        table_copy_ms,
        counter_snapshot_ms,
        commit_ms,
        pending_ops = request.pending_ops,
        table_count,
        status,
        "db disk flush completed"
    );
    if elapsed >= DISK_FLUSH_INTERVAL {
        warn!(
            duration_ms = elapsed.as_secs_f64() * 1000.0,
            archive_ms,
            transaction_begin_ms,
            body_index_rows,
            body_index_ms,
            table_copy_ms,
            counter_snapshot_ms,
            commit_ms,
            pending_ops = request.pending_ops,
            table_count,
            status,
            "db disk flush exceeded its configured interval"
        );
    }
    result.map(|_| ())
}
