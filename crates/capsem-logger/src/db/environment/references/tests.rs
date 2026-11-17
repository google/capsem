use super::*;
use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
use crate::events::{EnvironmentRevisionTransition, PreparedEnvironmentRevision};

fn id(number: u64) -> String {
    format!("00000000-0000-4000-8000-{number:012x}")
}
async fn committed(db: &DbHandle, environment: u64, revision: u64) {
    db.write(WriteOp::PrepareEnvironmentRevision(PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(revision),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 10,
        actor: "operator".into(),
        reason: "reference query".into(),
        request_id: None,
        payload_event_id: format!("{revision:012x}"),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
    db.write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
        environment_id: id(environment),
        revision_id: id(revision),
        state: EnvironmentRevisionState::Committed,
        timestamp_unix_ms: 11,
        actor: "operator".into(),
    }))
    .await
    .unwrap();
    db.flush().await.unwrap();
}
fn sandbox(generation: u64) -> EnvironmentReferenceQuery {
    EnvironmentReferenceQuery::Sandbox {
        session_id: id(20),
        generation: id(generation),
    }
}

#[tokio::test]
async fn references_read_exact_current_and_independent_sandbox_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    let current = EnvironmentReferenceQuery::CurrentEnvironment { environment_id: id(2) };
    assert!(db.environment_references(current.clone()).await.unwrap().is_empty());
    for (env, rev) in [(2, 3), (4, 5), (6, 7)] {
        committed(&db, env, rev).await;
    }
    assert_eq!(
        db.environment_references(current).await.unwrap(),
        vec![EnvironmentReferenceRecord::CurrentEnvironment {
            revision: EnvironmentRevisionParent {
                environment_id: id(2),
                revision_id: id(3)
            }
        }]
    );
    for (kind, env, rev) in [
        (EnvironmentSandboxReferenceKind::Boot, 2, 3),
        (EnvironmentSandboxReferenceKind::LivePolicy, 4, 5),
        (EnvironmentSandboxReferenceKind::Desired, 6, 7),
    ] {
        db.write(WriteOp::EnvironmentSandboxReference(
            EnvironmentSandboxReferenceMutation {
                session_id: id(20),
                generation: id(21),
                kind,
                expected: None,
                revision: Some(EnvironmentRevisionParent {
                    environment_id: id(env),
                    revision_id: id(rev),
                }),
                timestamp_unix_ms: 12,
                actor: "operator".into(),
            },
        ))
        .await
        .unwrap();
        db.flush().await.unwrap();
    }
    let rows = db.environment_references(sandbox(21)).await.unwrap();
    assert_eq!(rows.len(), 3);
    for (kind, env, rev) in [
        (EnvironmentSandboxReferenceKind::Boot, 2, 3),
        (EnvironmentSandboxReferenceKind::LivePolicy, 4, 5),
        (EnvironmentSandboxReferenceKind::Desired, 6, 7),
    ] {
        assert!(rows.contains(&EnvironmentReferenceRecord::Sandbox {
            dimension: kind,
            revision: EnvironmentRevisionParent {
                environment_id: id(env),
                revision_id: id(rev)
            }
        }));
    }
    assert!(db.environment_references(sandbox(22)).await.unwrap().is_empty());
    let external = DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(external.environment_references(sandbox(21)).await.unwrap(), rows);
    db.write(WriteOp::EnvironmentSandboxReference(
        EnvironmentSandboxReferenceMutation {
            session_id: id(20),
            generation: id(21),
            kind: EnvironmentSandboxReferenceKind::Desired,
            expected: Some(EnvironmentRevisionParent {
                environment_id: id(6),
                revision_id: id(7),
            }),
            revision: None,
            timestamp_unix_ms: 13,
            actor: "operator".into(),
        },
    ))
    .await
    .unwrap();
    db.flush().await.unwrap();
    assert_eq!(external.environment_references(sandbox(21)).await.unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_reference_queries_and_missing_schema_fail_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    assert!(db
        .environment_references(EnvironmentReferenceQuery::CurrentEnvironment {
            environment_id: "../host".into()
        })
        .await
        .is_err());
    assert!(db
        .environment_references(EnvironmentReferenceQuery::Sandbox {
            session_id: id(20),
            generation: uuid::Uuid::nil().to_string()
        })
        .await
        .is_err());
    db.flush().await.unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE environment_revision_references")
        .unwrap();
    assert!(db
        .environment_references(sandbox(21))
        .await
        .unwrap_err()
        .contains("no such table"));
}

#[tokio::test]
async fn corrupt_reference_targets_and_owners_never_become_absent_or_current() {
    for corruption in [
        "UPDATE environment_revision_references SET owner_generation='unexpected' WHERE owner_id=?1",
        "UPDATE environment_revision_references SET revision_id='00000000-0000-4000-8000-000000000099' WHERE owner_id=?1",
        "UPDATE environment_revisions SET state='committing' WHERE environment_id=?1",
        "UPDATE environment_revision_references SET environment_id='bad' WHERE owner_id=?1",
        "UPDATE environment_revision_references SET environment_id='00000000-0000-4000-8000-000000000004', revision_id='00000000-0000-4000-8000-000000000005' WHERE owner_id=?1",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let db = DbHandle::open(&path).unwrap();
        committed(&db, 2, 3).await;
        committed(&db, 4, 5).await;
        let conn = rusqlite::Connection::open(&path).unwrap();
        // Deliberately model a damaged on-disk ledger, bypassing fixture-only FK enforcement.
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        conn.execute(corruption, [id(2)]).unwrap();
        assert!(db.environment_references(EnvironmentReferenceQuery::CurrentEnvironment {
            environment_id: id(2),
        }).await.is_err(), "corruption must be explicit: {corruption}");
    }
}
