use super::*;
use crate::events::{EnvironmentRevisionTransition, PreparedEnvironmentRevision};
#[path = "tests/recovery.rs"]
mod recovery;

fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn record(environment: u64, revision: u64, at: i64) -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(revision),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: at,
        actor: "operator".into(),
        reason: "query-reason-canary".into(),
        request_id: None,
        payload_event_id: format!("{revision:012x}"),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    }
}
fn page(after_id: i64, limit: u16) -> EnvironmentRevisionQuery {
    EnvironmentRevisionQuery::Page {
        environment_id: None,
        state: None,
        after_id,
        limit,
    }
}
async fn prepare(db: &DbHandle, record: PreparedEnvironmentRevision) {
    db.write(WriteOp::PrepareEnvironmentRevision(record)).await.unwrap();
    db.flush().await.unwrap();
}

#[tokio::test]
async fn revision_queries_page_by_insertion_and_filter_exact_identity_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&dir.path().join("host.db")).unwrap();
    assert!(db.environment_revisions(page(0, 2)).await.unwrap().is_empty());
    let first = record(1, 11, 200);
    prepare(&db, first.clone()).await;
    prepare(&db, record(2, 12, 100)).await;
    let records = db.environment_revisions(page(0, 1)).await.unwrap();
    assert_eq!(records.len(), 1);
    let row = &records[0];
    assert_eq!(row.environment_id, first.environment_id);
    assert_eq!(row.revision_id, first.revision_id);
    assert_eq!(row.timestamp_unix_ms, 200);
    assert_eq!(row.actor, "operator");
    assert_eq!(row.reason, first.reason);
    assert_eq!(row.payload_event_id, first.payload_event_id);
    assert!(row.replayable);
    assert_eq!(row.state, EnvironmentRevisionState::Committing);
    assert!(!format!("{row:?}").contains("query-reason-canary"));
    let next = db.environment_revisions(page(row.cursor, 2)).await.unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].revision_id, id(12), "timestamp must not define cursor order");
    assert!(db
        .environment_revisions(EnvironmentRevisionQuery::Get {
            environment_id: id(2),
            revision_id: id(11),
        })
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        db.environment_revisions(EnvironmentRevisionQuery::Get {
            environment_id: id(1),
            revision_id: id(11),
        })
        .await
        .unwrap(),
        records
    );
    db.write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: id(1),
        revision_id: id(11),
        state: EnvironmentRevisionState::Committed,
        timestamp_unix_ms: 300,
        actor: "operator".into(),
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
    let committed = db
        .environment_revisions(EnvironmentRevisionQuery::Page {
            environment_id: Some(id(1)),
            state: Some(EnvironmentRevisionState::Committed),
            after_id: 0,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].revision_id, id(11));
    assert_eq!(
        db.environment_revisions(EnvironmentRevisionQuery::Page {
            environment_id: None,
            state: Some(EnvironmentRevisionState::Committing),
            after_id: 0,
            limit: 2,
        })
        .await
        .unwrap()[0]
            .revision_id,
        id(12)
    );
}

#[tokio::test]
async fn revision_queries_observe_external_commits_and_report_missing_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let owner = DbHandle::open(&path).unwrap();
    let reader = DbHandle::open_external_reader(&path).unwrap();
    assert!(reader.environment_revisions(page(0, 1)).await.unwrap().is_empty());
    let epoch = reader.read_cache_epoch(ReadCacheDomain::All);
    prepare(&owner, record(1, 11, 1)).await;
    assert_eq!(reader.environment_revisions(page(0, 1)).await.unwrap().len(), 1);
    assert!(reader.read_cache_epoch(ReadCacheDomain::All) > epoch);
    let tamper = rusqlite::Connection::open(&path).unwrap();
    tamper
        .execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE environment_revisions SET state='unknown'")
        .unwrap();
    assert!(reader
        .environment_revisions(page(0, 1))
        .await
        .unwrap_err()
        .contains("unknown environment revision state"));
    tamper.execute_batch("DROP TABLE environment_revisions").unwrap();
    assert!(
        reader.environment_revisions(page(0, 1)).await.is_err(),
        "broken schema is not empty history"
    );
}

#[tokio::test]
async fn invalid_revision_queries_are_refused_before_worker_admission() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&dir.path().join("host.db")).unwrap();
    for query in [
        page(-1, 1),
        page(0, 0),
        page(0, u16::MAX),
        EnvironmentRevisionQuery::Get {
            environment_id: "not-an-identity".into(),
            revision_id: id(11),
        },
    ] {
        let before = db.reader_requests();
        assert!(db.environment_revisions(query).await.is_err());
        assert_eq!(db.reader_requests(), before);
    }
}

#[tokio::test]
async fn environment_snapshots_reconstruct_exact_archived_specification_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&dir.path().join("host.db")).unwrap();
    let mut source = record(1, 11, 100);
    source
        .specification
        .variables
        .insert("PRIVATE".into(), "snapshot-reconstruction-canary".into());
    assert!(db
        .environment_revision_snapshot(&id(1), &id(11))
        .await
        .unwrap()
        .is_none());
    prepare(&db, source.clone()).await;
    let snapshot = db
        .environment_revision_snapshot(&id(1), &id(11))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.specification, source.specification);
    assert_eq!(snapshot.record.state, EnvironmentRevisionState::Committing);
    assert!(!format!("{snapshot:?}").contains("snapshot-reconstruction-canary"));
    db.write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: id(1),
        revision_id: id(11),
        state: EnvironmentRevisionState::Committed,
        timestamp_unix_ms: 101,
        actor: "operator".into(),
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
    let committed = db
        .environment_revision_snapshot(&id(1), &id(11))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed.record.state, EnvironmentRevisionState::Committed);
    assert_eq!(committed.specification, snapshot.specification);
}

#[tokio::test]
async fn environment_snapshots_refuse_purged_truncated_and_missing_payloads() {
    for (damage, expected) in [
        ("UPDATE environment_revisions SET replayable=0", "non-replayable"),
        (
            "UPDATE event_body_blobs SET truncated=1 WHERE source_table='environment_revisions'",
            "truncated",
        ),
        (
            "DELETE FROM event_body_blobs WHERE source_table='environment_revisions'",
            "missing",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let db = DbHandle::open(&path).unwrap();
        prepare(&db, record(1, 11, 1)).await;
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(damage)
            .unwrap();
        let error = db.environment_revision_snapshot(&id(1), &id(11)).await.unwrap_err();
        assert!(error.contains(expected), "{damage}: {error}");
    }
}

#[tokio::test]
async fn environment_snapshot_keeps_replayability_and_bytes_in_one_read_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let owner = DbHandle::open(&path).unwrap();
    let source = record(1, 11, 1);
    prepare(&owner, source.clone()).await;
    let reader = DbHandle::open_external_reader(&path).unwrap();
    let (reached, resume) = bodies::pause_next_archive_capture_for_tests(&path);
    let request = tokio::spawn(async move { reader.environment_revision_snapshot(&id(1), &id(11)).await });
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .expect("capture reached its pinned SQL snapshot");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE environment_revisions SET replayable=0", [])
        .unwrap();
    resume.send(()).unwrap();
    // This read began before the marker changed. It must observe the complete
    // old snapshot; a subsequent read must refuse the now-purged revision.
    let captured = request.await.unwrap().unwrap().unwrap();
    assert!(captured.record.replayable);
    assert_eq!(captured.specification, source.specification);
    assert!(owner
        .environment_revision_snapshot(&id(1), &id(11))
        .await
        .unwrap_err()
        .contains("non-replayable"));
}
