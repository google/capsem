//! Connection pragmas and the mmap telemetry that reports on them.
//!
//! One place for what a writer, a reader and a shared-cache reader ask of
//! SQLite, so a route never learns whether a query reads through the page
//! cache, mmap or the DB-owned memory tables.
use std::path::{Path, PathBuf};
use std::time::Duration;

use capsem_telemetry::db::{
    DB_SQLITE_FILE_SIZE_BYTES, DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL, DB_SQLITE_MMAP_CONFIG_BYTES,
    DB_SQLITE_MMAP_COVERAGE_RATIO, DB_SQLITE_MMAP_EFFECTIVE_BYTES, DB_SQLITE_WAL_SIZE_BYTES,
};
use rusqlite::Connection;

/// How long a disk-only reader waits out an exclusive file lock, matching the
/// writer's own `busy_timeout`.
const READER_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// SQLite mmap window for file-backed ledger databases.
///
/// Keep this in the DB layer: routes and security components should not know
/// whether a query reads through SQLite's page cache, mmap, or DB-owned memory
/// tables.
pub const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;

fn apply_mmap_pragma(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "mmap_size", SQLITE_MMAP_SIZE_BYTES)
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", path.display(), suffix))
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub fn record_sqlite_mmap_telemetry(conn: &Connection, path: &Path, role: &'static str, phase: &'static str) {
    let effective_mmap: i64 = conn.query_row("PRAGMA mmap_size", [], |row| row.get(0)).unwrap_or(0);
    let db_file_size = file_len(path);
    let wal_file_size = file_len(&sqlite_sidecar_path(path, "-wal"));
    let status = if db_file_size == 0 {
        "empty"
    } else if db_file_size <= effective_mmap.max(0) as u64 {
        "within_window"
    } else {
        "over_window"
    };
    let coverage_ratio = if db_file_size == 0 {
        1.0
    } else {
        (effective_mmap.max(0) as u64).min(db_file_size) as f64 / db_file_size as f64
    };

    ::metrics::gauge!(DB_SQLITE_MMAP_CONFIG_BYTES, "role" => role, "phase" => phase).set(SQLITE_MMAP_SIZE_BYTES as f64);
    ::metrics::gauge!(DB_SQLITE_MMAP_EFFECTIVE_BYTES, "role" => role, "phase" => phase).set(effective_mmap as f64);
    ::metrics::gauge!(DB_SQLITE_FILE_SIZE_BYTES, "role" => role, "phase" => phase).set(db_file_size as f64);
    ::metrics::gauge!(DB_SQLITE_WAL_SIZE_BYTES, "role" => role, "phase" => phase).set(wal_file_size as f64);
    ::metrics::gauge!(DB_SQLITE_MMAP_COVERAGE_RATIO, "role" => role, "phase" => phase).set(coverage_ratio);
    ::metrics::counter!(
        DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL,
        "role" => role,
        "phase" => phase,
        "status" => status
    )
    .increment(1);

    tracing::debug!(
        target: "capsem.db",
        db_path = %path.display(),
        role,
        phase,
        mmap_config_bytes = SQLITE_MMAP_SIZE_BYTES,
        mmap_effective_bytes = effective_mmap,
        db_file_size_bytes = db_file_size,
        wal_file_size_bytes = wal_file_size,
        mmap_coverage_ratio = coverage_ratio,
        mmap_budget_status = status,
        "sqlite mmap telemetry recorded"
    );
}

/// Apply session write-mode pragmas: WAL with durable FULL commits.
/// Only call on read-write connections (the writer).
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    #[cfg(target_os = "macos")]
    conn.pragma_update(None, "fullfsync", "ON")?;
    apply_mmap_pragma(conn)?;
    Ok(())
}

/// Apply read-safe pragmas for DB-owned query connections.
///
/// Readers open the file read-write (see `DbReader::open`) and `query_only`
/// is what keeps the read worker from writing through it.
pub fn apply_reader_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    apply_mmap_pragma(conn)?;
    conn.pragma_update(None, "query_only", "ON")?;
    // WAL lets a reader read the file while the writer commits, but
    // `wal_checkpoint(TRUNCATE)` and `VACUUM` take an exclusive file lock,
    // and a read that lands during one gets SQLITE_BUSY. Wait it out the same
    // way the writer does rather than failing a route because a ledger was
    // being compacted.
    conn.busy_timeout(READER_BUSY_TIMEOUT)?;
    Ok(())
}
