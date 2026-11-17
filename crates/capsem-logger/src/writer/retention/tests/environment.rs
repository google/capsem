//! Actual configuration bytes must survive ordinary forensic archive retention.
use crate::events::environment::request::{EnvironmentRequestPayload, PreparedEnvironmentRequest};
use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
use crate::events::{
    EnvironmentRevisionParent, EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision,
};
use crate::{DbHandle, WriteOp};
use rusqlite::Connection;

const CUTOFF: &str = "9999-12-31T23:59:59Z";
fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn revision(environment: u64, number: u64, previous: Option<u64>) -> PreparedEnvironmentRevision {
    let mut specification: capsem_config::environment::EnvironmentSpecification =
        toml::from_str("name='development'").unwrap();
    specification
        .variables
        .insert("PUBLIC".into(), format!("configuration-{number}"));
    PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(number),
        previous_revision_id: previous.map(id),
        fork_parent: None,
        timestamp_unix_ms: 10,
        actor: "operator".into(),
        reason: "retention test".into(),
        request_id: None,
        payload_event_id: format!("{number:012x}"),
        specification: Box::new(specification),
    }
}
async fn durable(db: &DbHandle, op: WriteOp) {
    db.write(op).await.unwrap();
    db.flush().await.unwrap();
}
async fn committed(db: &DbHandle, record: &PreparedEnvironmentRevision) {
    durable(db, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(
        db,
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: record.environment_id.clone(),
            revision_id: record.revision_id.clone(),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 11,
            actor: "operator".into(),
        }),
    )
    .await;
}
fn archive_environment(conn: &Connection, environment: u64) {
    // Fixture for an archived document's released current reference. Archive
    // publication is service-owned; this test exercises only ledger retention.
    conn.execute(
        "DELETE FROM environment_revision_references WHERE owner_kind='current_environment' AND owner_id=?1",
        [id(environment)],
    )
    .unwrap();
}
async fn exact(db: &DbHandle, record: &PreparedEnvironmentRevision) {
    let snapshot = db
        .environment_revision_snapshot(&record.environment_id, &record.revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.specification, record.specification);
}
fn open(path: &std::path::Path) -> DbHandle {
    crate::writer::close_blocks_at_every_flush_for_tests(path);
    DbHandle::open(path).unwrap()
}

#[tokio::test]
async fn current_reference_keeps_complete_predecessor_payloads_and_expires_only_unreferenced_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let first = revision(2, 3, None);
    let current = revision(2, 6, Some(3));
    let orphan = revision(9, 10, None);
    committed(&db, &first).await;
    committed(&db, &current).await;
    committed(&db, &orphan).await;
    archive_environment(&conn, 9);
    db.retain_bodies_since(CUTOFF).await.unwrap();
    exact(&db, &first).await;
    exact(&db, &current).await;
    let error = db
        .environment_revision_snapshot(&orphan.environment_id, &orphan.revision_id)
        .await
        .unwrap_err();
    assert!(
        error.contains("non-replayable"),
        "expired history must not become a broken/missing-body guess: {error}"
    );
    let mut statement = conn
        .prepare("SELECT revision_id,replayable FROM environment_revisions ORDER BY id")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows, [(id(3), true), (id(6), true), (id(10), false)]);
}

#[tokio::test]
async fn sandbox_dimensions_and_pending_promotion_source_protect_archived_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let base = revision(2, 3, None);
    committed(&db, &base).await;
    let mut fork = revision(6, 7, None);
    fork.fork_parent = Some(EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    });
    committed(&db, &fork).await;
    archive_environment(&conn, 6);
    let request = PreparedEnvironmentRequest {
        request_id: id(100),
        environment_id: id(2),
        base_revision_id: Some(id(3)),
        requester: None,
        timestamp_unix_ms: 20,
        expires_at_unix_ms: 10_000,
        actor: "operator".into(),
        reason: "promote retained fork".into(),
        payload_event_id: "abcdef012345".into(),
        payload: Box::new(EnvironmentRequestPayload::Promotion {
            source: EnvironmentRevisionParent {
                environment_id: id(6),
                revision_id: id(7),
            },
        }),
    };
    durable(&db, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
    for (kind, environment, number) in [
        (EnvironmentSandboxReferenceKind::Boot, 12, 13),
        (EnvironmentSandboxReferenceKind::LivePolicy, 14, 15),
        (EnvironmentSandboxReferenceKind::Desired, 16, 17),
    ] {
        let target = revision(environment, number, None);
        committed(&db, &target).await;
        archive_environment(&conn, environment);
        durable(
            &db,
            WriteOp::EnvironmentSandboxReference(EnvironmentSandboxReferenceMutation {
                session_id: id(4),
                generation: id(5),
                kind,
                expected: None,
                revision: Some(EnvironmentRevisionParent {
                    environment_id: id(environment),
                    revision_id: id(number),
                }),
                timestamp_unix_ms: 21,
                actor: "operator".into(),
            }),
        )
        .await;
    }
    db.retain_bodies_since(CUTOFF).await.unwrap();
    for record in [
        base,
        fork,
        revision(12, 13, None),
        revision(14, 15, None),
        revision(16, 17, None),
    ] {
        exact(&db, &record).await;
    }
    assert_eq!(
        *db.environment_request_snapshot(&request.request_id)
            .await
            .unwrap()
            .unwrap()
            .payload,
        *request.payload
    );
}

#[tokio::test]
async fn retained_fork_history_keeps_its_older_creation_and_parent_closure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let base = revision(2, 3, None);
    committed(&db, &base).await;
    let mut fork = revision(6, 7, None);
    fork.fork_parent = Some(EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    });
    committed(&db, &fork).await;
    archive_environment(&conn, 2);
    archive_environment(&conn, 6);
    conn.execute("UPDATE body_blocks SET sealed_at='2026-01-01T00:00:00Z'", [])
        .unwrap();
    conn.execute("UPDATE event_body_blobs SET created_at='2026-01-01T00:00:00Z'", [])
        .unwrap();
    conn.execute(
        "UPDATE event_body_blobs SET created_at='2026-06-01T00:00:00Z' WHERE event_id=?1",
        [&fork.payload_event_id],
    )
    .unwrap();
    conn.execute("UPDATE body_blocks SET sealed_at='2026-06-01T00:00:00Z' WHERE block_offset=(SELECT block_offset FROM event_body_blobs WHERE event_id=?1)",[&fork.payload_event_id]).unwrap();
    db.retain_bodies_since("2026-03-01T00:00:00Z").await.unwrap();
    exact(&db, &base).await;
    exact(&db, &fork).await;
}

#[tokio::test]
async fn expiration_marker_index_and_generation_roll_back_together_on_publication_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let retained = revision(2, 3, None);
    let expired = revision(9, 10, None);
    committed(&db, &retained).await;
    committed(&db, &expired).await;
    archive_environment(&conn, 9);
    let before = crate::schema::archive_state(&conn).unwrap();
    crate::writer::fail_retention_for_path_for_tests(&path, crate::writer::RetentionFault::IndexTransaction);
    assert!(db.retain_bodies_since(CUTOFF).await.is_err());
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), before);
    exact(&db, &retained).await;
    exact(&db, &expired).await;
    db.retain_bodies_since(CUTOFF).await.unwrap();
    exact(&db, &retained).await;
    assert!(db
        .environment_revision_snapshot(&expired.environment_id, &expired.revision_id)
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

#[tokio::test]
async fn expiration_publishes_even_when_all_physical_blocks_remain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let retained = revision(2, 3, None);
    let expired = revision(9, 10, None);
    committed(&db, &retained).await;
    committed(&db, &expired).await;
    archive_environment(&conn, 9);
    // A young block can contain old configuration alongside younger forensic
    // data. Physical block retention must not suppress logical expiration.
    conn.execute("UPDATE body_blocks SET sealed_at=?1", [CUTOFF]).unwrap();
    let before = crate::schema::archive_state(&conn).unwrap();
    let outcome = db.retain_bodies_since(CUTOFF).await.unwrap();
    assert_eq!(outcome.blocks_dropped, 0);
    assert_eq!(outcome.rows_dropped, 1);
    let after = crate::schema::archive_state(&conn).unwrap();
    assert_eq!(before.committed_end, after.committed_end);
    assert_ne!(before.header.generation_id, after.header.generation_id);
    exact(&db, &retained).await;
    assert!(db
        .environment_revision_snapshot(&expired.environment_id, &expired.revision_id)
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

#[tokio::test]
async fn missing_protected_payload_refuses_retention_without_reclassifying_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = open(&path);
    let conn = Connection::open(&path).unwrap();
    let retained = revision(2, 3, None);
    committed(&db, &retained).await;
    conn.execute(
        "DELETE FROM event_body_blobs WHERE event_id=?1",
        [&retained.payload_event_id],
    )
    .unwrap();
    let before = crate::schema::archive_state(&conn).unwrap();
    assert!(db
        .retain_bodies_since(CUTOFF)
        .await
        .unwrap_err()
        .contains("reconstruction payload is missing"));
    assert_eq!(crate::schema::archive_state(&conn).unwrap(), before);
    let replayable: bool = conn
        .query_row("SELECT replayable FROM environment_revisions", [], |row| row.get(0))
        .unwrap();
    assert!(replayable);
}
