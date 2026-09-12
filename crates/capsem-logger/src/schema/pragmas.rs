//! Connection pragmas and the mmap telemetry that reports on them.
//!
//! One place for what a writer, a reader and a shared-cache reader ask of
//! SQLite, so a route never learns whether a query reads through the page
//! cache, mmap or the DB-owned memory tables.
use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// SQLite mmap window for file-backed ledger databases.
///
/// Keep this in the DB layer: routes and security components should not know
/// whether a query reads through SQLite's page cache, mmap, or DB-owned memory
/// tables.
pub const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
pub const DB_SQLITE_MMAP_CONFIG_BYTES: &str = "db.sqlite_mmap_config_bytes";
pub const DB_SQLITE_MMAP_EFFECTIVE_BYTES: &str = "db.sqlite_mmap_effective_bytes";
pub const DB_SQLITE_FILE_SIZE_BYTES: &str = "db.sqlite_file_size_bytes";
pub const DB_SQLITE_WAL_SIZE_BYTES: &str = "db.sqlite_wal_size_bytes";
pub const DB_SQLITE_MMAP_COVERAGE_RATIO: &str = "db.sqlite_mmap_coverage_ratio";
pub const DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL: &str = "db.sqlite_mmap_budget_checks_total";

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

/// Apply write-mode pragmas: WAL journal + relaxed synchronous.
/// Only call on read-write connections (the writer).
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    apply_mmap_pragma(conn)?;
    Ok(())
}

/// Apply read-safe pragmas for DB-owned query connections.
///
/// These connections may be opened read-write briefly so the DB layer can
/// attach and populate its private `mem` schema. After setup, `query_only`
/// prevents writes through the read worker.
pub fn apply_reader_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    apply_mmap_pragma(conn)?;
    conn.pragma_update(None, "query_only", "ON")?;
    // The hot ledger tables live in a shared-cache memory schema, where a
    // reader takes a table-level read lock and fails at once with
    // SQLITE_LOCKED while the writer's batch holds the table -- and starves
    // outright while batches are back to back. `read_uncommitted` is
    // SQLite's answer for shared-cache readers: no table locks, so a read
    // never waits on or fails against the single writer. What it may see is
    // the tail of a batch before its commit, which for an append-only ledger
    // written by one thread is the same rows a moment early.
    conn.pragma_update(None, "read_uncommitted", "ON")?;
    Ok(())
}
