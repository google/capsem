use super::*;
use crate::db::EnvironmentHistoryPayload;

#[tokio::test]
async fn variable_purge_revision_history_distinguishes_complete_partial_and_unavailable_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = owner(&path, true).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let complete = reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap()
        .unwrap();
    let EnvironmentHistoryPayload::Complete { value } = complete.payload else {
        panic!("complete revision")
    };
    assert_eq!(value.variables["WRONG"], SELECTED);
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    crate::writer::fail_retention_for_path_for_tests(&path, crate::writer::RetentionFault::IndexTransaction);
    assert!(writer.purge_variable_history(execution(51)).await.is_err());
    assert!(
        matches!(
            reader
                .environment_revision_history(&id(2), &id(3))
                .await
                .unwrap()
                .unwrap()
                .payload,
            EnvironmentHistoryPayload::Complete { .. }
        ),
        "unpublished purge targets must not become purge markers"
    );
    writer.purge_variable_history(execution(52)).await.unwrap();
    let partial = reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap()
        .unwrap();
    assert!(!partial.record.replayable);
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = partial.payload
    else {
        panic!("partial revision")
    };
    assert!(!value.variables.contains_key("WRONG"));
    assert_eq!(value.variables["KEEP"], "unrelated value");
    assert_eq!(purged_variables.len(), 1);
    assert_eq!(purged_variables[0].variable_name, "WRONG");
    assert_eq!(purged_variables[0].purge_id, id(50));
    assert_eq!(purged_variables[0].state, EnvironmentVariablePurgeState::Committed);
    assert!(reader
        .environment_revision_snapshot(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("non-replayable"));
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    assert!(matches!(
        reader
            .environment_revision_history(&id(2), &id(3))
            .await
            .unwrap()
            .unwrap()
            .payload,
        EnvironmentHistoryPayload::Purged { .. }
    ));
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM environment_revision_references WHERE owner_id=?1", [id(2)])
        .unwrap();
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    let unavailable = reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap()
        .unwrap();
    let EnvironmentHistoryPayload::Unavailable { purged_variables } = unavailable.payload else {
        panic!("unavailable revision")
    };
    assert_eq!(purged_variables.len(), 1);
    let expired = reader
        .environment_revision_history(&id(2), &id(4))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(expired.payload,EnvironmentHistoryPayload::Unavailable {purged_variables} if purged_variables.is_empty())
    );
    assert!(reader
        .environment_revision_history(&id(8), &id(3))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn variable_purge_revision_history_refuses_contradictory_corrupt_or_missing_schema() {
    for mutation in [
        "UPDATE environment_revisions SET replayable=1 WHERE revision_id='00000000-0000-4000-8000-000000000003'",
        "UPDATE environment_variable_purges SET variable_name='KEEP'",
        "UPDATE environment_variable_purges SET variable_name='../WRONG'",
        "UPDATE event_body_blobs SET body_hash='xxh3-128:00000000000000000000000000000000' WHERE event_id='000000000003'",
        "ALTER TABLE environment_revision_purges RENAME TO missing_revision_purges",
    ] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("host.db");let writer=owner(&path,true).await;
        durable(&writer,WriteOp::PrepareEnvironmentVariablePurge(intent())).await;writer.purge_variable_history(execution(51)).await.unwrap();
        let conn=Connection::open(&path).unwrap();conn.execute_batch(mutation).unwrap();
        let reader=crate::DbHandle::open_external_reader(&path).unwrap();
        let error=reader.environment_revision_history(&id(2),&id(3)).await.unwrap_err();assert!(!error.contains(SELECTED));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM event_body_blobs WHERE event_id='000000000003'", [])
        .unwrap();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(
        reader
            .environment_revision_history(&id(2), &id(3))
            .await
            .unwrap_err()
            .contains("missing"),
        "missing replayable payload must not masquerade as expired history"
    );
    let requests = reader.reader_requests();
    assert!(reader.environment_revision_history("bad", &id(3)).await.is_err());
    assert_eq!(reader.reader_requests(), requests);
    writer.shutdown_blocking();
}

#[tokio::test]
async fn variable_purge_revision_history_bounds_published_marker_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    let conn = Connection::open(&path).unwrap();
    for n in 0..capsem_proto::ENVIRONMENT_MAX_VARIABLES {
        conn.execute("INSERT INTO environment_variable_purges(purge_id,environment_id,source_revision_id,variable_name,timestamp_unix_ms,actor,state,old_generation_id,new_generation_id) VALUES(?1,?2,?3,?4,50,'operator','committed',?5,?6)",rusqlite::params![id(100+n as u64),id(2),id(3),format!("KEY_{n}"),[1u8;16].as_slice(),[2u8;16].as_slice()]).unwrap();
        conn.execute(
            "INSERT INTO environment_revision_purges(purge_id,revision_id) VALUES(?1,?2)",
            rusqlite::params![id(100 + n as u64), id(3)],
        )
        .unwrap();
    }
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("marker budget"));
}

#[tokio::test]
async fn variable_purge_revision_history_captures_values_and_markers_from_one_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let caller = reader.clone();
    let (reached, resume) = crate::db::pause_next_archive_capture_for_tests(&path);
    let capture = tokio::spawn(async move { caller.environment_revision_history(&id(2), &id(3)).await });
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    let result = writer.purge_variable_history(execution(51)).await;
    resume.send(()).unwrap();
    let old = capture.await.unwrap().unwrap().unwrap();
    assert_eq!(result.unwrap().state, EnvironmentVariablePurgeState::ReclaimPending);
    let EnvironmentHistoryPayload::Complete { value } = old.payload else {
        panic!("old complete capture")
    };
    assert_eq!(value.variables["WRONG"], SELECTED);
    let new = reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap()
        .unwrap();
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = new.payload
    else {
        panic!("new partial capture")
    };
    assert!(!value.variables.contains_key("WRONG"));
    assert_eq!(purged_variables[0].state, EnvironmentVariablePurgeState::ReclaimPending);
    assert_eq!(
        writer.purge_variable_history(execution(52)).await.unwrap().state,
        EnvironmentVariablePurgeState::Committed
    );
}
