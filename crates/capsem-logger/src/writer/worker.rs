//! Dedicated SQLite writer-thread event loop.

use super::*;

pub(super) fn writer_loop(
    conn: Connection,
    rx: WriterReceiver,
    db_path: Option<PathBuf>,
    batch_capacity: usize,
    pending_body_bytes: &AtomicU64,
    mut bodies: BodyArchive,
    mut tally: LedgerTally,
) {
    let mut flush_watermarks =
        schema::with_memory_schema_lock(|| schema::initial_memory_flush_watermarks(&conn, schema::hot_ledger_tables()))
            .unwrap_or_else(|error| {
                warn!(error = %error, "db initial memory flush watermark load failed");
                schema::MemoryFlushWatermarks::new()
            });
    // Rows at or below the open-time watermark were written by an earlier
    // writer; exec ids restart with the process, so a completion must not
    // reach one of those.
    let exec_floor = flush_watermarks.get("exec_events").copied().unwrap_or(0);
    let mut dirty_tables = BTreeSet::new();
    let mut dirty_ops = 0_usize;
    let mut last_disk_flush = Instant::now();
    // 1. Block until at least one op arrives. Returns None when all
    //    Senders are dropped (clean shutdown) and ends the loop.
    loop {
        let first_message = if dirty_ops == 0 && !bodies.has_work() {
            rx.recv().ok()
        } else {
            match rx.recv_timeout(DISK_FLUSH_INTERVAL) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(error) = flush_dirty_tables_to_disk_recorded(
                        &conn,
                        &mut dirty_tables,
                        &mut flush_watermarks,
                        db_path.as_deref(),
                        &mut bodies,
                        tally.counters(),
                        FlushRequest {
                            pending_ops: dirty_ops,
                            wait_for_body_sync: false,
                        },
                    ) {
                        warn!(error = %error, "db interval flush failed");
                    } else {
                        dirty_ops = 0;
                        last_disk_flush = Instant::now();
                    }
                    ::metrics::gauge!(DB_MEMORY_UNFLUSHED_OPS).set(dirty_ops as f64);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            }
        };
        let Some(first_message) = first_message else {
            break;
        };

        let mut batch = Vec::with_capacity(batch_capacity);
        let mut barriers = Barriers::default();
        let mut at_barrier = barriers.accept(first_message.into_message(), &mut batch);

        // 2. Drain any ops already queued (non-blocking).
        while !at_barrier && batch.len() < batch_capacity {
            match rx.try_recv() {
                Ok(message) => at_barrier = barriers.accept(message.into_message(), &mut batch),
                Err(_) => break,
            }
        }

        // 3. Execute entire batch in a single transaction.
        let batch_size = batch.len();
        let batch_bucket = batch_size_bucket(batch_size);
        let span = tracing::debug_span!(
            target: "capsem.db",
            DB_WRITE_BATCH_SPAN,
            batch_size_bucket = batch_bucket,
            status = tracing::field::Empty,
        );
        let started = Instant::now();
        let batch_capacity = batch.capacity();
        if batch.is_empty() {
            record_batch(started, batch_size, batch_capacity, batch_bucket, "ok", &span);
        } else {
            match span.in_scope(|| execute_memory_batch(&conn, &batch, &mut bodies, exec_floor, &mut tally)) {
                Ok(outcome) => {
                    dirty_tables.extend(outcome.tables);
                    dirty_ops += outcome.written;
                    record_batch(started, batch_size, batch_capacity, batch_bucket, "ok", &span);
                }
                Err(e) => {
                    record_batch(started, batch_size, batch_capacity, batch_bucket, "error", &span);
                    warn!(
                        error = %e,
                        count = batch.len(),
                        "db memory write batch failed; retrying its ops individually"
                    );
                    let salvaged = span
                        .in_scope(|| retry_batch_ops_individually(&conn, &batch, &mut bodies, exec_floor, &mut tally));
                    dirty_tables.extend(salvaged.tables);
                    dirty_ops += salvaged.written;
                }
            }
        }
        // A burst of large bodies closes its block as soon as the batch ends
        // rather than growing it until the interval expires. Its index rows
        // wait for the next transaction; its bytes are already on disk.
        bodies.close_if_due();
        pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);
        let disk_flush_due = dirty_ops >= DISK_FLUSH_THRESHOLD_OPS
            || last_disk_flush.elapsed() >= DISK_FLUSH_INTERVAL
            || barriers.waiting();
        let mut barrier_outcome: FlushOutcome = Ok(());
        if disk_flush_due {
            match flush_dirty_tables_to_disk_recorded(
                &conn,
                &mut dirty_tables,
                &mut flush_watermarks,
                db_path.as_deref(),
                &mut bodies,
                tally.counters(),
                FlushRequest {
                    pending_ops: dirty_ops,
                    wait_for_body_sync: barriers.waiting(),
                },
            ) {
                Ok(()) => {
                    dirty_ops = 0;
                    last_disk_flush = Instant::now();
                    pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);
                }
                Err(error) => {
                    warn!(error = %error, "db dirty table flush failed");
                    barrier_outcome = Err(format!("db dirty table flush failed: {error}"));
                }
            }
        }
        ::metrics::gauge!(DB_MEMORY_UNFLUSHED_OPS).set(dirty_ops as f64);
        barriers.answer(&conn, &mut bodies, &barrier_outcome);
    }

    // Test hook: lets `test_wal_absent_after_clean_shutdown`-style tests
    // simulate a slow checkpoint so the explicit-cleanup path can be
    // distinguished from implicit tokio-runtime-drop ordering. Gated on
    // an env var so it's a no-op in production.
    if let Ok(ms) = std::env::var("CAPSEM_TEST_SLOW_CHECKPOINT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    // The open block ends with its FINAL segment, committed by this flush.
    bodies.close_block();
    if let Err(error) = flush_dirty_tables_to_disk_recorded(
        &conn,
        &mut dirty_tables,
        &mut flush_watermarks,
        db_path.as_deref(),
        &mut bodies,
        tally.counters(),
        FlushRequest {
            pending_ops: dirty_ops,
            wait_for_body_sync: true,
        },
    ) {
        warn!(error = %error, "db shutdown dirty table flush failed");
        // There is no next flush to retry into, so the bodies still waiting
        // for an index row are lost here. Counted, so the drop counter closes
        // over the session rather than ending on a number that is short by
        // however much the last flush was carrying.
        bodies.abandon_uncommitted("shutdown");
    }
    bodies.sync();
    pending_body_bytes.store(bodies.pending_bytes() as u64, Ordering::Release);

    // All senders dropped -- checkpoint WAL before closing connection.
    let span = tracing::debug_span!(
        target: "capsem.db",
        DB_SHUTDOWN_FLUSH_SPAN,
        status = tracing::field::Empty,
    );
    let started = Instant::now();
    let result = span.in_scope(|| conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"));
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = if result.is_ok() { "ok" } else { "error" };
    ::metrics::histogram!(DB_SHUTDOWN_FLUSH_MS, "status" => status).record(elapsed_ms);
    span.record("status", status);
    // mmap coverage and file-size diagnostics query SQLite and the filesystem.
    // Sampling them on every periodic flush once held this admission thread
    // for more than a second under DNS load. Shutdown is outside the producer
    // path and still records the final ledger size and mmap budget.
    if let Some(path) = db_path.as_deref() {
        schema::record_sqlite_mmap_telemetry(&conn, path, "writer", "shutdown");
    }
}
