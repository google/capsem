use super::*;

fn invalidation(kind: EnvironmentRequestInvalidationKind, at: i64) -> EnvironmentRequestInvalidation {
    EnvironmentRequestInvalidation {
        request_id: id(1),
        timestamp_unix_ms: at,
        actor: "system".into(),
        kind,
        reason: "request lifecycle reason canary".into(),
    }
}

#[tokio::test]
async fn expiry_is_immutable_and_refuses_late_decisions_and_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let mut request = proposal();
    request.expires_at_unix_ms = 20;
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
    request.expires_at_unix_ms = 21;
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    let mut late = decision(EnvironmentRequestDecisionKind::Approve);
    late.timestamp_unix_ms = 20;
    rejected(&writer, WriteOp::EnvironmentRequestDecision(late)).await;
    rejected(
        &writer,
        WriteOp::EnvironmentRequestInvalidation(invalidation(EnvironmentRequestInvalidationKind::Expired, 19)),
    )
    .await;
    let expired = invalidation(EnvironmentRequestInvalidationKind::Expired, 20);
    durable(&writer, WriteOp::EnvironmentRequestInvalidation(expired.clone())).await;
    durable(&writer, WriteOp::EnvironmentRequestInvalidation(expired)).await;
    rejected(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    rejected(
        &writer,
        WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: id(1),
            result: crate::events::EnvironmentRevisionParent {
                environment_id: id(2),
                revision_id: id(3),
            },
            timestamp_unix_ms: 21,
            actor: "operator".into(),
        }),
    )
    .await;
    writer.shutdown_blocking();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let snapshot = reader.environment_request_snapshot(&id(1)).await.unwrap().unwrap();
    assert_eq!(snapshot.record.state, EnvironmentRequestState::Expired);
    assert_eq!(snapshot.record.expires_at_unix_ms, 20);
    assert_eq!(
        snapshot.record.invalidation.unwrap().kind,
        EnvironmentRequestInvalidationKind::Expired
    );
    assert!(snapshot.record.decision.is_none());
}

#[tokio::test]
async fn stale_outcome_preserves_original_approval_and_archived_intent_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let request = proposal();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
    let approved = decision(EnvironmentRequestDecisionKind::Approve);
    durable(&writer, WriteOp::EnvironmentRequestDecision(approved.clone())).await;
    let stale = invalidation(EnvironmentRequestInvalidationKind::Stale, 12);
    durable(&writer, WriteOp::EnvironmentRequestInvalidation(stale.clone())).await;
    let mut changed = stale.clone();
    changed.reason = "changed evidence".into();
    rejected(&writer, WriteOp::EnvironmentRequestInvalidation(changed)).await;
    rejected(&writer, WriteOp::EnvironmentRequestDecision(approved.clone())).await;
    writer.shutdown_blocking();
    let writer = DbWriter::open(&path, 8).unwrap();
    durable(&writer, WriteOp::EnvironmentRequestInvalidation(stale.clone())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let snapshot = reader.environment_request_snapshot(&id(1)).await.unwrap().unwrap();
    assert_eq!(snapshot.record.state, EnvironmentRequestState::Stale);
    assert_eq!(snapshot.record.decision, Some(approved));
    assert_eq!(snapshot.record.invalidation, Some(stale));
    assert_eq!(snapshot.payload, request.payload);
    assert!(!format!("{:?}", snapshot.record).contains("reason canary"));
}

#[tokio::test]
async fn expiry_metadata_requires_a_future_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("host.db"), 8).unwrap();
    let mut request = proposal();
    request.expires_at_unix_ms = request.timestamp_unix_ms;
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
}

#[tokio::test]
async fn invalidation_state_evidence_and_audit_roll_back_together() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(proposal())).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_invalidation BEFORE INSERT ON host_events
        WHEN NEW.kind='environment_request_invalidated' BEGIN SELECT RAISE(ABORT,'injected invalidation audit failure'); END").unwrap();
    let stale = invalidation(EnvironmentRequestInvalidationKind::Stale, 12);
    rejected(&writer, WriteOp::EnvironmentRequestInvalidation(stale.clone())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let snapshot = reader.environment_request_snapshot(&id(1)).await.unwrap().unwrap();
    assert_eq!(snapshot.record.state, EnvironmentRequestState::Approved);
    assert!(snapshot.record.invalidation.is_none());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_request_invalidations", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER reject_invalidation").unwrap();
    durable(&writer, WriteOp::EnvironmentRequestInvalidation(stale)).await;
    assert_eq!(
        reader
            .environment_request_snapshot(&id(1))
            .await
            .unwrap()
            .unwrap()
            .record
            .state,
        EnvironmentRequestState::Stale
    );
}
