use super::*;
use crate::events::environment::request::PreparedEnvironmentRequest;

fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn proposal(number: u64, generation: u64) -> PreparedEnvironmentRequest {
    PreparedEnvironmentRequest {
        request_id: id(number),
        environment_id: id(2),
        base_revision_id: Some(id(3)),
        requester: Some(EnvironmentRequestTarget {
            session_id: id(4),
            generation: id(generation),
        }),
        timestamp_unix_ms: 200 - number as i64,
        actor: format!("agent:{}", id(10)),
        reason: "request-query-reason-canary".into(),
        payload_event_id: format!("{number:012x}"),
        payload: Box::new(EnvironmentRequestPayload::Change {
            specification: Box::new(
                toml::from_str("name='development'\n[variables]\nPRIVATE='request-query-value-canary'").unwrap(),
            ),
        }),
    }
}
async fn durable(db: &DbHandle, op: WriteOp) {
    db.write(op).await.unwrap();
    db.flush().await.unwrap();
}
async fn owner(path: &std::path::Path) -> DbHandle {
    let db = DbHandle::open(path).unwrap();
    durable(
        &db,
        WriteOp::PrepareEnvironmentRevision(crate::events::PreparedEnvironmentRevision {
            environment_id: id(2),
            revision_id: id(3),
            previous_revision_id: None,
            fork_parent: None,
            timestamp_unix_ms: 1,
            actor: "operator".into(),
            reason: "base environment".into(),
            request_id: None,
            payload_event_id: "abcdef012345".into(),
            specification: Box::new(toml::from_str("name='development'").unwrap()),
        }),
    )
    .await;
    durable(
        &db,
        WriteOp::EnvironmentRevisionTransition(crate::events::EnvironmentRevisionTransition {
            environment_id: id(2),
            revision_id: id(3),
            state: crate::events::EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 2,
            actor: "operator".into(),
        }),
    )
    .await;
    db
}
fn page(after_id: i64, limit: u16) -> EnvironmentRequestQuery {
    EnvironmentRequestQuery::Page {
        environment_id: None,
        requester: None,
        pending_only: false,
        after_id,
        limit,
    }
}

#[tokio::test]
async fn request_views_page_by_cursor_and_filter_exact_requester_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = owner(&path).await;
    assert!(db.environment_requests(page(0, 1)).await.unwrap().is_empty());
    durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(11, 5))).await;
    durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(12, 6))).await;
    let first = db.environment_requests(page(0, 1)).await.unwrap();
    assert_eq!(first[0].request_id, id(11));
    assert_eq!(first[0].state, EnvironmentRequestState::Pending);
    assert!(first[0].decision.is_none());
    assert!(!format!("{:?}", first[0]).contains("request-query-reason-canary"));
    let next = db.environment_requests(page(first[0].cursor, 2)).await.unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].request_id, id(12));
    let selected = db
        .environment_requests(EnvironmentRequestQuery::Page {
            environment_id: Some(id(2)),
            requester: Some(EnvironmentRequestTarget {
                session_id: id(4),
                generation: id(5),
            }),
            pending_only: true,
            after_id: 0,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(selected, first);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let plan: String=conn.query_row("EXPLAIN QUERY PLAN SELECT id FROM environment_requests WHERE requester_session_id=?1 AND requester_generation=?2 AND id>?3 ORDER BY id LIMIT ?4", rusqlite::params![id(4),id(5),0,2], |row| row.get(3)).unwrap();
    assert!(
        plan.contains("idx_environment_requests_requester"),
        "scoped request history must not scan other requesters' history: {plan}"
    );
    assert!(db.environment_request_snapshot(&id(99)).await.unwrap().is_none());
    assert!(db
        .environment_requests(EnvironmentRequestQuery::Get { request_id: id(99) })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn exact_request_snapshot_and_decision_survive_reopen_and_external_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = owner(&path).await;
    let source = proposal(11, 5);
    durable(&db, WriteOp::PrepareEnvironmentRequest(source.clone())).await;
    let reader = DbHandle::open_external_reader(&path).unwrap();
    let initial = reader.environment_request_snapshot(&id(11)).await.unwrap().unwrap();
    assert_eq!(*initial.payload, *source.payload);
    assert!(!format!("{initial:?}").contains("request-query-value-canary"));
    let epoch = reader.read_cache_epoch(ReadCacheDomain::All);
    let decision = EnvironmentRequestDecision {
        request_id: id(11),
        timestamp_unix_ms: 201,
        actor: "operator".into(),
        decision: EnvironmentRequestDecisionKind::Deny,
        reason: "decision-query-reason-canary".into(),
    };
    durable(&db, WriteOp::EnvironmentRequestDecision(decision.clone())).await;
    let resolved = reader.environment_request_snapshot(&id(11)).await.unwrap().unwrap();
    assert_eq!(resolved.record.state, EnvironmentRequestState::Denied);
    assert_eq!(resolved.record.decision, Some(decision));
    assert!(reader.read_cache_epoch(ReadCacheDomain::All) > epoch);
    assert!(!format!("{resolved:?}").contains("decision-query-reason-canary"));
    drop(reader);
    drop(db);
    let reopened = DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(
        reopened.environment_request_snapshot(&id(11)).await.unwrap().unwrap(),
        resolved
    );
}

#[tokio::test]
async fn request_snapshot_refuses_missing_truncated_and_purged_required_payloads() {
    for (damage, message) in [
        ("UPDATE environment_requests SET replayable=0", "purged"),
        (
            "UPDATE event_body_blobs SET truncated=1 WHERE source_table='environment_requests'",
            "truncated",
        ),
        (
            "DELETE FROM event_body_blobs WHERE source_table='environment_requests'",
            "missing",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let db = owner(&path).await;
        durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(11, 5))).await;
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(damage)
            .unwrap();
        let error = db.environment_request_snapshot(&id(11)).await.unwrap_err();
        assert!(error.contains(message), "{damage}: {error}");
    }
}

#[tokio::test]
async fn request_query_bounds_and_broken_schema_never_become_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = owner(&path).await;
    for query in [
        page(-1, 1),
        page(0, 0),
        page(0, u16::MAX),
        EnvironmentRequestQuery::Get {
            request_id: "invalid".into(),
        },
    ] {
        let requests = db.reader_requests();
        assert!(db.environment_requests(query).await.is_err());
        assert_eq!(db.reader_requests(), requests);
    }
    durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(11, 5))).await;
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE environment_requests SET state='approved'")
        .unwrap();
    assert!(db
        .environment_requests(page(0, 1))
        .await
        .unwrap_err()
        .contains("decision evidence"));
    conn.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE environment_requests SET state='unknown'")
        .unwrap();
    assert!(db.environment_requests(page(0, 1)).await.is_err());
    conn.execute_batch("DROP TABLE environment_requests").unwrap();
    assert!(db.environment_requests(page(0, 1)).await.is_err());
}

#[tokio::test]
async fn request_view_keeps_original_actor_operator_decision_and_committed_fork_result() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = owner(&path).await;
    durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(11, 5))).await;
    durable(
        &db,
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(11),
            timestamp_unix_ms: 201,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "approved fork".into(),
        }),
    )
    .await;
    let selected = crate::events::EnvironmentRevisionParent {
        environment_id: id(6),
        revision_id: id(7),
    };
    durable(
        &db,
        WriteOp::PrepareEnvironmentRevision(crate::events::PreparedEnvironmentRevision {
            environment_id: id(6),
            revision_id: id(7),
            previous_revision_id: None,
            fork_parent: Some(crate::events::EnvironmentRevisionParent {
                environment_id: id(2),
                revision_id: id(3),
            }),
            timestamp_unix_ms: 202,
            actor: "operator".into(),
            reason: "approved fork".into(),
            request_id: Some(id(11)),
            payload_event_id: "abcdef012346".into(),
            specification: Box::new(
                toml::from_str("name='development'\n[variables]\nPRIVATE='request-query-value-canary'").unwrap(),
            ),
        }),
    )
    .await;
    durable(
        &db,
        WriteOp::EnvironmentRevisionTransition(crate::events::EnvironmentRevisionTransition {
            environment_id: id(6),
            revision_id: id(7),
            state: crate::events::EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 203,
            actor: "operator".into(),
        }),
    )
    .await;
    durable(
        &db,
        WriteOp::EnvironmentRequestResult(crate::events::environment::request::EnvironmentRequestResult {
            request_id: id(11),
            result: selected.clone(),
            timestamp_unix_ms: 204,
            actor: "operator".into(),
        }),
    )
    .await;
    let view = db.environment_request_snapshot(&id(11)).await.unwrap().unwrap();
    assert_eq!(view.record.state, EnvironmentRequestState::Approved);
    assert_eq!(view.record.actor, format!("agent:{}", id(10)));
    assert_eq!(view.record.decision.as_ref().unwrap().actor, "operator");
    assert_eq!(view.record.result, Some(selected));
    assert_eq!(
        view.record.requester,
        Some(EnvironmentRequestTarget {
            session_id: id(4),
            generation: id(5)
        })
    );
}

#[tokio::test]
async fn request_capture_keeps_payload_and_decision_in_one_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = owner(&path).await;
    durable(&db, WriteOp::PrepareEnvironmentRequest(proposal(11, 5))).await;
    let reader = DbHandle::open_external_reader(&path).unwrap();
    let (reached, resume) = crate::db::bodies::pause_next_archive_capture_for_tests(&path);
    let request = tokio::spawn(async move { reader.environment_request_snapshot(&id(11)).await });
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    durable(
        &db,
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(11),
            timestamp_unix_ms: 201,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Deny,
            reason: "declined after capture began".into(),
        }),
    )
    .await;
    resume.send(()).unwrap();
    let earlier = request.await.unwrap().unwrap().unwrap();
    assert_eq!(earlier.record.state, EnvironmentRequestState::Pending);
    assert!(earlier.record.decision.is_none());
    let current = db.environment_request_snapshot(&id(11)).await.unwrap().unwrap();
    assert_eq!(current.record.state, EnvironmentRequestState::Denied);
    assert_eq!(current.payload, earlier.payload);
}
