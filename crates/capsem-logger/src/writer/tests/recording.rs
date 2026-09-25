//! The writer's metric recorders (`writer/recording.rs`) observed through a
//! debugging recorder.

use super::*;
use capsem_telemetry::db::{
    DB_ENQUEUE_TOTAL, DB_ENQUEUE_WAIT_MS, DB_SHUTDOWN_FLUSH_MS, DB_WRITE_BATCH_CAPACITY, DB_WRITE_BATCH_DURATION_MS,
    DB_WRITE_BATCH_ROWS_PER_SEC, DB_WRITE_BATCH_SIZE, DB_WRITE_BATCH_TOTAL, DB_WRITE_OPS_TOTAL,
};

#[test]
fn db_writer_records_enqueue_batch_and_shutdown_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let (tx, rx) = writer_channel(16);
    tx.send(WriterMessage::write(WriteOp::FileEvent(file_event(
        "/metrics",
        crate::events::FileAction::Created,
        None,
    ))))
    .unwrap();
    drop(tx);

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::schema::apply_pragmas(&conn).unwrap();
    crate::schema::create_tables(&conn).unwrap();
    crate::schema::create_memory_tables(&conn, &crate::schema::memory_uri_for_name("writer-metrics-test")).unwrap();

    let pending_body_bytes = AtomicU64::new(0);
    metrics::with_local_recorder(&recorder, || {
        writer_loop(
            conn,
            rx,
            None,
            16,
            &pending_body_bytes,
            BodyArchive::disabled(SystemTime::now),
            LedgerTally::default(),
        )
    });

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot
        .iter()
        .any(|(key, _, _, value)| key.key().name() == DB_WRITE_BATCH_TOTAL && matches!(value, DebugValue::Counter(1))));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_DURATION_MS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_SIZE && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_CAPACITY && matches!(value, DebugValue::Gauge(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_BATCH_ROWS_PER_SEC && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_WRITE_OPS_TOTAL && matches!(value, DebugValue::Counter(1))
    }));
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_SHUTDOWN_FLUSH_MS && matches!(value, DebugValue::Histogram(_))
    }));
}

#[test]
fn db_writer_records_enqueue_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("enqueue.db"), 1).unwrap();
    let accepted = writer.try_write(WriteOp::FileEvent(file_event(
        "/enqueue",
        crate::events::FileAction::Created,
        None,
    )));
    assert!(accepted);
    writer.shutdown_blocking();

    let snapshot = snapshotter.snapshot().into_vec();
    assert!(snapshot.iter().any(|(key, _, _, value)| {
        key.key().name() == DB_ENQUEUE_WAIT_MS && matches!(value, DebugValue::Histogram(_))
    }));
    assert!(snapshot
        .iter()
        .any(|(key, _, _, value)| { key.key().name() == DB_ENQUEUE_TOTAL && matches!(value, DebugValue::Counter(_)) }));
}
