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

#[tokio::test]
async fn accepted_durable_query_body_retention_and_warc_run_through_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let producer_grant = grant(LedgerClientRole::VmOwner);
    let (producer, producer_replies, producer_task) = connect(Arc::clone(&server), producer_grant).await;
    let body = vec![b'z'; 300_000];
    assert!(matches!(
        request(
            &producer,
            &producer_replies,
            1,
            LedgerCommand::Admit {
                event: Box::new(body_event(body.clone())),
            },
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
