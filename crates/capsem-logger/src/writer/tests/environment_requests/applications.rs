use super::*;
use crate::events::environment::application::*;
use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
use crate::events::EnvironmentRevisionParent;

fn reference(environment: u64, revision: u64) -> EnvironmentRevisionParent {
    EnvironmentRevisionParent {
        environment_id: id(environment),
        revision_id: id(revision),
    }
}
fn target() -> EnvironmentRequestTarget {
    EnvironmentRequestTarget {
        session_id: id(20),
        generation: id(21),
    }
}
fn initial_references() -> EnvironmentSandboxReferences {
    EnvironmentSandboxReferences {
        boot: Some(reference(2, 3)),
        live_policy: Some(reference(2, 3)),
        desired: Some(reference(2, 3)),
    }
}
fn pending() -> EnvironmentApplicationMutation {
    EnvironmentApplicationMutation {
        request_id: id(1),
        target: target(),
        expected_version: None,
        expected_references: initial_references(),
        timestamp_unix_ms: 33,
        actor: "operator".into(),
        state: EnvironmentApplicationState::Pending,
        composition: EnvironmentApplicationComposition {
            boot: reference(2, 3),
            live_policy: reference(2, 3),
            desired: reference(4, 5),
            corporate_policy_id: "corporate-policy-v1".into(),
            pending_restart: true,
        },
        failure_reason: None,
    }
}
async fn owner(path: &std::path::Path) -> DbWriter {
    let writer = DbWriter::open(path, 8).unwrap();
    for (environment, number) in [(2, 3), (4, 5)] {
        let record = revision(environment, number);
        durable(&writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
        durable(&writer, commit(&record)).await;
    }
    for kind in [
        EnvironmentSandboxReferenceKind::Boot,
        EnvironmentSandboxReferenceKind::LivePolicy,
        EnvironmentSandboxReferenceKind::Desired,
    ] {
        durable(
            &writer,
            WriteOp::EnvironmentSandboxReference(EnvironmentSandboxReferenceMutation {
                session_id: id(20),
                generation: id(21),
                kind,
                expected: None,
                revision: Some(reference(2, 3)),
                timestamp_unix_ms: 25,
                actor: "operator".into(),
            }),
        )
        .await;
    }
    let mut request = proposal();
    request.environment_id = id(4);
    request.base_revision_id = Some(id(5));
    request.timestamp_unix_ms = 30;
    request.payload = Box::new(EnvironmentRequestPayload::Apply {
        revision: reference(4, 5),
        targets: vec![target()],
    });
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    let mut approval = decision(EnvironmentRequestDecisionKind::Approve);
    approval.timestamp_unix_ms = 31;
    durable(&writer, WriteOp::EnvironmentRequestDecision(approval)).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: id(1),
            result: reference(4, 5),
            timestamp_unix_ms: 32,
            actor: "operator".into(),
        }),
    )
    .await;
    writer
}

#[tokio::test]
async fn application_keeps_independent_composition_and_completed_retry_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    let pending = pending();
    durable(&writer, WriteOp::EnvironmentApplication(pending.clone())).await;
    let mut applying = pending.clone();
    applying.expected_version = Some(1);
    applying.expected_references.desired = Some(reference(4, 5));
    applying.timestamp_unix_ms = 34;
    applying.state = EnvironmentApplicationState::Applying;
    durable(&writer, WriteOp::EnvironmentApplication(applying.clone())).await;
    let mut applied = applying;
    applied.expected_version = Some(2);
    applied.timestamp_unix_ms = 35;
    applied.state = EnvironmentApplicationState::Applied;
    applied.composition.live_policy = reference(4, 5);
    durable(&writer, WriteOp::EnvironmentApplication(applied.clone())).await;
    writer.shutdown_blocking();
    let writer = DbWriter::open(&path, 8).unwrap();
    durable(&writer, WriteOp::EnvironmentApplication(applied.clone())).await;
    rejected(&writer, WriteOp::EnvironmentApplication(pending)).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let rows = reader
        .environment_applications(crate::db::EnvironmentApplicationQuery::Get {
            request_id: id(1),
            target: target(),
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version, 3);
    assert_eq!(rows[0].composition, applied.composition);
    assert_eq!(rows[0].state, EnvironmentApplicationState::Applied);
    let refs = reader
        .environment_references(crate::db::EnvironmentReferenceQuery::Sandbox {
            session_id: id(20),
            generation: id(21),
        })
        .await
        .unwrap();
    assert!(refs.contains(&crate::db::EnvironmentReferenceRecord::Sandbox {
        dimension: EnvironmentSandboxReferenceKind::Boot,
        revision: reference(2, 3),
    }));
    assert!(refs.contains(&crate::db::EnvironmentReferenceRecord::Sandbox {
        dimension: EnvironmentSandboxReferenceKind::LivePolicy,
        revision: reference(4, 5),
    }));
}

#[tokio::test]
async fn application_rejects_wrong_target_stale_reference_cas_and_failed_ack_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    let mut wrong = pending();
    wrong.target.generation = id(22);
    rejected(&writer, WriteOp::EnvironmentApplication(wrong)).await;
    let mut wrong = pending();
    wrong.expected_references.live_policy = Some(reference(4, 5));
    rejected(&writer, WriteOp::EnvironmentApplication(wrong)).await;
    durable(&writer, WriteOp::EnvironmentApplication(pending())).await;
    let mut failure = pending();
    failure.expected_version = Some(1);
    failure.expected_references.desired = Some(reference(4, 5));
    failure.timestamp_unix_ms = 34;
    failure.state = EnvironmentApplicationState::Failed;
    failure.failure_reason = Some("publication failure canary".into());
    let mut wrong = failure.clone();
    wrong.composition.live_policy = reference(4, 5);
    rejected(&writer, WriteOp::EnvironmentApplication(wrong)).await;
    durable(&writer, WriteOp::EnvironmentApplication(failure.clone())).await;
    assert!(!format!("{failure:?}").contains("publication failure canary"));
    let mut retry = failure;
    retry.expected_version = Some(2);
    retry.timestamp_unix_ms = 35;
    retry.state = EnvironmentApplicationState::Applying;
    retry.failure_reason = None;
    durable(&writer, WriteOp::EnvironmentApplication(retry)).await;
    #[derive(serde::Deserialize)]
    struct SavedFailure {
        failure_reason: Option<String>,
    }
    let conn = Connection::open(&path).unwrap();
    let mut statement = conn
        .prepare("SELECT detail FROM host_events WHERE kind='environment_application_changed' ORDER BY id")
        .unwrap();
    let details = statement.query_map([], |row| row.get::<_, Vec<u8>>(0)).unwrap();
    assert!(
        details
            .map(|row| rmp_serde::from_slice::<SavedFailure>(&row.unwrap()).unwrap())
            .any(|event| event.failure_reason.as_deref() == Some("publication failure canary")),
        "retry must retain the original failed application reason in its durable audit"
    );
}

#[tokio::test]
async fn application_and_all_reference_dimensions_roll_back_with_the_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_application BEFORE INSERT ON host_events
        WHEN NEW.kind='environment_application_changed' BEGIN SELECT RAISE(ABORT,'injected application audit failure'); END").unwrap();
    rejected(&writer, WriteOp::EnvironmentApplication(pending())).await;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_applications", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT revision_id FROM environment_revision_references WHERE owner_kind='sandbox_desired'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        id(3)
    );
    conn.execute_batch("DROP TRIGGER reject_application").unwrap();
    durable(&writer, WriteOp::EnvironmentApplication(pending())).await;
}

#[tokio::test]
async fn unfinished_and_young_application_history_preserve_full_archive_closure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    crate::writer::close_blocks_at_every_flush_for_tests(&path);
    let writer = owner(&path).await;
    durable(&writer, WriteOp::EnvironmentApplication(pending())).await;
    let conn = Connection::open(&path).unwrap();
    // Model archival/release after the recorded application. Only application
    // reachability, rather than canonical/sandbox pointers, protects these rows.
    conn.execute("DELETE FROM environment_revision_references", []).unwrap();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    for young in [false, true] {
        if young {
            conn.execute(
                "UPDATE environment_applications SET state='applied',timestamp_unix_ms=253402300799000",
                [],
            )
            .unwrap();
        }
        writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
        assert!(reader
            .environment_revision_snapshot(&id(2), &id(3))
            .await
            .unwrap()
            .is_some());
        assert!(reader
            .environment_revision_snapshot(&id(4), &id(5))
            .await
            .unwrap()
            .is_some());
        assert!(reader.environment_request_snapshot(&id(1)).await.unwrap().is_some());
    }
    conn.execute("UPDATE environment_applications SET timestamp_unix_ms=33", [])
        .unwrap();
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    assert!(reader
        .environment_revision_snapshot(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("non-replayable"));
    assert!(reader
        .environment_request_snapshot(&id(1))
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

#[tokio::test]
async fn application_queries_bound_pages_and_fail_on_invalid_identity_or_schema() {
    use crate::db::EnvironmentApplicationQuery;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    durable(&writer, WriteOp::EnvironmentApplication(pending())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let page = |generation, after_id, limit| EnvironmentApplicationQuery::Page {
        request_id: Some(id(1)),
        target: Some(EnvironmentRequestTarget {
            session_id: id(20),
            generation: id(generation),
        }),
        after_id,
        limit,
    };
    let first = reader.environment_applications(page(21, 0, 1)).await.unwrap();
    assert_eq!(first.len(), 1);
    assert!(reader
        .environment_applications(page(21, first[0].cursor, 1))
        .await
        .unwrap()
        .is_empty());
    assert!(reader
        .environment_applications(page(22, 0, 1))
        .await
        .unwrap()
        .is_empty());
    for query in [
        page(21, 0, 0),
        page(21, -1, 1),
        EnvironmentApplicationQuery::Get {
            request_id: "bad".into(),
            target: target(),
        },
    ] {
        let count = reader.reader_requests();
        assert!(reader.environment_applications(query).await.is_err());
        assert_eq!(reader.reader_requests(), count);
    }
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("ALTER TABLE environment_applications RENAME TO missing_applications")
        .unwrap();
    assert!(reader
        .environment_applications(page(21, 0, 1))
        .await
        .unwrap_err()
        .contains("no such table"));
}

#[tokio::test]
async fn application_reads_refuse_corrupt_state_composition_and_version() {
    for corruption in [
        "UPDATE environment_applications SET version=0",
        "UPDATE environment_applications SET pending_restart=2",
        "UPDATE environment_applications SET state='failed'",
        "UPDATE environment_applications SET boot_revision_id='bad'",
        "UPDATE environment_applications SET corporate_policy_id=''",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let writer = owner(&path).await;
        durable(&writer, WriteOp::EnvironmentApplication(pending())).await;
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute_batch(corruption).unwrap();
        let reader = crate::DbHandle::open_external_reader(&path).unwrap();
        assert!(
            reader
                .environment_applications(crate::db::EnvironmentApplicationQuery::Get {
                    request_id: id(1),
                    target: target(),
                })
                .await
                .is_err(),
            "must refuse damaged application rows: {corruption}"
        );
    }
}

#[tokio::test]
async fn approved_agent_fork_application_binds_only_its_exact_requester() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    let mut request = proposal();
    request.request_id = id(40);
    request.environment_id = id(2);
    request.base_revision_id = Some(id(3));
    request.requester = Some(target());
    request.timestamp_unix_ms = 40;
    request.actor = format!("agent:{}", id(10));
    request.payload_event_id = "000000000028".into();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    let mut approval = decision(EnvironmentRequestDecisionKind::Approve);
    approval.request_id = id(40);
    approval.timestamp_unix_ms = 41;
    durable(&writer, WriteOp::EnvironmentRequestDecision(approval)).await;
    let mut fork = requested_revision(6, 7);
    fork.fork_parent = Some(reference(2, 3));
    fork.request_id = Some(id(40));
    fork.timestamp_unix_ms = 42;
    durable(&writer, WriteOp::PrepareEnvironmentRevision(fork.clone())).await;
    let mut transition = commit(&fork);
    if let WriteOp::EnvironmentRevisionTransition(value) = &mut transition {
        value.timestamp_unix_ms = 43;
    }
    durable(&writer, transition).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: id(40),
            result: reference(6, 7),
            timestamp_unix_ms: 44,
            actor: "operator".into(),
        }),
    )
    .await;
    let mut change = pending();
    change.request_id = id(40);
    change.timestamp_unix_ms = 45;
    change.composition.desired = reference(6, 7);
    let mut other = change.clone();
    other.target.generation = id(22);
    rejected(&writer, WriteOp::EnvironmentApplication(other)).await;
    let mut other = change.clone();
    other.target.session_id = id(23);
    rejected(&writer, WriteOp::EnvironmentApplication(other)).await;
    durable(&writer, WriteOp::EnvironmentApplication(change.clone())).await;
    change.expected_references.desired = Some(reference(6, 7));
    change.expected_version = Some(1);
    change.timestamp_unix_ms = 46;
    change.state = EnvironmentApplicationState::Applying;
    durable(&writer, WriteOp::EnvironmentApplication(change.clone())).await;
    change.expected_version = Some(2);
    change.timestamp_unix_ms = 47;
    change.state = EnvironmentApplicationState::Applied;
    change.composition.live_policy = reference(6, 7);
    durable(&writer, WriteOp::EnvironmentApplication(change)).await;

    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let boot = EnvironmentApplicationBoot {
        target: EnvironmentRequestTarget {
            session_id: id(20),
            generation: id(22),
        },
        revision: reference(6, 7),
        timestamp_unix_ms: 50,
    };
    let shared = EnvironmentApplicationBoot {
        revision: reference(2, 3),
        ..boot.clone()
    };
    let sibling = EnvironmentApplicationBoot {
        target: EnvironmentRequestTarget {
            session_id: id(23),
            generation: id(24),
        },
        ..boot.clone()
    };
    let original = EnvironmentApplicationBoot {
        target: target(),
        ..boot.clone()
    };
    for candidate in [shared, sibling, original] {
        durable(&writer, WriteOp::EnvironmentApplicationBoot(candidate)).await;
        assert!(
            reader
                .environment_request_history(&id(40))
                .await
                .unwrap()
                .unwrap()
                .record
                .application
                .unwrap()
                .pending_restart
        );
    }
    rejected(&writer, WriteOp::EnvironmentApplicationBoot(boot.clone())).await;
    durable(
        &writer,
        WriteOp::EnvironmentSandboxReference(EnvironmentSandboxReferenceMutation {
            session_id: boot.target.session_id.clone(),
            generation: boot.target.generation.clone(),
            kind: EnvironmentSandboxReferenceKind::Boot,
            expected: None,
            revision: Some(boot.revision.clone()),
            timestamp_unix_ms: 49,
            actor: "system".into(),
        }),
    )
    .await;
    durable(&writer, WriteOp::EnvironmentApplicationBoot(boot.clone())).await;
    let replay = EnvironmentApplicationBoot {
        timestamp_unix_ms: 51,
        ..boot
    };
    durable(&writer, WriteOp::EnvironmentApplicationBoot(replay)).await;
    assert!(
        !reader
            .environment_request_history(&id(40))
            .await
            .unwrap()
            .unwrap()
            .record
            .application
            .unwrap()
            .pending_restart
    );
    let saved = reader
        .environment_applications(crate::db::EnvironmentApplicationQuery::Get {
            request_id: id(40),
            target: target(),
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(saved.version, 4);
    assert_eq!(saved.composition.boot, reference(2, 3));
    assert_eq!(saved.target, target());
    assert_eq!(
        reader
            .environment_references(crate::db::EnvironmentReferenceQuery::CurrentEnvironment { environment_id: id(2) })
            .await
            .unwrap(),
        vec![crate::db::EnvironmentReferenceRecord::CurrentEnvironment {
            revision: reference(2, 3)
        }]
    );
    drop(reader);
    writer.shutdown_blocking();
    let reopened = crate::DbHandle::open_external_reader(&path).unwrap();
    assert!(
        !reopened
            .environment_request_history(&id(40))
            .await
            .unwrap()
            .unwrap()
            .record
            .application
            .unwrap()
            .pending_restart
    );
}

#[tokio::test]
async fn application_audit_preserves_maximum_bounded_metadata_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path).await;
    let mut initial = pending();
    initial.composition.corporate_policy_id = "p".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES);
    durable(&writer, WriteOp::EnvironmentApplication(initial.clone())).await;
    let mut failure = initial;
    failure.expected_version = Some(1);
    failure.expected_references.desired = Some(reference(4, 5));
    failure.timestamp_unix_ms = 34;
    failure.state = EnvironmentApplicationState::Failed;
    failure.failure_reason = Some("x".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES));
    durable(&writer, WriteOp::EnvironmentApplication(failure.clone())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let record = reader
        .environment_applications(crate::db::EnvironmentApplicationQuery::Get {
            request_id: id(1),
            target: target(),
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(record.failure_reason, failure.failure_reason);
    assert_eq!(
        record.composition.corporate_policy_id,
        failure.composition.corporate_policy_id
    );
}
