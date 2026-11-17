use super::*;
use crate::db::{EnvironmentRevisionQuery, EnvironmentRevisionRecord, EnvironmentRevisionSnapshot};
use crate::events::EnvironmentRevisionState;
use crate::ledger_protocol::{LedgerEnvironmentQuery, LedgerEnvironmentReply, LedgerResponse};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn record() -> EnvironmentRevisionRecord {
    EnvironmentRevisionRecord {
        cursor: 1,
        environment_id: id(2),
        revision_id: id(3),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 1,
        actor: "operator".into(),
        reason: "read fixture".into(),
        request_id: None,
        state: EnvironmentRevisionState::Committed,
        payload_event_id: "000000000003".into(),
        replayable: true,
    }
}
fn snapshot() -> EnvironmentRevisionSnapshot {
    EnvironmentRevisionSnapshot {
        record: record(),
        specification: Box::new(
            toml::from_str("name='development'\n[variables]\nWRONG='other-environment-value-canary'").unwrap(),
        ),
    }
}

async fn peer(
    reply: LedgerEnvironmentReply,
    healthy: bool,
) -> (
    LedgerClient,
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
) {
    let authority = LedgerChannelGrant::new(GENERATION, 2, LedgerClientRole::Reader).unwrap();
    let (client, worker) = std::os::unix::net::UnixStream::pair().unwrap();
    let (accepted, received) = oneshot::channel();
    let (resume, proceed) = oneshot::channel();
    let worker = tokio::spawn(async move {
        let (sender, receiver) =
            ipc_channel::channel_from_std::<LedgerServerMessage, LedgerClientMessage>(worker).unwrap();
        let LedgerClientMessage::Hello { hello } = receiver.recv().await.unwrap() else {
            panic!("hello")
        };
        authority.validate_hello(&hello).unwrap();
        sender
            .send(LedgerServerMessage::Welcome {
                welcome: capsem_proto::ledger::LedgerWelcome::for_grant(&authority),
            })
            .await
            .unwrap();
        let LedgerClientMessage::Request { request } = receiver.recv().await.unwrap() else {
            panic!("environment read")
        };
        request.authorize(&authority).unwrap();
        assert!(matches!(request.operation(), LedgerCommand::EnvironmentRead { .. }));
        accepted.send(()).unwrap();
        proceed.await.unwrap();
        sender
            .send(LedgerServerMessage::Response {
                response: LedgerResponse::success(
                    request.request_id(),
                    LedgerReply::EnvironmentRead { reply: Box::new(reply) },
                )
                .unwrap(),
            })
            .await
            .unwrap();
        if healthy {
            let LedgerClientMessage::Request { request } = receiver.recv().await.unwrap() else {
                panic!("counters")
            };
            assert!(matches!(request.operation(), LedgerCommand::Counters));
            sender
                .send(LedgerServerMessage::Response {
                    response: LedgerResponse::success(
                        request.request_id(),
                        LedgerReply::Counters {
                            counters: Box::default(),
                            read_cache_epoch: 7,
                        },
                    )
                    .unwrap(),
                })
                .await
                .unwrap();
        } else {
            assert!(tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .is_err());
        }
    });
    let client = LedgerClient::connect(client, authority, PathBuf::from("/unused/session.db"))
        .await
        .unwrap();
    (client, worker, received, resume)
}

#[tokio::test]
async fn environment_read_client_rejects_corrupt_mismatched_or_out_of_scope_replies() {
    let mut wrong = snapshot();
    wrong.record.environment_id = id(9);
    let mut nonreplay = snapshot();
    nonreplay.record.replayable = false;
    for reply in [
        LedgerEnvironmentReply::RevisionSnapshot {
            snapshot: Some(Box::new(wrong)),
        },
        LedgerEnvironmentReply::Requests { records: vec![] },
        LedgerEnvironmentReply::RevisionSnapshot {
            snapshot: Some(Box::new(nonreplay)),
        },
        LedgerEnvironmentReply::Revisions {
            records: vec![record(); crate::ledger_protocol::MAX_LEDGER_QUERY_ROWS + 1],
        },
    ] {
        let (client, worker, received, resume) = peer(reply, false).await;
        let caller = client.clone();
        let call = tokio::spawn(async move {
            caller
                .environment_read(LedgerEnvironmentQuery::RevisionSnapshot {
                    environment_id: id(2),
                    revision_id: id(3),
                })
                .await
        });
        received.await.unwrap();
        resume.send(()).unwrap();
        let error = call.await.unwrap().unwrap_err();
        assert!(!error.contains("other-environment-value-canary"));
        assert!(!error.contains(&id(9)));
        drop(client);
        worker.await.unwrap();
    }
    let (client, worker, received, resume) = peer(
        LedgerEnvironmentReply::Revisions {
            records: vec![record(), record()],
        },
        false,
    )
    .await;
    let caller = client.clone();
    let call = tokio::spawn(async move {
        caller
            .environment_read(LedgerEnvironmentQuery::Revisions {
                query: EnvironmentRevisionQuery::Page {
                    environment_id: Some(id(2)),
                    state: None,
                    after_id: 0,
                    limit: 2,
                },
            })
            .await
    });
    received.await.unwrap();
    resume.send(()).unwrap();
    assert!(call.await.unwrap().is_err());
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn environment_read_client_consumes_reply_after_caller_cancellation() {
    let (client, worker, received, resume) = peer(
        LedgerEnvironmentReply::RevisionSnapshot {
            snapshot: Some(Box::new(snapshot())),
        },
        true,
    )
    .await;
    let caller = client.clone();
    let call = tokio::spawn(async move {
        caller
            .environment_read(LedgerEnvironmentQuery::RevisionSnapshot {
                environment_id: id(2),
                revision_id: id(3),
            })
            .await
    });
    received.await.unwrap();
    call.abort();
    resume.send(()).unwrap();
    assert!(call.await.unwrap_err().is_cancelled());
    client.counters().await.unwrap();
    assert_eq!(client.read_cache_epoch(), 7);
    drop(client);
    worker.await.unwrap();
}
