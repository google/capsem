use super::*;
use crate::db::EnvironmentHistoryPayload;
use crate::events::environment::request::{
    EnvironmentRequestDecision, EnvironmentRequestDecisionKind, EnvironmentRequestState,
};

async fn proposal_owner(path: &Path) -> DbWriter {
    let writer = owner(path, true).await;
    durable(
        &writer,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: Some(id(6)),
            requester: None,
            timestamp_unix_ms: 40,
            expires_at_unix_ms: 10000,
            actor: "operator".into(),
            reason: "proposal history".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(
                    toml::from_str(&format!(
                        "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='unrelated proposal field'"
                    ))
                    .unwrap(),
                ),
            }),
        }),
    )
    .await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(40),
            timestamp_unix_ms: 41,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "original review".into(),
        }),
    )
    .await;
    writer
}

#[tokio::test]
async fn variable_purge_request_history_preserves_partial_intent_and_review_then_reports_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = proposal_owner(&path).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let before = reader.environment_request_history(&id(40)).await.unwrap().unwrap();
    assert!(matches!(before.payload, EnvironmentHistoryPayload::Complete { .. }));
    assert_eq!(before.record.state, EnvironmentRequestState::Approved);
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    let partial = reader.environment_request_history(&id(40)).await.unwrap().unwrap();
    assert!(!partial.record.replayable);
    assert_eq!(partial.record.state, EnvironmentRequestState::Stale);
    assert_eq!(partial.record.decision.unwrap().reason, "original review");
    assert!(partial.record.invalidation.is_some());
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = partial.payload
    else {
        panic!("partial intent")
    };
    let EnvironmentRequestPayload::Change { specification } = *value else {
        panic!("change intent")
    };
    assert!(!specification.variables.contains_key("WRONG"));
    assert_eq!(specification.variables["KEEP"], "unrelated proposal field");
    assert_eq!(purged_variables.len(), 1);
    assert_eq!(purged_variables[0].purge_id, id(50));
    assert_eq!(purged_variables[0].variable_name, "WRONG");
    assert!(reader
        .environment_request_snapshot(&id(40))
        .await
        .unwrap_err()
        .contains("non-replayable"));
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    let expired = reader.environment_request_history(&id(40)).await.unwrap().unwrap();
    assert!(
        matches!(expired.payload,EnvironmentHistoryPayload::Unavailable {purged_variables} if purged_variables.len()==1)
    );
    assert_eq!(expired.record.decision.unwrap().reason, "original review");
    assert!(reader.environment_request_history(&id(99)).await.unwrap().is_none());
}

#[tokio::test]
async fn variable_purge_request_history_refuses_corrupt_metadata_payload_and_schema() {
    for mutation in [
        "UPDATE environment_requests SET replayable=1 WHERE request_id='00000000-0000-4000-8000-000000000028'",
        "UPDATE environment_variable_purges SET variable_name='KEEP'",
        "UPDATE environment_requests SET kind='image_update' WHERE request_id='00000000-0000-4000-8000-000000000028'",
        "UPDATE event_body_blobs SET body_hash='xxh3-128:00000000000000000000000000000000' WHERE event_id='000000000028'",
        "ALTER TABLE environment_request_purges RENAME TO missing_request_purges",
    ] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("host.db");let writer=proposal_owner(&path).await;
        durable(&writer,WriteOp::PrepareEnvironmentVariablePurge(intent())).await;writer.purge_variable_history(execution(51)).await.unwrap();
        let conn=Connection::open(&path).unwrap();conn.execute_batch(mutation).unwrap();
        let reader=crate::DbHandle::open_external_reader(&path).unwrap();
        assert!(!reader.environment_request_history(&id(40)).await.unwrap_err().contains(SELECTED));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = proposal_owner(&path).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM event_body_blobs WHERE event_id='000000000028'", [])
        .unwrap();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(reader
        .environment_request_history(&id(40))
        .await
        .unwrap_err()
        .contains("missing"));
    let count = reader.reader_requests();
    assert!(reader.environment_request_history("../bad").await.is_err());
    assert_eq!(reader.reader_requests(), count);
    writer.shutdown_blocking();
}

#[tokio::test]
async fn variable_purge_request_history_bounds_published_markers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = proposal_owner(&path).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    let conn = Connection::open(&path).unwrap();
    for n in 0..capsem_proto::ENVIRONMENT_MAX_VARIABLES {
        conn.execute("INSERT INTO environment_variable_purges(purge_id,environment_id,source_revision_id,variable_name,timestamp_unix_ms,actor,state,old_generation_id,new_generation_id) VALUES(?1,?2,?3,?4,50,'operator','committed',?5,?6)",rusqlite::params![id(100+n as u64),id(2),id(3),format!("KEY_{n}"),[1u8;16].as_slice(),[2u8;16].as_slice()]).unwrap();
        conn.execute(
            "INSERT INTO environment_request_purges(purge_id,request_id) VALUES(?1,?2)",
            rusqlite::params![id(100 + n as u64), id(40)],
        )
        .unwrap();
    }
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(reader
        .environment_request_history(&id(40))
        .await
        .unwrap_err()
        .contains("marker budget"));
}

#[tokio::test]
async fn variable_purge_request_history_captures_review_intent_and_markers_coherently() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = proposal_owner(&path).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let caller = reader.clone();
    let (reached, resume) = crate::db::pause_next_archive_capture_for_tests(&path);
    let capture = tokio::spawn(async move { caller.environment_request_history(&id(40)).await });
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    let result = writer.purge_variable_history(execution(51)).await;
    resume.send(()).unwrap();
    let old = capture.await.unwrap().unwrap().unwrap();
    assert_eq!(result.unwrap().state, EnvironmentVariablePurgeState::ReclaimPending);
    assert_eq!(old.record.state, EnvironmentRequestState::Approved);
    assert!(old.record.invalidation.is_none());
    let EnvironmentHistoryPayload::Complete { value } = old.payload else {
        panic!("original intent")
    };
    let EnvironmentRequestPayload::Change { specification } = *value else {
        panic!("original change")
    };
    assert_eq!(specification.variables["WRONG"], SELECTED);
    let new = reader.environment_request_history(&id(40)).await.unwrap().unwrap();
    assert_eq!(new.record.state, EnvironmentRequestState::Stale);
    assert_eq!(new.record.decision.unwrap().reason, "original review");
    assert!(new.record.invalidation.is_some());
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = new.payload
    else {
        panic!("partial intent")
    };
    let EnvironmentRequestPayload::Change { specification } = *value else {
        panic!("partial change")
    };
    assert!(!specification.variables.contains_key("WRONG"));
    assert_eq!(purged_variables[0].state, EnvironmentVariablePurgeState::ReclaimPending);
    assert_eq!(
        writer.purge_variable_history(execution(52)).await.unwrap().state,
        EnvironmentVariablePurgeState::Committed
    );
}
