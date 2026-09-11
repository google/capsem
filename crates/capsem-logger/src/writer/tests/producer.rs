use super::*;

#[test]
fn write_blocking_persists_without_try_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("blocking.db");
    let writer = DbWriter::open(&db_path, 1).unwrap();
    writer
        .write_blocking_checked(WriteOp::FileEvent(crate::events::FileEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/blocking".into(),
            size: None,
            trace_id: None,
            credential_ref: None,
        }))
        .unwrap();
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fs_events WHERE path = '/blocking'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn write_blocking_is_safe_inside_tokio_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("runtime-blocking.db");
    let writer = DbWriter::open(&db_path, 1).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        writer.write_blocking(WriteOp::FileEvent(crate::events::FileEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            action: crate::events::FileAction::Created,
            path: "/runtime-safe".into(),
            size: None,
            trace_id: None,
            credential_ref: None,
        }));
    });
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fs_events WHERE path = '/runtime-safe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
