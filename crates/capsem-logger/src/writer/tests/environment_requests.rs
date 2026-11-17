use super::*;
use crate::events::environment::request::*;

fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn proposal() -> PreparedEnvironmentRequest {
    PreparedEnvironmentRequest {
        request_id: id(1),
        environment_id: id(2),
        base_revision_id: None,
        requester: None,
        timestamp_unix_ms: 10,
        actor: "operator".into(),
        reason: "request-reason-canary".into(),
        payload_event_id: "abcdef012345".into(),
        payload: Box::new(EnvironmentRequestPayload::Change {
            specification: Box::new(
                toml::from_str("name='development'\n[variables]\nPRIVATE='proposal-archive-canary'").unwrap(),
            ),
        }),
    }
}
fn decision(kind: EnvironmentRequestDecisionKind) -> EnvironmentRequestDecision {
    EnvironmentRequestDecision {
        request_id: id(1),
        timestamp_unix_ms: 11,
        actor: "operator".into(),
        decision: kind,
        reason: "approve requested environment".into(),
    }
}

#[tokio::test]
async fn request_proposal_and_immutable_decision_survive_archive_flush_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let request = proposal();
    let writer = DbWriter::open(&path, 8).unwrap();
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(request.clone()))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    writer
        .write_checked(WriteOp::EnvironmentRequestDecision(decision(
            EnvironmentRequestDecisionKind::Approve,
        )))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    writer.shutdown_blocking();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let body = reader
        .read_body(
            &request.payload_event_id,
            "environment_requests",
            crate::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .unwrap();
    let payload: EnvironmentRequestPayload = rmp_serde::from_slice(&body.bytes).unwrap();
    assert_eq!(payload, *request.payload);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM environment_requests", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "approved"
    );
    assert_eq!(
        conn.query_row("SELECT actor FROM environment_request_decisions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "operator"
    );
    assert!(!format!("{request:?}").contains("proposal-archive-canary"));
    assert!(!format!("{request:?}").contains("request-reason-canary"));
    drop(reader);
    let writer = DbWriter::open(&path, 8).unwrap();
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(request))
        .await
        .unwrap();
    writer
        .write_checked(WriteOp::EnvironmentRequestDecision(decision(
            EnvironmentRequestDecisionKind::Approve,
        )))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_request_decisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

async fn rejected(writer: &DbWriter, op: WriteOp) {
    writer.write_checked(op).await.unwrap();
    assert!(writer.flush_checked().await.is_err());
}

#[tokio::test]
async fn request_decision_waits_for_durable_proposal_and_cannot_change_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("host.db"), 8).unwrap();
    rejected(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(proposal()))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    writer
        .write_checked(WriteOp::EnvironmentRequestDecision(decision(
            EnvironmentRequestDecisionKind::Deny,
        )))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    rejected(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
}

async fn durable(writer: &DbWriter, operation: WriteOp) {
    writer.write_checked(operation).await.unwrap();
    writer.flush_checked().await.unwrap();
}
fn revision(environment: u64, revision: u64) -> crate::events::PreparedEnvironmentRevision {
    crate::events::PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(revision),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 20,
        actor: "operator".into(),
        reason: "requested revision".into(),
        request_id: None,
        payload_event_id: format!("{revision:012x}"),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    }
}
fn commit(record: &crate::events::PreparedEnvironmentRevision) -> WriteOp {
    WriteOp::EnvironmentRevisionTransition(crate::events::EnvironmentRevisionTransition {
        environment_id: record.environment_id.clone(),
        revision_id: record.revision_id.clone(),
        state: crate::events::EnvironmentRevisionState::Committed,
        timestamp_unix_ms: 21,
        actor: "operator".into(),
    })
}
async fn committed(writer: &DbWriter, record: &crate::events::PreparedEnvironmentRevision) {
    durable(writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(writer, commit(record)).await;
}
fn result(environment: u64, revision: u64) -> EnvironmentRequestResult {
    EnvironmentRequestResult {
        request_id: id(1),
        result: crate::events::EnvironmentRevisionParent {
            environment_id: id(environment),
            revision_id: id(revision),
        },
        timestamp_unix_ms: 22,
        actor: "operator".into(),
    }
}

#[tokio::test]
async fn approved_request_binds_one_committed_result_and_retries_its_original_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let request = proposal();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
    let mut created = revision(2, 3);
    created.request_id = Some(request.request_id.clone());
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(created.clone())).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    durable(&writer, WriteOp::PrepareEnvironmentRevision(created.clone())).await;
    rejected(&writer, WriteOp::EnvironmentRequestResult(result(2, 3))).await;
    durable(&writer, commit(&created)).await;
    durable(&writer, WriteOp::EnvironmentRequestResult(result(2, 3))).await;
    durable(&writer, WriteOp::EnvironmentRequestResult(result(2, 3))).await;
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    let conn = Connection::open(&path).unwrap();
    let stored: (String, String) = conn
        .query_row(
            "SELECT result_environment_id,result_revision_id FROM environment_requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (id(2), id(3)));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_request_result_bound'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    rejected(&writer, WriteOp::EnvironmentRequestResult(result(2, 4))).await;
}

#[tokio::test]
async fn proposal_identity_and_bytes_never_change_on_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let request = proposal();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
    let mut changed = request.clone();
    changed.reason = "different reason".into();
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(changed)).await;
    let mut changed = request.clone();
    let EnvironmentRequestPayload::Change { specification } = changed.payload.as_mut() else {
        unreachable!()
    };
    specification
        .variables
        .insert("PRIVATE".into(), "different value".into());
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(changed)).await;
    let mut collision = request.clone();
    collision.request_id = id(99);
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(collision)).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let stored = reader
        .read_body(
            &request.payload_event_id,
            "environment_requests",
            crate::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rmp_serde::from_slice::<EnvironmentRequestPayload>(&stored.bytes).unwrap(),
        *request.payload
    );
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM environment_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn proposal_and_decision_cannot_share_an_unflushed_batch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    writer
        .write_batch(vec![
            WriteOp::PrepareEnvironmentRequest(proposal()),
            WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
        ])
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM environment_requests", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_request_decisions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn request_metadata_and_final_encoded_payload_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let mut cases = Vec::new();
    let mut request = proposal();
    request.actor = "forged".into();
    cases.push(request);
    let mut request = proposal();
    request.actor = format!("agent:{}", id(10));
    cases.push(request);
    let mut request = proposal();
    request.actor = format!("vm:{}/{}", id(4), id(5));
    request.requester = Some(EnvironmentRequestTarget {
        session_id: id(4),
        generation: id(6),
    });
    cases.push(request);
    let mut request = proposal();
    request.base_revision_id = Some(id(100));
    cases.push(request);
    let mut request = proposal();
    request.reason = "x".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES + 1);
    cases.push(request);
    let mut request = proposal();
    let EnvironmentRequestPayload::Change { specification } = request.payload.as_mut() else {
        unreachable!()
    };
    specification.image = Some(capsem_config::environment::EnvironmentImageSelection {
        reference: "development".into(),
        update_policy: Default::default(),
    });
    specification
        .repositories
        .push(capsem_config::environment::EnvironmentRepositorySelection {
            url: format!(
                "https://example.org/{}",
                "x".repeat(capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
            ),
            reference: "main".into(),
            destination: "project".into(),
        });
    specification.validate().unwrap(); // The final encoder owns the aggregate byte ceiling.
    cases.push(request);
    for request in cases {
        rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    }
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_requests", [], |row| row
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

#[tokio::test]
async fn agent_result_is_a_fork_of_its_exact_request_base() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("host.db"), 16).unwrap();
    let base = revision(2, 3);
    committed(&writer, &base).await;
    let mut request = proposal();
    request.base_revision_id = Some(base.revision_id.clone());
    request.actor = format!("agent:{}", id(10));
    request.requester = Some(EnvironmentRequestTarget {
        session_id: id(4),
        generation: id(5),
    });
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    rejected(&writer, WriteOp::EnvironmentRequestResult(result(2, 3))).await;
    let mut fork = revision(6, 7);
    fork.fork_parent = Some(crate::events::EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    });
    fork.request_id = Some(id(1));
    committed(&writer, &fork).await;
    durable(&writer, WriteOp::EnvironmentRequestResult(result(6, 7))).await;
}

#[tokio::test]
async fn image_promotion_and_apply_proposals_archive_their_exact_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let base = revision(2, 3);
    committed(&writer, &base).await;
    let mut fork = revision(6, 7);
    fork.fork_parent = Some(crate::events::EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    });
    committed(&writer, &fork).await;
    let selected = crate::events::EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    };
    let payloads = [
        EnvironmentRequestPayload::ImageUpdate {
            specification: Box::new(
                toml::from_str("name='development'\n[image]\nreference='development:stable'").unwrap(),
            ),
        },
        EnvironmentRequestPayload::Promotion {
            source: crate::events::EnvironmentRevisionParent {
                environment_id: id(6),
                revision_id: id(7),
            },
        },
        EnvironmentRequestPayload::Apply {
            revision: selected,
            targets: vec![EnvironmentRequestTarget {
                session_id: id(4),
                generation: id(5),
            }],
        },
    ];
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    for (index, payload) in payloads.into_iter().enumerate() {
        let mut request = proposal();
        request.request_id = id(100 + index as u64);
        request.payload_event_id = format!("{:012x}", 100 + index);
        request.base_revision_id = Some(id(3));
        *request.payload = payload;
        durable(&writer, WriteOp::PrepareEnvironmentRequest(request.clone())).await;
        let bytes = reader
            .read_body(
                &request.payload_event_id,
                "environment_requests",
                crate::BodyDirection::Payload,
            )
            .await
            .unwrap()
            .unwrap()
            .bytes;
        assert_eq!(
            rmp_serde::from_slice::<EnvironmentRequestPayload>(&bytes).unwrap(),
            *request.payload
        );
        let expected = match request.payload.as_ref() {
            EnvironmentRequestPayload::Promotion { source } => {
                (Some(source.environment_id.clone()), Some(source.revision_id.clone()))
            }
            EnvironmentRequestPayload::Apply { revision, .. } => (
                Some(revision.environment_id.clone()),
                Some(revision.revision_id.clone()),
            ),
            _ => (None, None),
        };
        let stored: (Option<String>, Option<String>) = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT source_environment_id,source_revision_id FROM environment_requests WHERE request_id=?1",
                [&request.request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            stored, expected,
            "retention roots must be indexed with the proposal rather than decoded later"
        );
        let mut approve = decision(EnvironmentRequestDecisionKind::Approve);
        approve.request_id = request.request_id.clone();
        durable(&writer, WriteOp::EnvironmentRequestDecision(approve)).await;
        if matches!(request.payload.as_ref(), EnvironmentRequestPayload::Apply { .. }) {
            let mut applied = result(2, 3);
            applied.request_id = request.request_id;
            durable(&writer, WriteOp::EnvironmentRequestResult(applied)).await;
        }
    }
}

#[tokio::test]
async fn purged_request_can_be_denied_but_cannot_be_approved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(proposal())).await;
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE environment_requests SET replayable=0", [])
        .unwrap();
    rejected(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Deny)),
    )
    .await;
}

#[tokio::test]
async fn failed_request_flush_publishes_neither_proposal_nor_body_index_until_retry() {
    let _fault = DISK_FLUSH_FAULT_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    fail_disk_flushes_for_path_for_tests(&path, usize::MAX);
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(proposal()))
        .await
        .unwrap();
    assert!(writer.flush_checked().await.is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_requests", [], |row| row
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
        conn.query_row("SELECT COUNT(*) FROM environment_requests", [], |row| row
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

#[tokio::test]
async fn pending_environment_requests_are_bounded_across_staged_and_durable_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, capsem_proto::APPROVAL_PENDING_TOTAL + 1).unwrap();
    let requests = (0..capsem_proto::APPROVAL_PENDING_TOTAL)
        .map(|index| {
            let mut request = proposal();
            request.request_id = id(100 + index as u64);
            request.environment_id = id(1000 + index as u64);
            request.payload_event_id = format!("{:012x}", 100 + index);
            WriteOp::PrepareEnvironmentRequest(request)
        })
        .collect();
    writer.write_batch(requests).await.unwrap();
    writer.flush_checked().await.unwrap();
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(proposal())).await;
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM environment_requests WHERE state='pending'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        capsem_proto::APPROVAL_PENDING_TOTAL as i64
    );
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM environment_requests WHERE state='pending'",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(
        plan.contains("idx_environment_requests_pending"),
        "pending admission must scan only bounded pending rows, not all request history: {plan}"
    );
    let mut deny = decision(EnvironmentRequestDecisionKind::Deny);
    deny.request_id = id(100);
    durable(&writer, WriteOp::EnvironmentRequestDecision(deny)).await;
    durable(&writer, WriteOp::PrepareEnvironmentRequest(proposal())).await;
}

#[tokio::test]
async fn requester_pending_capacity_does_not_reset_with_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, capsem_proto::APPROVAL_PENDING_PER_SESSION + 1).unwrap();
    committed(&writer, &revision(2, 3)).await;
    let mut request = proposal();
    request.base_revision_id = Some(id(3));
    request.actor = format!("agent:{}", id(10));
    request.requester = Some(EnvironmentRequestTarget {
        session_id: id(4),
        generation: id(5),
    });
    let requests = (0..capsem_proto::APPROVAL_PENDING_PER_SESSION)
        .map(|index| {
            let mut request = request.clone();
            request.request_id = id(100 + index as u64);
            request.payload_event_id = format!("{:012x}", 100 + index);
            WriteOp::PrepareEnvironmentRequest(request)
        })
        .collect();
    writer.write_batch(requests).await.unwrap();
    writer.flush_checked().await.unwrap();
    request.requester.as_mut().unwrap().generation = id(6);
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
}

#[tokio::test]
async fn promotion_lineage_and_apply_target_budgets_fail_before_archiving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    committed(&writer, &revision(2, 3)).await;
    let selected = crate::events::EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(3),
    };
    let payloads = [
        EnvironmentRequestPayload::Promotion {
            source: selected.clone(),
        },
        EnvironmentRequestPayload::Apply {
            revision: selected.clone(),
            targets: Vec::new(),
        },
        EnvironmentRequestPayload::Apply {
            revision: selected.clone(),
            targets: vec![
                EnvironmentRequestTarget {
                    session_id: id(4),
                    generation: id(5),
                },
                EnvironmentRequestTarget {
                    session_id: id(4),
                    generation: id(6),
                },
            ],
        },
        EnvironmentRequestPayload::Apply {
            revision: selected,
            targets: (0..=capsem_proto::APPROVAL_PENDING_TOTAL)
                .map(|index| EnvironmentRequestTarget {
                    session_id: id(1000 + index as u64),
                    generation: id(5),
                })
                .collect(),
        },
    ];
    for payload in payloads {
        let mut request = proposal();
        request.base_revision_id = Some(id(3));
        *request.payload = payload;
        rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    }
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM event_body_blobs WHERE source_table='environment_requests'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn approved_request_cannot_prepare_another_environment_or_a_second_revision() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("host.db"), 16).unwrap();
    durable(&writer, WriteOp::PrepareEnvironmentRequest(proposal())).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    let mut wrong = revision(99, 100);
    wrong.request_id = Some(id(1));
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(wrong)).await;
    let mut correct = revision(2, 3);
    correct.request_id = Some(id(1));
    committed(&writer, &correct).await;
    let mut second = revision(2, 4);
    second.previous_revision_id = Some(id(3));
    second.request_id = Some(id(1));
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(second)).await;
}

#[tokio::test]
async fn approved_agent_request_cannot_prepare_an_edit_to_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let writer = DbWriter::open(&dir.path().join("host.db"), 16).unwrap();
    committed(&writer, &revision(2, 3)).await;
    let mut request = proposal();
    request.base_revision_id = Some(id(3));
    request.actor = format!("agent:{}", id(10));
    request.requester = Some(EnvironmentRequestTarget {
        session_id: id(4),
        generation: id(5),
    });
    durable(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(decision(EnvironmentRequestDecisionKind::Approve)),
    )
    .await;
    let mut parent_edit = revision(2, 8);
    parent_edit.previous_revision_id = Some(id(3));
    parent_edit.request_id = Some(id(1));
    rejected(&writer, WriteOp::PrepareEnvironmentRevision(parent_edit)).await;
}
