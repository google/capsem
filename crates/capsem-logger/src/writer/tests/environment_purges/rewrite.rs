use super::*;

fn execution(at: i64) -> EnvironmentVariablePurgeExecution {
    EnvironmentVariablePurgeExecution {
        purge_id: id(50),
        timestamp_unix_ms: at,
    }
}

#[tokio::test]
async fn variable_purge_rewrite_preserves_unrelated_values_and_expires_full_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let conn = Connection::open(&path).unwrap();
    let old = crate::schema::archive_state(&conn).unwrap();
    let rows_before: i64 = conn
        .query_row("SELECT SUM(id) FROM environment_revisions", [], |row| row.get(0))
        .unwrap();
    let result = writer.purge_variable_history(execution(51)).await.unwrap();
    assert_eq!(result.state, EnvironmentVariablePurgeState::Committed);
    let published = crate::schema::archive_state(&conn).unwrap();
    assert_ne!(published.header.generation_id, old.header.generation_id);
    assert!(!crate::writer::archive_path_for_db(&path)
        .join(old.header.generation_id.file_name())
        .exists());
    assert_eq!(
        conn.query_row("SELECT SUM(id) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        rows_before
    );
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    for number in [3, 5] {
        assert!(reader
            .environment_revision_snapshot(&id(2), &id(number))
            .await
            .unwrap_err()
            .contains("non-replayable"));
        let body = reader
            .read_body(
                &format!("{number:012x}"),
                "environment_revisions",
                crate::BodyDirection::Payload,
            )
            .await
            .unwrap()
            .unwrap();
        let partial: capsem_config::environment::EnvironmentSpecification = rmp_serde::from_slice(&body.bytes).unwrap();
        assert!(!partial.variables.contains_key("WRONG"));
        assert_eq!(partial.variables["KEEP"], "unrelated value");
    }
    assert_eq!(
        reader
            .environment_revision_snapshot(&id(2), &id(4))
            .await
            .unwrap()
            .unwrap()
            .specification
            .variables["WRONG"],
        "different legitimate value"
    );
    assert_eq!(
        reader
            .environment_revision_snapshot(&id(8), &id(10))
            .await
            .unwrap()
            .unwrap()
            .specification
            .variables["WRONG"],
        SELECTED
    );
    let again = writer.purge_variable_history(execution(52)).await.unwrap();
    assert_eq!(again, result);
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), published);
}

#[tokio::test]
async fn variable_purge_rewrite_does_not_retain_forbidden_compression_dictionary_bytes() {
    use capsem_archive::{BlockExtent, BodyLogReader, BodyRef};
    use capsem_foundation::unix::contained::ContainedDir;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    committed(&writer, revision(2, 3, None, Some(SELECTED))).await;
    committed(&writer, revision(2, 6, Some(3), None)).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    let conn = Connection::open(&path).unwrap();
    let state = crate::schema::archive_state(&conn).unwrap();
    let directory = ContainedDir::open_root(&crate::writer::archive_path_for_db(&path)).unwrap();
    let reader = BodyLogReader::open_generation(&directory, state.header, state.committed_end).unwrap();
    let mut statement = conn
        .prepare("SELECT block_offset,disk_len,raw_len FROM body_blocks ORDER BY block_offset")
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })
        .unwrap();
    let mut blocks = 0;
    for row in rows {
        let (offset, disk_len, raw_len) = row.unwrap();
        let bytes = reader
            .read_bounded(
                BodyRef {
                    block_offset: offset as u64,
                    offset: 0,
                    len: raw_len as u32,
                },
                BlockExtent {
                    disk_len: disk_len as u64,
                    raw_len: raw_len as u32,
                },
            )
            .unwrap();
        assert!(
            !bytes
                .windows(SELECTED.len())
                .any(|window| window == SELECTED.as_bytes()),
            "fresh compressed streams must not retain removed values outside indexed spans"
        );
        blocks += 1;
    }
    assert!(blocks > 0);
}

#[tokio::test]
async fn variable_purge_rewrite_keeps_reclaim_pending_until_captured_reader_releases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let (reached, resume) = crate::db::pause_next_archive_capture_for_tests(&path);
    let capture = tokio::spawn(async move { reader.environment_revision_snapshot(&id(2), &id(3)).await });
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    let first = writer.purge_variable_history(execution(51)).await.unwrap();
    let conn = Connection::open(&path).unwrap();
    let state = crate::schema::archive_state(&conn).unwrap();
    // Resume before assertions so RED cannot strand a native reader barrier.
    resume.send(()).unwrap();
    let captured = capture.await.unwrap().unwrap().unwrap();
    assert_eq!(captured.specification.variables["WRONG"], SELECTED);
    assert_eq!(first.state, EnvironmentVariablePurgeState::ReclaimPending);
    let completed = writer.purge_variable_history(execution(52)).await.unwrap();
    assert_eq!(completed.state, EnvironmentVariablePurgeState::Committed);
    assert_eq!(
        crate::schema::archive_state(&conn).unwrap(),
        state,
        "reclamation retry must not rewrite already-published bytes"
    );
}

#[tokio::test]
async fn variable_purge_rewrite_rolls_back_index_markers_and_root_on_publication_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let conn = Connection::open(&path).unwrap();
    let old = crate::schema::archive_state(&conn).unwrap();
    crate::writer::fail_retention_for_path_for_tests(&path, crate::writer::RetentionFault::IndexTransaction);
    assert!(writer.purge_variable_history(execution(51)).await.is_err());
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), old);
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(
        reader
            .environment_revision_snapshot(&id(2), &id(3))
            .await
            .unwrap()
            .unwrap()
            .specification
            .variables["WRONG"],
        SELECTED
    );
    assert_eq!(
        writer.purge_variable_history(execution(52)).await.unwrap().state,
        EnvironmentVariablePurgeState::Committed
    );
}

#[tokio::test]
async fn variable_purge_rewrite_retains_partial_ancestors_without_restoring_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let partial = reader
        .read_body("000000000003", "environment_revisions", crate::BodyDirection::Payload)
        .await
        .unwrap()
        .expect("retained ancestor must preserve its unrelated fields after purge");
    let spec: capsem_config::environment::EnvironmentSpecification = rmp_serde::from_slice(&partial.bytes).unwrap();
    assert_eq!(spec.variables["KEEP"], "unrelated value");
    assert!(!spec.variables.contains_key("WRONG"));
    assert!(reader
        .environment_revision_snapshot(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

#[tokio::test]
async fn variable_purge_rewrite_recovers_both_uncertain_publication_outcomes_after_restart() {
    for fault in [
        crate::writer::RetentionFault::CommitUnknownBefore,
        crate::writer::RetentionFault::CommitUnknownAfter,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let writer = owner(&path, true).await;
        durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
        crate::writer::fail_retention_for_path_for_tests(&path, fault);
        assert!(writer
            .purge_variable_history(execution(51))
            .await
            .unwrap_err()
            .contains("unknown"));
        writer.shutdown_blocking();
        let writer = DbWriter::open(&path, 8).unwrap();
        assert_eq!(
            writer.purge_variable_history(execution(52)).await.unwrap().state,
            EnvironmentVariablePurgeState::Committed
        );
        let reader = crate::DbHandle::open_external_reader(&path).unwrap();
        assert!(reader
            .environment_revision_snapshot(&id(2), &id(3))
            .await
            .unwrap_err()
            .contains("non-replayable"));
    }
}

#[tokio::test]
async fn variable_purge_rewrite_stales_proposals_and_preserves_operator_decisions() {
    use crate::events::environment::request::{EnvironmentRequestDecision, EnvironmentRequestDecisionKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    for number in [40, 41, 42] {
        durable(
            &writer,
            WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
                request_id: id(number),
                environment_id: id(2),
                base_revision_id: Some(id(6)),
                requester: None,
                timestamp_unix_ms: 40,
                expires_at_unix_ms: 10000,
                actor: "operator".into(),
                reason: "proposal".into(),
                payload_event_id: format!("{number:012x}"),
                payload: Box::new(EnvironmentRequestPayload::Change {
                    specification: Box::new(
                        toml::from_str(&format!(
                            "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='proposal field'"
                        ))
                        .unwrap(),
                    ),
                }),
            }),
        )
        .await;
        if number != 40 {
            durable(
                &writer,
                WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
                    request_id: id(number),
                    timestamp_unix_ms: 43,
                    actor: "operator".into(),
                    decision: if number == 41 {
                        EnvironmentRequestDecisionKind::Approve
                    } else {
                        EnvironmentRequestDecisionKind::Deny
                    },
                    reason: "original decision".into(),
                }),
            )
            .await;
        }
    }
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let result = writer.purge_variable_history(execution(51)).await.unwrap();
    assert_eq!(result.request_count, 3);
    let conn = Connection::open(&path).unwrap();
    for number in [40, 41, 42] {
        let (state, replayable): (String, bool) = conn
            .query_row(
                "SELECT state,replayable FROM environment_requests WHERE request_id=?1",
                [id(number)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, if number == 42 { "denied" } else { "stale" });
        assert!(!replayable);
    }
    let decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM environment_request_decisions WHERE reason='original decision'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(decisions, 2);
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let body = reader
        .read_body("000000000028", "environment_requests", crate::BodyDirection::Payload)
        .await
        .unwrap()
        .unwrap();
    let payload: EnvironmentRequestPayload = rmp_serde::from_slice(&body.bytes).unwrap();
    let EnvironmentRequestPayload::Change { specification } = payload else {
        panic!("wrong proposal kind")
    };
    assert_eq!(specification.variables["KEEP"], "proposal field");
    assert!(!specification.variables.contains_key("WRONG"));
}

#[tokio::test]
async fn variable_purge_rewrite_refuses_a_second_plan_until_reclamation_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    committed(&writer, revision(8, 11, Some(10), None)).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let mut second = intent();
    second.purge_id = id(51);
    second.environment_id = id(8);
    second.source_revision_id = id(10);
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(second)).await;
}

#[tokio::test]
async fn variable_purge_rewrite_refuses_new_unplanned_copies_and_current_pointer_corruption() {
    for corruption in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let writer = owner(&path, true).await;
        durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
        if corruption {
            let conn = Connection::open(&path).unwrap();
            conn.execute("UPDATE environment_revision_references SET environment_id=?1,revision_id=?2 WHERE owner_kind='current_environment' AND owner_id=?3",rusqlite::params![id(8),id(10),id(2)]).unwrap();
        } else {
            committed(&writer, revision(2, 7, Some(6), Some(SELECTED))).await;
            committed(&writer, revision(2, 9, Some(7), None)).await;
        }
        assert!(writer.purge_variable_history(execution(51)).await.is_err());
    }
}

#[tokio::test]
async fn variable_purge_rewrite_allows_replanning_after_history_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    committed(&writer, revision(2, 7, Some(6), Some(SELECTED))).await;
    committed(&writer, revision(2, 9, Some(7), None)).await;
    assert!(writer.purge_variable_history(execution(51)).await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM environment_variable_purges WHERE purge_id=?1",
            [id(50)],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "failed"
    );
    let mut replacement = intent();
    replacement.purge_id = id(52);
    replacement.timestamp_unix_ms = 52;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(replacement)).await;
    assert_eq!(
        writer
            .purge_variable_history(EnvironmentVariablePurgeExecution {
                purge_id: id(52),
                timestamp_unix_ms: 53
            })
            .await
            .unwrap()
            .state,
        EnvironmentVariablePurgeState::Committed
    );
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(reader
        .environment_revision_snapshot(&id(2), &id(7))
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

#[tokio::test]
async fn variable_purge_rewrite_keeps_failure_audit_after_successful_retry() {
    #[derive(serde::Deserialize)]
    struct Audit {
        state: EnvironmentVariablePurgeState,
        failure_reason: Option<String>,
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    crate::writer::fail_retention_for_path_for_tests(&path, crate::writer::RetentionFault::CandidateSync);
    assert!(writer.purge_variable_history(execution(51)).await.is_err());
    assert_eq!(
        writer.purge_variable_history(execution(52)).await.unwrap().state,
        EnvironmentVariablePurgeState::Committed
    );
    let conn = Connection::open(&path).unwrap();
    let mut statement = conn
        .prepare("SELECT detail FROM host_events WHERE kind='environment_variable_purge_changed' ORDER BY id")
        .unwrap();
    let audits = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| rmp_serde::from_slice::<Audit>(&row.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert!(audits
        .iter()
        .any(|audit| audit.state == EnvironmentVariablePurgeState::Prepared && audit.failure_reason.is_some()));
    assert_eq!(audits.last().unwrap().state, EnvironmentVariablePurgeState::Committed);
    assert!(audits.last().unwrap().failure_reason.is_none());
}
