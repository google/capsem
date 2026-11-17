//! Recover the durable revision even before the final request receipt was written.
use super::*;
use crate::events::environment::request::{
    EnvironmentRequestDecision, EnvironmentRequestDecisionKind, EnvironmentRequestPayload, PreparedEnvironmentRequest,
};

#[tokio::test]
async fn request_result_revision_lookup_survives_reopen_before_result_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    let query = EnvironmentRevisionQuery::ForRequest { request_id: id(40) };
    assert!(db.environment_revisions(query.clone()).await.unwrap().is_empty());
    let mut result = record(1, 11, 40);
    result.request_id = Some(id(40));
    for event in [
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(1),
            base_revision_id: None,
            requester: None,
            timestamp_unix_ms: 30,
            expires_at_unix_ms: 1000,
            actor: "operator".into(),
            reason: "reviewed content".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: result.specification.clone(),
            }),
        }),
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(40),
            timestamp_unix_ms: 31,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "approved".into(),
        }),
        WriteOp::PrepareEnvironmentRevision(result),
    ] {
        db.write(event).await.unwrap();
        db.flush().await.unwrap();
    }
    let prepared = db.environment_revisions(query.clone()).await.unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].state, EnvironmentRevisionState::Committing);
    db.write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: id(1),
        revision_id: id(11),
        state: EnvironmentRevisionState::Committed,
        timestamp_unix_ms: 41,
        actor: "operator".into(),
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
    let request = db.environment_request_snapshot(&id(40)).await.unwrap().unwrap();
    assert!(request.record.result.is_none(), "simulate crash before final receipt");
    drop(db);
    let reopened = DbHandle::open_external_reader(&path).unwrap();
    let found = reopened.environment_revisions(query).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, Some(id(40)));
    assert_eq!(found[0].revision_id, id(11));
    assert_eq!(found[0].state, EnvironmentRevisionState::Committed);
    assert!(reopened
        .environment_revisions(EnvironmentRevisionQuery::ForRequest { request_id: id(99) })
        .await
        .unwrap()
        .is_empty());
    for invalid in ["bad".to_owned(), uuid::Uuid::nil().to_string(), id(0xab).to_uppercase()] {
        assert!(reopened
            .environment_revisions(EnvironmentRevisionQuery::ForRequest { request_id: invalid })
            .await
            .is_err());
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    let plan: String = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT revision_id FROM environment_revisions WHERE request_id=?1",
            [id(40)],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("idx_environment_revisions_request"), "{plan}");
}
