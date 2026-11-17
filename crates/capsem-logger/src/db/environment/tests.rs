use super::*;
use crate::events::{EnvironmentRevisionTransition, PreparedEnvironmentRevision};

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
