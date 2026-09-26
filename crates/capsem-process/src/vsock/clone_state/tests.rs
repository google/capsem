use super::*;

use capsem_logger::{DbWriter, FileAction, FileEvent, FileKind, WriteOp};

/// A fork of a running session carries the rows its writer accepted a moment
/// ago (#243). The writer holds accepted rows in memory until its next disk
/// flush, up to five seconds away, and the clone copies the file.
#[tokio::test]
async fn a_fork_carries_rows_the_writer_has_not_flushed_yet() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("src");
    let destination = tmp.path().join("dst");
    std::fs::create_dir_all(source.join("system")).unwrap();
    std::fs::create_dir_all(source.join("guest/workspace")).unwrap();
    std::fs::create_dir(&destination).unwrap();

    let db = DbWriter::open(&source.join("session.db"), 64).unwrap();
    db.write(WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: FileAction::Created,
        path: "/root/written-just-before-the-fork".into(),
        size: Some(7),
        kind: FileKind::File,
        trace_id: None,
        credential_ref: None,
    }))
    .await;

    flush_then_clone(&db, source.clone(), destination.clone())
        .await
        .unwrap();

    let cloned = capsem_logger::DbReader::open(&destination.join("session.db")).unwrap();
    let rows = cloned.query_raw("SELECT path FROM fs_events").unwrap();
    assert_eq!(
        rows,
        r#"{"columns":["path"],"rows":[["/root/written-just-before-the-fork"]]}"#
    );
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();
}
