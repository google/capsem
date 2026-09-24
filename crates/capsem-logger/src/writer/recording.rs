//! Metric recorders for the writer's two hot boundaries: enqueueing an op and
//! executing a batch of them.
//!
//! Named `recording` rather than `metrics` so it cannot shadow the `metrics`
//! crate inside this module tree. Split out of the loop because they are the only code in it that measures
//! rather than does, and because a recorder that lives beside the thing it
//! measures gets edited in step with it -- which is how a bucket boundary
//! and the histogram it feeds come to disagree.

use std::time::Instant;

use capsem_telemetry::db::{
    DB_ENQUEUE_TOTAL, DB_ENQUEUE_WAIT_MS, DB_WRITE_BATCH_CAPACITY, DB_WRITE_BATCH_DURATION_MS,
    DB_WRITE_BATCH_ROWS_PER_SEC, DB_WRITE_BATCH_SIZE, DB_WRITE_BATCH_TOTAL,
};

pub(super) fn record_enqueue(started: Instant, queue_result: &'static str, span: &tracing::Span) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(DB_ENQUEUE_TOTAL, "queue_result" => queue_result).increment(1);
    ::metrics::histogram!(DB_ENQUEUE_WAIT_MS, "queue_result" => queue_result).record(elapsed_ms);
    span.record("status", if queue_result == "queued" { "ok" } else { "error" });
    span.record("queue_result", queue_result);
}

pub(super) fn record_batch(
    started: Instant,
    batch_size: usize,
    batch_capacity: usize,
    batch_size_bucket: &'static str,
    status: &'static str,
    span: &tracing::Span,
) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let rows_per_sec = if elapsed_ms > 0.0 {
        batch_size as f64 / (elapsed_ms / 1000.0)
    } else {
        0.0
    };
    ::metrics::counter!(DB_WRITE_BATCH_TOTAL,
        "batch_size_bucket" => batch_size_bucket,
        "status" => status)
    .increment(1);
    ::metrics::histogram!(DB_WRITE_BATCH_DURATION_MS,
        "batch_size_bucket" => batch_size_bucket,
        "status" => status)
    .record(elapsed_ms);
    ::metrics::histogram!(DB_WRITE_BATCH_SIZE,
        "batch_size_bucket" => batch_size_bucket)
    .record(batch_size as f64);
    ::metrics::gauge!(DB_WRITE_BATCH_CAPACITY).set(batch_capacity as f64);
    ::metrics::histogram!(DB_WRITE_BATCH_ROWS_PER_SEC,
        "batch_size_bucket" => batch_size_bucket,
        "status" => status)
    .record(rows_per_sec);
    span.record("status", status);
}

pub(super) fn batch_size_bucket(size: usize) -> &'static str {
    match size {
        0 => "0",
        1 => "1",
        2..=8 => "2_8",
        9..=32 => "9_32",
        33..=128 => "33_128",
        _ => "gt_128",
    }
}
