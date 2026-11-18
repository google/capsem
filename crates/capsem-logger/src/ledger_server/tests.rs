use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::SystemTime;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{
    LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerHello, LedgerOutcome, LedgerRequest,
};

use super::*;
use crate::ledger_protocol::{
    LedgerClientMessage, LedgerCommand, LedgerQuery, LedgerReply, LedgerServerMessage, LedgerTimelineLayer,
};
use crate::{Decision, NetEvent, WriteOp};

type ClientSender = ipc_channel::Sender<LedgerClientMessage>;
type ClientReceiver = ipc_channel::Receiver<LedgerServerMessage>;

#[tokio::test]
async fn only_coordinator_can_attest_terminal_approval_in_single_or_batch_admission() {
    use crate::{SecurityAskEvent, SecurityAskPending, SecurityAskStatus};
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let event = SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_001,
        ask_id: "0123456789ab".into(),
        event_id: "0123456789ac".into(),
        event_type: "process.exec".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        rule_json: "{}".into(),
        event_json: "{}".into(),
    });
    for role in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Coordinator,
    ] {
        let (sender, receiver, task) = connect(Arc::clone(&server), grant(role)).await;
        assert!(matches!(
            request(
                &sender,
                &receiver,
                1,
                LedgerCommand::Admit {
                    event: Box::new(WriteOp::SecurityAskEvent(event.clone()))
                }
            )
            .await,
            LedgerReply::Accepted { .. }
        ));
        for (status_index, status) in [SecurityAskStatus::Approved, SecurityAskStatus::Denied]
            .into_iter()
            .enumerate()
        {
            let event = event.clone().with_status(status);
            for (index, operation) in [
                LedgerCommand::Admit {
                    event: Box::new(WriteOp::SecurityAskEvent(event.clone())),
                },
                LedgerCommand::AdmitBatch {
                    events: vec![body_event(vec![]), WriteOp::SecurityAskEvent(event.clone())],
                },
            ]
            .into_iter()
            .enumerate()
            {
                sender
                    .send(LedgerClientMessage::Request {
                        request: LedgerRequest::new((status_index * 2 + index + 2) as u64, operation).unwrap(),
                    })
                    .await
                    .unwrap();
                let LedgerServerMessage::Response { response } = receiver.recv().await.unwrap() else {
                    panic!("response")
                };
                if role == LedgerClientRole::Coordinator {
                    assert!(matches!(response.outcome(), LedgerOutcome::Success { .. }));
                } else {
                    let LedgerOutcome::Failure { failure } = response.outcome() else {
                        panic!("producer forged approval: {role:?}")
                    };
                    assert_eq!(failure.code(), LedgerFailureCode::UnauthorizedOperation);
                }
            }
        }
        drop(sender);
        drop(receiver);
        task.await.unwrap().unwrap();
    }
}

fn grant(role: LedgerClientRole) -> LedgerChannelGrant {
    LedgerChannelGrant::new(LedgerGeneration::new([4; 16]), 17, role).unwrap()
}

async fn connect(
    server: Arc<LedgerServer>,
    authority: LedgerChannelGrant,
) -> (
    ClientSender,
    ClientReceiver,
    tokio::task::JoinHandle<Result<LedgerClientExit, String>>,
) {
    let (client, worker) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move { server.serve_client(worker, authority).await });
    let (sender, receiver) = ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(client).unwrap();
    sender
        .send(LedgerClientMessage::Hello {
            hello: LedgerHello::for_grant(&authority),
        })
        .await
        .unwrap();
    assert!(matches!(
        receiver.recv().await.unwrap(),
        LedgerServerMessage::Welcome { .. }
    ));
    (sender, receiver, task)
}

async fn request(sender: &ClientSender, receiver: &ClientReceiver, id: u64, operation: LedgerCommand) -> LedgerReply {
    sender
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(id, operation).unwrap(),
        })
        .await
        .unwrap();
    let LedgerServerMessage::Response { response } = receiver.recv().await.unwrap() else {
        panic!("expected response")
    };
    assert_eq!(response.request_id(), id);
    match response.outcome() {
        LedgerOutcome::Success { reply } => reply.clone(),
        LedgerOutcome::Failure { failure } => panic!("request failed: {}", failure.message()),
    }
}

fn body_event(body: Vec<u8>) -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some("0123456789ab".into()),
        timestamp: SystemTime::UNIX_EPOCH,
        domain: "example.test".into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("POST".into()),
        path: Some("/v1".into()),
        query: None,
        status_code: Some(200),
        bytes_sent: 2,
        bytes_received: body.len() as u64,
        duration_ms: 1,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body: None,
        response_body: Some(body),
        conn_type: Some("https".into()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    })
}

#[test]
fn prepared_server_holds_writer_authority_until_workers_start() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let prepared = LedgerServer::prepare_with_codecs(&database, capsem_archive::ArchiveCodecs::default()).unwrap();
    assert!(LedgerServer::prepare_with_codecs(&database, capsem_archive::ArchiveCodecs::default()).is_err());
    let server = prepared.start();
    drop(server);
    drop(LedgerServer::prepare_with_codecs(&database, capsem_archive::ArchiveCodecs::default()).unwrap());
}

#[tokio::test]
async fn admission_batch_gets_one_contiguous_acceptance_reply() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&database).unwrap());
    let producer_grant = grant(LedgerClientRole::VmOwner);
    let (producer, replies, task) = connect(Arc::clone(&server), producer_grant).await;
    let mut first = body_event(Vec::new());
    let WriteOp::NetEvent(first_event) = &mut first else {
        unreachable!()
    };
    first_event.event_id = Some("0123456789ac".into());
    first_event.domain = "first.test".into();
    let mut second = body_event(Vec::new());
    let WriteOp::NetEvent(second_event) = &mut second else {
        unreachable!()
    };
    second_event.event_id = Some("0123456789ad".into());
    second_event.domain = "second.test".into();

    assert!(matches!(
        request(
            &producer,
            &replies,
            1,
            LedgerCommand::AdmitBatch {
                events: vec![first, second],
            },
        )
        .await,
        LedgerReply::AcceptedBatch {
            first_admission_id: 1,
            through_admission_id: 2,
            accepted: 2,
        }
    ));
    assert!(matches!(
        request(&producer, &replies, 2, LedgerCommand::Flush).await,
        LedgerReply::Durable {
            through_admission_id: 2
        }
    ));
    drop(producer);
    drop(replies);
    assert_eq!(task.await.unwrap().unwrap(), LedgerClientExit::Disconnected);
}

#[tokio::test]
async fn accepted_durable_query_body_retention_and_warc_run_through_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let producer_grant = grant(LedgerClientRole::VmOwner);
    let (producer, producer_replies, producer_task) = connect(Arc::clone(&server), producer_grant).await;
    let body = vec![b'z'; 300_000];
    let event = body_event(body.clone());
    assert!(matches!(
        request(
            &producer,
            &producer_replies,
            1,
            LedgerCommand::Admit { event: Box::new(event) },
        )
        .await,
        LedgerReply::Accepted { admission_id: 1 }
    ));
    assert!(matches!(
        request(&producer, &producer_replies, 2, LedgerCommand::Flush).await,
        LedgerReply::Durable {
            through_admission_id: 1
        }
    ));

    let reader_grant = LedgerChannelGrant::new(producer_grant.generation(), 18, LedgerClientRole::Maintainer).unwrap();
    let (reader, reader_replies, reader_task) = connect(Arc::clone(&server), reader_grant).await;
    let query = request(
        &reader,
        &reader_replies,
        3,
        LedgerCommand::Query {
            query: LedgerQuery::Timeline {
                layers: vec![LedgerTimelineLayer::Net],
                cutoff: String::new(),
                trace_id: None,
                limit: 10,
            },
        },
    )
    .await;
    assert!(matches!(query, LedgerReply::Query { ref sets } if sets.len() == 1 && sets[0].rows.len() == 1));

    reader
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(
                4,
                LedgerCommand::ReadBodies {
                    event_id: "0123456789ab".into(),
                },
            )
            .unwrap(),
        })
        .await
        .unwrap();
    let mut restored = Vec::new();
    loop {
        let LedgerServerMessage::Response { response } = reader_replies.recv().await.unwrap() else {
            panic!("expected body response")
        };
        let LedgerOutcome::Success { reply } = response.outcome() else {
            panic!("body read failed")
        };
        match reply {
            LedgerReply::BodyStart { metadata, .. } => assert_eq!(metadata.stored_bytes, body.len() as u64),
            LedgerReply::BodyChunk { bytes, .. } => restored.extend_from_slice(bytes),
            LedgerReply::BodiesComplete => break,
            other => panic!("unexpected body reply {other:?}"),
        }
    }
    assert_eq!(restored, body);

    reader
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(5, LedgerCommand::ExportWarc).unwrap(),
        })
        .await
        .unwrap();
    let mut warc = Vec::new();
    loop {
        let LedgerServerMessage::Response { response } = reader_replies.recv().await.unwrap() else {
            panic!("expected WARC response")
        };
        let LedgerOutcome::Success { reply } = response.outcome() else {
            panic!("WARC failed")
        };
        match reply {
            LedgerReply::WarcChunk { bytes, .. } => warc.extend_from_slice(bytes),
            LedgerReply::WarcComplete { summary } => {
                assert_eq!(summary.records, 1);
                break;
            }
            other => panic!("unexpected WARC reply {other:?}"),
        }
    }
    assert!(warc.starts_with(&[0x1f, 0x8b]));
    assert!(matches!(
        request(
            &reader,
            &reader_replies,
            6,
            LedgerCommand::Retain {
                cutoff: "2999-01-01T00:00:00Z".into(),
            },
        )
        .await,
        LedgerReply::Retained { .. }
    ));

    drop(producer);
    drop(reader);
    assert_eq!(producer_task.await.unwrap().unwrap(), LedgerClientExit::Disconnected);
    assert_eq!(reader_task.await.unwrap().unwrap(), LedgerClientExit::Disconnected);
}

#[tokio::test]
async fn wrong_generation_never_reaches_storage_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let expected = grant(LedgerClientRole::Reader);
    let (client, worker) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move { server.serve_client(worker, expected).await });
    let (sender, _receiver) =
        ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(client).unwrap();
    let wrong = LedgerChannelGrant::new(LedgerGeneration::new([8; 16]), expected.client_id(), expected.role()).unwrap();
    sender
        .send(LedgerClientMessage::Hello {
            hello: LedgerHello::for_grant(&wrong),
        })
        .await
        .unwrap();
    drop(sender);
    assert!(task.await.unwrap().unwrap_err().contains("generation"));
}

#[tokio::test]
async fn environment_authority_cannot_be_forged_by_producer_admission() {
    use crate::events::environment::purge::PreparedEnvironmentVariablePurge;
    use crate::events::environment::request::{EnvironmentRequestDecision, EnvironmentRequestDecisionKind};
    use crate::events::{EnvironmentRevisionState, EnvironmentRevisionTransition};
    let id = |n: u64| format!("00000000-0000-4000-8000-{n:012x}");
    let mut operations = vec![
        WriteOp::ObserveCatalogRevocation(crate::catalog_revocations::CatalogRevocation {
            key: crate::catalog_revocations::CatalogRevocationKey {
                kind: capsem_config::catalogs::CatalogKind::Skills,
                source_repository: "registry.example/team/catalog".into(),
                entry_id: "review".into(),
                version: "1.0.0".into(),
            },
            catalog_digest: format!("sha256:{}", "a".repeat(64)),
            observed_at_unix_ms: 10,
        }),
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(2),
            revision_id: id(3),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 10,
            actor: "operator".into(),
        }),
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(4),
            timestamp_unix_ms: 10,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "forged approval".into(),
        }),
        WriteOp::PrepareEnvironmentVariablePurge(PreparedEnvironmentVariablePurge {
            purge_id: id(5),
            environment_id: id(2),
            source_revision_id: id(3),
            variable_name: "WRONG".into(),
            timestamp_unix_ms: 10,
            actor: "operator".into(),
        }),
    ];
    operations.extend(
        crate::HostEventKind::ALL
            .into_iter()
            .filter(|kind| kind.as_str().starts_with("environment_"))
            .map(|kind| {
                WriteOp::HostEvent(crate::HostEvent {
                    timestamp_unix_ms: 10,
                    kind,
                    session_id: None,
                    actor: "operator".into(),
                    detail: vec![0x80],
                    trace_id: None,
                })
            }),
    );
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    for role in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Reader,
        LedgerClientRole::Maintainer,
        LedgerClientRole::Supervisor,
    ] {
        let (sender, receiver, task) = connect(Arc::clone(&server), grant(role)).await;
        let mut request_id = 1;
        for event in &operations {
            for command in [
                LedgerCommand::Admit {
                    event: Box::new(event.clone()),
                },
                LedgerCommand::AdmitBatch {
                    events: vec![body_event(vec![]), event.clone()],
                },
            ] {
                sender
                    .send(LedgerClientMessage::Request {
                        request: LedgerRequest::new(request_id, command).unwrap(),
                    })
                    .await
                    .unwrap();
                let LedgerServerMessage::Response { response } = receiver.recv().await.unwrap() else {
                    panic!("response")
                };
                let LedgerOutcome::Failure { failure } = response.outcome() else {
                    panic!("{role:?} forged environment authority")
                };
                assert_eq!(failure.code(), LedgerFailureCode::UnauthorizedOperation);
                request_id += 1;
            }
        }
        assert_eq!(
            server.admission.lock().await.last_accepted,
            0,
            "mixed-batch rejection must not admit its telemetry prefix"
        );
        drop(sender);
        drop(receiver);
        task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn coordinator_environment_mutations_keep_the_existing_durability_path() {
    use crate::events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision};
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let (sender, receiver, task) = connect(Arc::clone(&server), grant(LedgerClientRole::Coordinator)).await;
    let environment_id = "00000000-0000-4000-8000-000000000002".to_owned();
    let revision_id = "00000000-0000-4000-8000-000000000003".to_owned();
    let revision = PreparedEnvironmentRevision {
        environment_id: environment_id.clone(),
        revision_id: revision_id.clone(),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 1,
        actor: "operator".into(),
        reason: "operator edit".into(),
        request_id: None,
        payload_event_id: "000000000003".into(),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    };
    assert!(matches!(
        request(
            &sender,
            &receiver,
            1,
            LedgerCommand::Admit {
                event: Box::new(WriteOp::PrepareEnvironmentRevision(revision))
            }
        )
        .await,
        LedgerReply::Accepted { .. }
    ));
    assert!(matches!(
        request(&sender, &receiver, 2, LedgerCommand::Flush).await,
        LedgerReply::Durable { .. }
    ));
    assert!(matches!(
        request(
            &sender,
            &receiver,
            3,
            LedgerCommand::AdmitBatch {
                events: vec![
                    body_event(vec![]),
                    WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
                        environment_id: environment_id.clone(),
                        revision_id: revision_id.clone(),
                        state: EnvironmentRevisionState::Committed,
                        timestamp_unix_ms: 2,
                        actor: "operator".into()
                    })
                ]
            }
        )
        .await,
        LedgerReply::AcceptedBatch { .. }
    ));
    assert!(matches!(
        request(&sender, &receiver, 4, LedgerCommand::Flush).await,
        LedgerReply::Durable { .. }
    ));
    let snapshot = server
        .db
        .environment_revision_snapshot(&environment_id, &revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.record.state, EnvironmentRevisionState::Committed);
    assert_eq!(snapshot.specification.name, "development");
    drop(sender);
    drop(receiver);
    task.await.unwrap().unwrap();
}

mod purge;

mod environment;

#[tokio::test]
async fn coordinator_environment_audit_evidence_remains_durable() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let (sender, receiver, task) = connect(Arc::clone(&server), grant(LedgerClientRole::Coordinator)).await;
    let events = crate::HostEventKind::ALL
        .into_iter()
        .filter(|kind| kind.as_str().starts_with("environment_"))
        .map(|kind| {
            WriteOp::HostEvent(crate::HostEvent {
                timestamp_unix_ms: 10,
                kind,
                session_id: None,
                actor: "operator".into(),
                detail: vec![0x80],
                trace_id: None,
            })
        })
        .collect::<Vec<_>>();
    let count = events.len();
    assert!(
        matches!(request(&sender,&receiver,1,LedgerCommand::AdmitBatch {events}).await,LedgerReply::AcceptedBatch {accepted,..} if usize::from(accepted)==count)
    );
    assert!(matches!(
        request(&sender, &receiver, 2, LedgerCommand::Flush).await,
        LedgerReply::Durable { .. }
    ));
    let rows = server
        .db
        .query(
            "SELECT COUNT(*) FROM host_events WHERE actor='operator' AND kind LIKE 'environment_%'",
            &[],
        )
        .await
        .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(rows["rows"][0][0].as_u64(), Some(count as u64));
    drop(sender);
    drop(receiver);
    task.await.unwrap().unwrap();
}
