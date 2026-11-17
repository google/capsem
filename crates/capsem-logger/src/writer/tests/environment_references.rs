use super::*;
use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
use crate::events::{
    EnvironmentRevisionParent, EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision,
};

fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn revision(environment: u64, number: u64, previous: Option<u64>) -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(number),
        previous_revision_id: previous.map(id),
        fork_parent: None,
        timestamp_unix_ms: 10,
        actor: "operator".into(),
        reason: "reference test".into(),
        request_id: None,
        payload_event_id: format!("{number:012x}"),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    }
}
fn transition(record: &PreparedEnvironmentRevision, state: EnvironmentRevisionState) -> WriteOp {
    WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: record.environment_id.clone(),
        revision_id: record.revision_id.clone(),
        state,
        timestamp_unix_ms: 11,
        actor: "operator".into(),
    })
}
async fn durable(writer: &DbWriter, op: WriteOp) {
    writer.write_checked(op).await.unwrap();
    writer.flush_checked().await.unwrap();
}
async fn rejected(writer: &DbWriter, op: WriteOp) {
    writer.write_checked(op).await.unwrap();
    assert!(writer.flush_checked().await.is_err());
}
async fn committed(writer: &DbWriter, record: &PreparedEnvironmentRevision) {
    durable(writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(writer, transition(record, EnvironmentRevisionState::Committed)).await;
}
fn current(conn: &Connection, environment: u64) -> Option<String> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT revision_id FROM environment_revision_references WHERE owner_kind='current_environment' AND owner_id=?1 AND owner_generation=''",[id(environment)],|row|row.get(0)).optional().unwrap()
}
fn selected(environment: u64, revision: u64) -> EnvironmentRevisionParent {
    EnvironmentRevisionParent {
        environment_id: id(environment),
        revision_id: id(revision),
    }
}
fn reference(
    kind: EnvironmentSandboxReferenceKind,
    expected: Option<EnvironmentRevisionParent>,
    revision: Option<EnvironmentRevisionParent>,
) -> WriteOp {
    WriteOp::EnvironmentSandboxReference(EnvironmentSandboxReferenceMutation {
        session_id: id(4),
        generation: id(5),
        kind,
        expected,
        revision,
        timestamp_unix_ms: 20,
        actor: "operator".into(),
    })
}

#[tokio::test]
async fn canonical_reference_advances_only_with_committed_revision_and_rejects_stale_branches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let conn = Connection::open(&path).unwrap();
    let first = revision(2, 3, None);
    committed(&writer, &first).await;
    assert_eq!(current(&conn, 2), Some(id(3)));
    let next = revision(2, 6, Some(3));
    durable(&writer, WriteOp::PrepareEnvironmentRevision(next.clone())).await;
    assert_eq!(current(&conn, 2), Some(id(3)));
    durable(&writer, transition(&next, EnvironmentRevisionState::Committed)).await;
    assert_eq!(current(&conn, 2), Some(id(6)));
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(revision(2, 7, Some(3)))).await;
    durable(&writer, transition(&first, EnvironmentRevisionState::Committed)).await;
    assert_eq!(
        current(&conn, 2),
        Some(id(6)),
        "historical transition retry cannot rewind current"
    );
}

#[tokio::test]
async fn recovered_unapplied_revision_cannot_reprepare_after_current_advances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3, None)).await;
    let abandoned = revision(2, 6, Some(3));
    durable(&writer, WriteOp::PrepareEnvironmentRevision(abandoned.clone())).await;
    durable(
        &writer,
        transition(&abandoned, EnvironmentRevisionState::RecoveredUnapplied),
    )
    .await;
    committed(&writer, &revision(2, 7, Some(3))).await;
    rejected(&writer, transition(&abandoned, EnvironmentRevisionState::Committing)).await;
}

#[tokio::test]
async fn reference_failure_rolls_back_committed_state_and_its_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let conn = Connection::open(&path).unwrap();
    let first = revision(2, 3, None);
    durable(&writer, WriteOp::PrepareEnvironmentRevision(first.clone())).await;
    conn.execute_batch("CREATE TRIGGER fail_current_ref BEFORE INSERT ON environment_revision_references WHEN NEW.owner_kind='current_environment' BEGIN SELECT RAISE(FAIL,'injected reference publication failure'); END;").unwrap();
    rejected(&writer, transition(&first, EnvironmentRevisionState::Committed)).await;
    assert_eq!(
        conn.query_row("SELECT state FROM environment_revisions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "committing"
    );
    assert_eq!(current(&conn, 2), None);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_revision_committed'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER fail_current_ref").unwrap();
    durable(&writer, transition(&first, EnvironmentRevisionState::Committed)).await;
    assert_eq!(current(&conn, 2), Some(id(3)));
}

#[tokio::test]
async fn sandbox_boot_live_and_desired_references_are_independent_and_generation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3, None)).await;
    committed(&writer, &revision(6, 7, None)).await;
    for kind in [
        EnvironmentSandboxReferenceKind::Boot,
        EnvironmentSandboxReferenceKind::LivePolicy,
        EnvironmentSandboxReferenceKind::Desired,
    ] {
        durable(&writer, reference(kind, None, Some(selected(2, 3)))).await;
    }
    durable(
        &writer,
        reference(
            EnvironmentSandboxReferenceKind::LivePolicy,
            Some(selected(2, 3)),
            Some(selected(6, 7)),
        ),
    )
    .await;
    let mut other = reference(EnvironmentSandboxReferenceKind::Boot, None, Some(selected(6, 7)));
    let WriteOp::EnvironmentSandboxReference(record) = &mut other else {
        unreachable!()
    };
    record.generation = id(8);
    durable(&writer, other).await;
    let conn = Connection::open(&path).unwrap();
    for (kind, generation, expected) in [
        ("sandbox_boot", 5, 3),
        ("sandbox_desired", 5, 3),
        ("sandbox_live_policy", 5, 7),
        ("sandbox_boot", 8, 7),
    ] {
        assert_eq!(conn.query_row("SELECT revision_id FROM environment_revision_references WHERE owner_kind=?1 AND owner_id=?2 AND owner_generation=?3",rusqlite::params![kind,id(4),id(generation)],|row|row.get::<_,String>(0)).unwrap(),id(expected));
    }
    rejected(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, None, Some(selected(6, 7))),
    )
    .await;
    durable(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, Some(selected(2, 3)), None),
    )
    .await;
}

#[tokio::test]
async fn sandbox_reference_refuses_missing_uncommitted_and_purged_targets_but_allows_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3, None)).await;
    durable(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, None, Some(selected(2, 3))),
    )
    .await;
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE environment_revisions SET replayable=0 WHERE revision_id=?1",
        [id(3)],
    )
    .unwrap();
    rejected(
        &writer,
        reference(EnvironmentSandboxReferenceKind::LivePolicy, None, Some(selected(2, 3))),
    )
    .await;
    durable(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, Some(selected(2, 3)), None),
    )
    .await;
    durable(&writer, WriteOp::PrepareEnvironmentRevision(revision(6, 7, None))).await;
    rejected(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Desired, None, Some(selected(6, 7))),
    )
    .await;
    rejected(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Desired, None, Some(selected(8, 9))),
    )
    .await;
    let mut wrong_actor = reference(EnvironmentSandboxReferenceKind::Boot, None, None);
    let WriteOp::EnvironmentSandboxReference(record) = &mut wrong_actor else {
        unreachable!()
    };
    record.actor = format!("vm:{}/{}", id(4), id(8));
    rejected(&writer, wrong_actor).await;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_reference_changed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    durable(&writer, reference(EnvironmentSandboxReferenceKind::Boot, None, None)).await;
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_reference_changed'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        count,
        "no-op release must not fabricate another application"
    );
}

#[tokio::test]
async fn sandbox_reference_change_rolls_back_when_its_audit_cannot_publish() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3, None)).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_reference_audit BEFORE INSERT ON host_events WHEN NEW.kind='environment_reference_changed' BEGIN SELECT RAISE(FAIL,'injected reference audit failure'); END;").unwrap();
    rejected(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, None, Some(selected(2, 3))),
    )
    .await;
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM environment_revision_references WHERE owner_kind='sandbox_boot'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER fail_reference_audit").unwrap();
    durable(
        &writer,
        reference(EnvironmentSandboxReferenceKind::Boot, None, Some(selected(2, 3))),
    )
    .await;
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_reference_changed'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn missing_canonical_reference_is_not_permission_to_guess_current_from_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3, None)).await;
    Connection::open(&path)
        .unwrap()
        .execute(
            "DELETE FROM environment_revision_references WHERE owner_kind='current_environment'",
            [],
        )
        .unwrap();
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(revision(2, 6, Some(3)))).await;
}
