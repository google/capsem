use super::bodies::net_event_with_response;
use super::*;
use capsem_foundation::unix::lock::{try_lock_descriptor, DescriptorLockAttempt, LockMode};
use std::time::Duration;

#[tokio::test]
async fn raw_snapshot_copy_leases_old_generation_until_copy_finishes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("snapshot");
    std::fs::create_dir(&source).unwrap();
    let source_db = source.join("session.db");
    let db = DbHandle::open(&source_db).unwrap();
    let body = "a captured snapshot must remain visibly pinned".repeat(32);
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789af",
        "snapshot.example",
        &body,
    )))
    .await
    .unwrap();
    db.flush().await.unwrap();
    let conn = rusqlite::Connection::open(&source_db).unwrap();
    let state = crate::schema::archive_state(&conn).unwrap();
    drop(conn);
    let generation = crate::writer::archive_path_for_db(&source_db).join(state.header.generation_id.file_name());
    let observer = std::fs::File::open(&generation).unwrap();
    let (reached, resume) = crate::db::maintenance::pause_next_snapshot_after_capture_for_tests(&source_db);
    let copy_source = source.clone();
    let copy_destination = destination.clone();
    let snapshot = tokio::task::spawn_blocking(move || crate::snapshot_session_ledger(&copy_source, &copy_destination));
    tokio::task::spawn_blocking(move || reached.recv_timeout(Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    let before = try_lock_descriptor(observer.try_clone().unwrap(), LockMode::Exclusive).unwrap();
    let leased_before = matches!(before, DescriptorLockAttempt::Contended(_));
    drop(before);
    db.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    assert!(
        !generation.exists(),
        "ordinary retention may unlink the captured old generation"
    );
    let after = try_lock_descriptor(observer.try_clone().unwrap(), LockMode::Exclusive).unwrap();
    let leased_after = matches!(after, DescriptorLockAttempt::Contended(_));
    drop(after);
    // Finish the worker even on RED, so an assertion cannot strand the barrier.
    resume.send(()).unwrap();
    snapshot.await.unwrap().unwrap();
    assert!(matches!(
        try_lock_descriptor(observer, LockMode::Exclusive).unwrap(),
        DescriptorLockAttempt::Acquired(_)
    ));
    let copied = DbHandle::open_external_reader(&destination.join("session.db")).unwrap();
    let stored = copied
        .read_body("0123456789af", "net_events", BodyDirection::Response)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.bytes, body.as_bytes());
    assert!(
        leased_before && leased_after,
        "raw snapshot source must exclude reclamation throughout copying"
    );
}
