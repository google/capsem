//! Reviewed content crosses the real confined process unchanged, including after restart.
use super::*;
use capsem_logger::ledger_protocol::{LedgerEnvironmentQuery as Q, LedgerEnvironmentReply as R};

fn specification() -> EnvironmentSpecification {
    let mut value: EnvironmentSpecification = toml::from_str(
        r#"
name='development'
[image]
reference='development:stable'
resolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
[[repositories]]
url='https://example.org/team/project.git'
ref='main'
resolved_commit='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
destination='project'
[[skills.selected]]
id='sandbox'
version='1.0.0'
catalog_reference='registry.example/skills:stable'
resolved_digest='sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
[[access.catalog.selected]]
id='access-preset'
version='1.0.0'
catalog_reference='registry.example/rules:stable'
resolved_digest='sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
[[detection.catalog.selected]]
id='observe'
version='1.0.0'
catalog_reference='registry.example/rules:stable'
resolved_digest='sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
[[mcps.catalog.selected]]
id='server'
version='1.0.0'
catalog_reference='registry.example/mcps:stable'
resolved_digest='sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'
[plugins.pii_filter.inbound]
mode='rewrite'
detection_level='high'
[plugins.pii_filter.inbound.options]
email=false
phone=true
[plugins.pii_filter.outbound]
mode='block'
detection_level='low'
[plugins.pii_filter.outbound.options]
email=true
phone=false
[plugins.credential_broker.options]
capture_enabled=false
"#,
    )
    .unwrap();
    value.startup = Some(capsem_config::environment::EnvironmentStartup {
        script: "printf 'native proposal script'\n".repeat(1024),
        timeout_seconds: 30,
    });
    value.variables.insert("LARGE".into(), "retained ".repeat(1024));
    value.validate().unwrap();
    value
}

async fn assert_snapshots(client: &LedgerClient, expected: &EnvironmentSpecification) {
    let R::Revisions { records } = client
        .environment_read(Q::Revisions {
            query: capsem_logger::db::EnvironmentRevisionQuery::ForRequest { request_id: id(40) },
        })
        .await
        .unwrap()
    else {
        panic!("native indexed request result lookup")
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request_id, Some(id(40)));
    assert_eq!(records[0].environment_id, id(2));
    assert_eq!(records[0].revision_id, id(3));
    assert_eq!(records[0].state, EnvironmentRevisionState::Committed);
    let R::RequestSnapshot {
        snapshot: Some(snapshot),
    } = client
        .environment_read(Q::RequestSnapshot { request_id: id(40) })
        .await
        .unwrap()
    else {
        panic!("durable reviewed proposal")
    };
    let EnvironmentRequestPayload::Change { specification } = snapshot.payload.as_ref() else {
        panic!("canonical change proposal")
    };
    assert_eq!(specification.as_ref(), expected);
    assert_eq!(snapshot.record.state, EnvironmentRequestState::Approved);
    let R::RevisionSnapshot {
        snapshot: Some(snapshot),
    } = client
        .environment_read(Q::RevisionSnapshot {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap()
    else {
        panic!("durable exact approved result")
    };
    assert_eq!(snapshot.specification.as_ref(), expected);
    assert_eq!(snapshot.record.state, EnvironmentRevisionState::Committed);
}

#[tokio::test]
async fn executable_approval_preserves_resolved_content_and_rejects_pin_substitution_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let worker = Worker::start(&database, 0x73).await;
    let (sender, receiver) = worker.producer(1, LedgerClientRole::Coordinator).await;
    let mut next = 1;
    let specification = specification();
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: None,
            requester: None,
            timestamp_unix_ms: 30,
            expires_at_unix_ms: 1000,
            actor: "operator".into(),
            reason: "native resolved proposal".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification.clone()),
            }),
        }),
    )
    .await;
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(40),
            timestamp_unix_ms: 31,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "reviewed exact canonical content".into(),
        }),
    )
    .await;
    // Approval and result preparation happen in different worker lifetimes.
    worker.stop().await;
    let worker = Worker::start(&database, 0x74).await;
    let (sender, receiver) = worker.producer(1, LedgerClientRole::Coordinator).await;
    let result = PreparedEnvironmentRevision {
        environment_id: id(2),
        revision_id: id(3),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 40,
        actor: "operator".into(),
        reason: "apply approved content".into(),
        request_id: Some(id(40)),
        payload_event_id: "000000000003".into(),
        specification: Box::new(specification.clone()),
    };
    let mut substituted = result.clone();
    substituted.specification.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "f".repeat(64));
    let response = command(
        &sender,
        &receiver,
        next,
        LedgerCommand::Admit {
            event: Box::new(WriteOp::PrepareEnvironmentRevision(substituted)),
        },
    )
    .await;
    next += 1;
    assert!(matches!(
        response.outcome(),
        LedgerOutcome::Success {
            reply: LedgerReply::Accepted { .. }
        }
    ));
    let response = command(&sender, &receiver, next, LedgerCommand::Flush).await;
    next += 1;
    assert!(
        matches!(response.outcome(), LedgerOutcome::Failure { .. }),
        "{response:?}"
    );
    // A rejected result must not reserve the business revision or prevent the exact result.
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::PrepareEnvironmentRevision(result),
    )
    .await;
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(2),
            revision_id: id(3),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 41,
            actor: "operator".into(),
        }),
    )
    .await;
    let client = worker.maintainer(2, &database).await;
    assert_snapshots(&client, &specification).await;
    drop(client);
    worker.stop().await;
    let worker = Worker::start(&database, 0x75).await;
    let client = worker.maintainer(1, &database).await;
    assert_snapshots(&client, &specification).await;
    drop(client);
    worker.stop().await;
}
