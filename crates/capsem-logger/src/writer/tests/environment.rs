use super::*;
use crate::events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision};

fn prepared() -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: "1f7c44b0-fd7a-4c6f-8388-d98bec21a12e".into(),
        revision_id: "e45b8a3d-fae7-4a11-89ed-9431ae799c21".into(),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 42,
        actor: "operator".into(),
        reason: "create environment".into(),
        request_id: None,
        payload_event_id: "abcdef012345".into(),
        specification: Box::new(
            toml::from_str("name='development'\n[variables]\nPRIVATE='approved-snapshot-canary'\n").unwrap(),
        ),
    }
}

#[tokio::test]
async fn environment_revision_prepare_snapshot_and_transition_survive_flush_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let record = prepared();
    let writer = DbWriter::open(&path, 8).unwrap();
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(record.clone()))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state,actor,timestamp_unix_ms FROM environment_revisions WHERE revision_id=?1",
            [&record.revision_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        ("committing".into(), "operator".into(), 42)
    );
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let body = reader
        .read_body(
            &record.payload_event_id,
            "environment_revisions",
            crate::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .unwrap();
    let stored: capsem_config::environment::EnvironmentSpecification = rmp_serde::from_slice(&body.bytes).unwrap();
    assert_eq!(stored, *record.specification);
    writer
        .write_checked(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: record.environment_id.clone(),
            revision_id: record.revision_id.clone(),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 43,
            actor: "operator".into(),
        }))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    writer.shutdown_blocking();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM environment_revisions WHERE revision_id=?1",
            [&record.revision_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "committed"
    );
    assert!(!format!("{record:?}").contains("approved-snapshot-canary"));
}

fn transition(record: &PreparedEnvironmentRevision, state: EnvironmentRevisionState) -> WriteOp {
    WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: record.environment_id.clone(),
        revision_id: record.revision_id.clone(),
        state,
        timestamp_unix_ms: 43,
        actor: "operator".into(),
    })
}

async fn durable(writer: &DbWriter, operation: WriteOp) {
    writer.write_checked(operation).await.unwrap();
    writer.flush_checked().await.unwrap();
}

#[tokio::test]
async fn environment_revision_retries_preserve_snapshot_timestamp_and_audit_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let record = prepared();
    durable(&writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(&writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(&writer, transition(&record, EnvironmentRevisionState::Committed)).await;
    durable(&writer, transition(&record, EnvironmentRevisionState::Committed)).await;
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*),MIN(timestamp_unix_ms) FROM environment_revisions",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        )
        .unwrap(),
        (1, 42)
    );
    let kinds: Vec<String> = conn
        .prepare("SELECT kind FROM host_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        kinds,
        ["environment_revision_prepared", "environment_revision_committed"]
    );
    let mut changed = record.clone();
    changed
        .specification
        .variables
        .insert("PRIVATE".into(), "must-not-replace-original".into());
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(changed))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let snapshot = reader
        .read_body(
            &record.payload_event_id,
            "environment_revisions",
            crate::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .unwrap();
    let stored: capsem_config::environment::EnvironmentSpecification = rmp_serde::from_slice(&snapshot.bytes).unwrap();
    assert_eq!(stored, *record.specification);
}

#[tokio::test]
async fn environment_revision_recovery_retries_and_terminal_commits_are_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let record = prepared();
    durable(&writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    for state in [
        EnvironmentRevisionState::RecoveredUnapplied,
        EnvironmentRevisionState::RecoveredUnapplied,
        EnvironmentRevisionState::Committing,
        EnvironmentRevisionState::Committed,
    ] {
        durable(&writer, transition(&record, state)).await;
    }
    writer
        .write_checked(transition(&record, EnvironmentRevisionState::RecoveredUnapplied))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM environment_revisions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "committed"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM host_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[tokio::test]
async fn environment_revision_cannot_commit_before_snapshot_durability() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let record = prepared();
    writer
        .write_batch(vec![
            WriteOp::PrepareEnvironmentRevision(record.clone()),
            transition(&record, EnvironmentRevisionState::Committed),
        ])
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM environment_revisions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "committing"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_revision_committed'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

fn next_revision(record: &PreparedEnvironmentRevision) -> PreparedEnvironmentRevision {
    let mut next = record.clone();
    next.revision_id = "c756eb83-8cb6-418c-958c-e07ad118ea11".into();
    next.previous_revision_id = Some(record.revision_id.clone());
    next.payload_event_id = "abcdef012346".into();
    next.timestamp_unix_ms += 1;
    next
}

#[tokio::test]
async fn environment_revision_inflight_checks_cover_memory_and_disk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let first = prepared();
    let next = next_revision(&first);
    writer
        .write_batch(vec![
            WriteOp::PrepareEnvironmentRevision(first.clone()),
            WriteOp::PrepareEnvironmentRevision(next.clone()),
        ])
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(next.clone()))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    durable(&writer, transition(&first, EnvironmentRevisionState::Committed)).await;
    durable(&writer, WriteOp::PrepareEnvironmentRevision(next)).await;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn environment_revision_rejects_invalid_metadata_and_missing_predecessors() {
    for case in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.db");
        let writer = DbWriter::open(&path, 8).unwrap();
        let mut record = prepared();
        match case {
            0 => record.previous_revision_id = Some("10330164-dfac-4abf-8344-9c2a11e5cf8c".into()),
            1 => {
                record.fork_parent = Some(crate::events::EnvironmentRevisionParent {
                    environment_id: "d7427f8e-46d1-49d0-919a-1468dec5f06a".into(),
                    revision_id: "10330164-dfac-4abf-8344-9c2a11e5cf8c".into(),
                })
            }
            2 => record.actor = "unverified-body-actor".into(),
            3 => record.revision_id = record.revision_id.to_uppercase(),
            _ => record.specification.name = "bad/path".into(),
        }
        writer
            .write_checked(WriteOp::PrepareEnvironmentRevision(record))
            .await
            .unwrap();
        assert!(writer.flush_checked().await.is_err(), "invalid case {case} accepted");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn environment_fork_lineage_is_verified_and_cannot_be_rewritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let parent = prepared();
    durable(&writer, WriteOp::PrepareEnvironmentRevision(parent.clone())).await;
    durable(&writer, transition(&parent, EnvironmentRevisionState::Committed)).await;
    let mut fork = next_revision(&parent);
    fork.environment_id = "d7427f8e-46d1-49d0-919a-1468dec5f06a".into();
    fork.previous_revision_id = None;
    fork.fork_parent = Some(crate::events::EnvironmentRevisionParent {
        environment_id: parent.environment_id.clone(),
        revision_id: parent.revision_id.clone(),
    });
    durable(&writer, WriteOp::PrepareEnvironmentRevision(fork.clone())).await;
    durable(&writer, transition(&fork, EnvironmentRevisionState::Committed)).await;
    let mut forged = next_revision(&fork);
    forged.revision_id = "8d69346b-82f2-41eb-b6da-aeac1f8250e2".into();
    forged.payload_event_id = "abcdef012347".into();
    forged.fork_parent = None;
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(forged))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn purged_history_allows_current_configuration_removal_but_forbids_fork_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let first = prepared();
    durable(&writer, WriteOp::PrepareEnvironmentRevision(first.clone())).await;
    durable(&writer, transition(&first, EnvironmentRevisionState::Committed)).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE environment_revisions SET replayable=0 WHERE revision_id=?1",
        [&first.revision_id],
    )
    .unwrap();
    let mut removal = next_revision(&first);
    removal.specification.variables.remove("PRIVATE");
    durable(&writer, WriteOp::PrepareEnvironmentRevision(removal)).await;
    let mut fork = next_revision(&first);
    fork.environment_id = "d7427f8e-46d1-49d0-919a-1468dec5f06a".into();
    fork.revision_id = "8d69346b-82f2-41eb-b6da-aeac1f8250e2".into();
    fork.payload_event_id = "abcdef012347".into();
    fork.previous_revision_id = None;
    fork.fork_parent = Some(crate::events::EnvironmentRevisionParent {
        environment_id: first.environment_id.clone(),
        revision_id: first.revision_id.clone(),
    });
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(fork))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn durable_revision_audit_never_overwrites_an_earlier_staged_host_event() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let first = prepared();
    durable(&writer, WriteOp::PrepareEnvironmentRevision(first.clone())).await;
    let mut other = next_revision(&first);
    other.environment_id = "d7427f8e-46d1-49d0-919a-1468dec5f06a".into();
    other.previous_revision_id = None;
    writer
        .write_batch(vec![
            WriteOp::PrepareEnvironmentRevision(other),
            transition(&first, EnvironmentRevisionState::Committed),
        ])
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    let conn = Connection::open(&path).unwrap();
    let kinds: Vec<String> = conn
        .prepare("SELECT kind FROM host_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        kinds,
        [
            "environment_revision_prepared",
            "environment_revision_prepared",
            "environment_revision_committed"
        ]
    );
}

#[tokio::test]
async fn environment_prepare_metadata_and_archive_index_publish_atomically_on_retry() {
    let _fault = DISK_FLUSH_FAULT_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let writer = DbWriter::open(&path, 8).unwrap();
    let record = prepared();
    fail_disk_flushes_for_path_for_tests(&path, usize::MAX);
    writer
        .write_checked(WriteOp::PrepareEnvironmentRevision(record.clone()))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    fail_disk_flushes_for_path_for_tests(&path, 0);
    writer.flush_checked().await.unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
