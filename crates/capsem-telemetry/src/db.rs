//! Session ledger metrics: the reader query path, the async writer, the body
//! archive and SQLite's mmap window.

use metrics::Unit;

use crate::{MetricKind, MetricSpec};

pub const DB_QUERY_TOTAL: &str = "db.query_total";
pub const DB_QUERY_DURATION_MS: &str = "db.query_duration_ms";
pub const DB_QUERY_RESULT_ROWS: &str = "db.query_result_rows";
pub const DB_QUERY_RESULT_BYTES: &str = "db.query_result_bytes";
pub const DB_QUERY_PARAMS_COUNT: &str = "db.query_params_count";

pub const DB_ENQUEUE_WAIT_MS: &str = "db.enqueue_wait_ms";
pub const DB_ENQUEUE_TOTAL: &str = "db.enqueue_total";
pub const DB_WRITE_BATCH_TOTAL: &str = "db.write_batch_total";
pub const DB_WRITE_BATCH_DURATION_MS: &str = "db.write_batch_duration_ms";
pub const DB_WRITE_OP_REJECTED_TOTAL: &str = "db.write_op_rejected_total";
pub const DB_WRITE_BATCH_SIZE: &str = "db.write_batch_size";
pub const DB_WRITE_BATCH_CAPACITY: &str = "db.write_batch_capacity";
pub const DB_WRITE_BATCH_ROWS_PER_SEC: &str = "db.write_batch_rows_per_sec";
pub const DB_WRITE_OPS_TOTAL: &str = "db.write_ops_total";
pub const DB_SHUTDOWN_FLUSH_MS: &str = "db.shutdown_flush_ms";
/// Bodies the archive gave up on, by the step that gave up: the only place a
/// poisoned archive surfaces besides a log line.
pub const DB_ARCHIVE_BODIES_DROPPED_TOTAL: &str = "db.archive_bodies_dropped_total";
/// Bodies indexed against identical bytes already stored, labelled by
/// scope: `block` (the open block) or `archive` (a committed earlier segment).
pub const DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL: &str = "db.archive_bodies_deduplicated_total";
/// Ops the writer holds in memory waiting for a disk flush. It falls to zero
/// on every flush that lands; a value that only climbs is a disk the writer
/// cannot flush to, with the session's rows piling up in RAM.
pub const DB_MEMORY_UNFLUSHED_OPS: &str = "db.memory_unflushed_ops";

pub const DB_SQLITE_MMAP_CONFIG_BYTES: &str = "db.sqlite_mmap_config_bytes";
pub const DB_SQLITE_MMAP_EFFECTIVE_BYTES: &str = "db.sqlite_mmap_effective_bytes";
pub const DB_SQLITE_FILE_SIZE_BYTES: &str = "db.sqlite_file_size_bytes";
pub const DB_SQLITE_WAL_SIZE_BYTES: &str = "db.sqlite_wal_size_bytes";
pub const DB_SQLITE_MMAP_COVERAGE_RATIO: &str = "db.sqlite_mmap_coverage_ratio";
pub const DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL: &str = "db.sqlite_mmap_budget_checks_total";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        DB_QUERY_TOTAL,
        Unit::Count,
        "Ledger read queries executed, by phase and status.",
    ),
    MetricSpec::histogram(
        DB_QUERY_DURATION_MS,
        Unit::Milliseconds,
        "Wall time of one ledger read query, by phase and status.",
    ),
    MetricSpec::histogram(
        DB_QUERY_RESULT_ROWS,
        Unit::Count,
        "Rows returned by one successful ledger read query, by phase.",
    ),
    MetricSpec::histogram(
        DB_QUERY_RESULT_BYTES,
        Unit::Bytes,
        "Serialized bytes returned by one successful ledger read query, by phase.",
    ),
    MetricSpec::histogram(
        DB_QUERY_PARAMS_COUNT,
        Unit::Count,
        "Bound parameters passed to one ledger read query, by phase and status.",
    ),
    MetricSpec::histogram(
        DB_ENQUEUE_WAIT_MS,
        Unit::Milliseconds,
        "Time a producer waited to hand one write op to the ledger writer, by queue result.",
    ),
    MetricSpec::counter(
        DB_ENQUEUE_TOTAL,
        Unit::Count,
        "Write ops offered to the ledger writer, by queue result.",
    ),
    MetricSpec::counter(
        DB_WRITE_BATCH_TOTAL,
        Unit::Count,
        "Write batches applied by the ledger writer, by batch size bucket and status.",
    ),
    MetricSpec::histogram(
        DB_WRITE_BATCH_DURATION_MS,
        Unit::Milliseconds,
        "Wall time to apply one write batch, by batch size bucket and status.",
    ),
    MetricSpec::counter(
        DB_WRITE_OP_REJECTED_TOTAL,
        Unit::Count,
        "Write ops SQLite rejected and the writer dropped alone during batch salvage, by op kind.",
    ),
    MetricSpec::histogram(
        DB_WRITE_BATCH_SIZE,
        Unit::Count,
        "Write ops in one batch, by batch size bucket.",
    ),
    MetricSpec::gauge(
        DB_WRITE_BATCH_CAPACITY,
        Unit::Count,
        "Maximum write ops the writer drains into one batch.",
    ),
    MetricSpec::histogram(
        DB_WRITE_BATCH_ROWS_PER_SEC,
        Unit::CountPerSecond,
        "Write ops applied per second within one batch, by batch size bucket and status.",
    ),
    MetricSpec::counter(
        DB_WRITE_OPS_TOTAL,
        Unit::Count,
        "Write ops stored by the ledger writer, by insert type.",
    ),
    MetricSpec::histogram(
        DB_SHUTDOWN_FLUSH_MS,
        Unit::Milliseconds,
        "Wall time of the final WAL checkpoint when the writer shuts down, by status.",
    ),
    MetricSpec::counter(
        DB_ARCHIVE_BODIES_DROPPED_TOTAL,
        Unit::Count,
        "Bodies the body archive gave up on, by the step that gave up.",
    ),
    MetricSpec::counter(
        DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL,
        Unit::Count,
        "Bodies indexed against identical bytes already stored, by scope (block|archive).",
    ),
    MetricSpec::gauge(
        DB_MEMORY_UNFLUSHED_OPS,
        Unit::Count,
        "Write ops held in memory waiting for a disk flush; only climbing means the disk cannot be flushed to.",
    ),
    MetricSpec::gauge(
        DB_SQLITE_MMAP_CONFIG_BYTES,
        Unit::Bytes,
        "SQLite mmap window the ledger asks for, by connection role and phase.",
    ),
    MetricSpec::gauge(
        DB_SQLITE_MMAP_EFFECTIVE_BYTES,
        Unit::Bytes,
        "SQLite mmap window the connection actually got, by connection role and phase.",
    ),
    MetricSpec::gauge(
        DB_SQLITE_FILE_SIZE_BYTES,
        Unit::Bytes,
        "Ledger database file size, by connection role and phase.",
    ),
    MetricSpec::gauge(
        DB_SQLITE_WAL_SIZE_BYTES,
        Unit::Bytes,
        "Ledger WAL file size, by connection role and phase.",
    ),
    MetricSpec::new(
        DB_SQLITE_MMAP_COVERAGE_RATIO,
        MetricKind::Gauge,
        None,
        "Fraction (0..1) of the ledger database file covered by the effective mmap window, by connection role and phase.",
    ),
    MetricSpec::counter(
        DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL,
        Unit::Count,
        "mmap budget checks, by connection role, phase and status (empty|within_window|over_window).",
    ),
];
