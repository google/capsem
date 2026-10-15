use super::*;
use capsem_telemetry::db::{
    DB_SQLITE_FILE_SIZE_BYTES, DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL, DB_SQLITE_MMAP_CONFIG_BYTES,
    DB_SQLITE_MMAP_COVERAGE_RATIO, DB_SQLITE_MMAP_EFFECTIVE_BYTES, DB_SQLITE_WAL_SIZE_BYTES,
};

/// Writer pragmas (WAL + synchronous) must only be applied to read-write
/// connections. Read-only connections must use apply_reader_pragmas instead.
#[test]
fn reader_pragmas_work_on_readonly_connection() {
    // Create a file-backed DB first (writer sets WAL).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    // Open read-only -- apply_reader_pragmas must not fail.
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();
}

#[test]
fn reader_pragmas_enable_mmap_before_query_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    {
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).unwrap();
    apply_reader_pragmas(&conn).unwrap();

    let mmap_size: i64 = conn.query_row("PRAGMA mmap_size", [], |row| row.get(0)).unwrap();
    assert!(
        mmap_size >= SQLITE_MMAP_SIZE_BYTES,
        "reader worker connections must enable SQLite mmap before query_only; got {mmap_size}"
    );

    let query_only: i64 = conn.query_row("PRAGMA query_only", [], |row| row.get(0)).unwrap();
    assert_eq!(query_only, 1, "reader worker must still be query-only");
}

#[test]
fn mmap_telemetry_records_budget_and_size_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let conn = Connection::open(&path).unwrap();
    apply_pragmas(&conn).unwrap();
    create_tables(&conn).unwrap();
    record_sqlite_mmap_telemetry(&conn, &path, "writer", "test");

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_CONFIG_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_EFFECTIVE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_FILE_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_WAL_SIZE_BYTES && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_COVERAGE_RATIO && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SQLITE_MMAP_BUDGET_CHECKS_TOTAL && matches!(value, DebugValue::Counter(1))
    }));
}
