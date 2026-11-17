use super::*;
use crate::events::environment::purge::*;
use crate::ledger_protocol::LedgerResponse;

fn record() -> EnvironmentVariablePurgeRecord {
    let id = |n: u64| format!("00000000-0000-4000-8000-{n:012x}");
    EnvironmentVariablePurgeRecord {
        cursor: 1,
        intent: PreparedEnvironmentVariablePurge {
            purge_id: id(5),
            environment_id: id(2),
            source_revision_id: id(3),
            variable_name: "WRONG".into(),
            timestamp_unix_ms: 20,
            actor: "operator".into(),
        },
        state: EnvironmentVariablePurgeState::Committed,
        old_generation_id: Some([1; 16]),
        new_generation_id: Some([2; 16]),
        failure_reason: None,
        revision_count: 1,
        request_count: 0,
    }
}

async fn delayed_purge(cancel: bool) {
    let authority = LedgerChannelGrant::new(GENERATION, 2, LedgerClientRole::Maintainer).unwrap();
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
            panic!("purge")
        };
        request.authorize(&authority).unwrap();
        assert!(matches!(
            request.operation(),
            LedgerCommand::PurgeVariableHistory { .. }
        ));
        accepted.send(()).unwrap();
        proceed.await.unwrap();
        tokio::time::sleep(OPERATION_TIMEOUT + Duration::from_millis(100)).await;
        sender
            .send(LedgerServerMessage::Response {
                response: LedgerResponse::success(
                    request.request_id(),
                    LedgerReply::VariableHistoryPurged {
                        record: Box::new(record()),
                    },
                )
                .unwrap(),
            })
            .await
            .unwrap();
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
                        read_cache_epoch: 42,
                    },
                )
                .unwrap(),
            })
            .await
            .unwrap();
    });
    let client = LedgerClient::connect(client, authority, PathBuf::from("/unused/session.db"))
        .await
        .unwrap();
    let caller = client.clone();
    let purge = tokio::spawn(async move {
        caller
            .purge_variable_history(EnvironmentVariablePurgeExecution {
                purge_id: record().intent.purge_id,
                timestamp_unix_ms: 21,
            })
            .await
    });
    received.await.unwrap();
    if cancel {
        purge.abort();
    }
    resume.send(()).unwrap();
    if cancel {
        assert!(purge.await.unwrap_err().is_cancelled());
    } else {
        assert_eq!(purge.await.unwrap().unwrap(), record());
    }
    client.counters().await.unwrap();
    assert_eq!(client.read_cache_epoch(), 42);
    drop(client);
    worker.await.unwrap();
}

#[tokio::test]
async fn variable_purge_client_waits_for_durable_work_beyond_admission_deadline() {
    delayed_purge(false).await;
}
#[tokio::test]
async fn variable_purge_client_consumes_accepted_reply_after_caller_cancellation() {
    delayed_purge(true).await;
}

#[test]
fn variable_purge_reply_refuses_corrupt_or_unbounded_metadata() {
    for mutate in [
        |r: &mut EnvironmentVariablePurgeRecord| r.cursor = 0,
        |r: &mut EnvironmentVariablePurgeRecord| r.revision_count = 0,
        |r: &mut EnvironmentVariablePurgeRecord| r.old_generation_id = None,
        |r: &mut EnvironmentVariablePurgeRecord| r.new_generation_id = Some([0; 16]),
        |r: &mut EnvironmentVariablePurgeRecord| {
            r.failure_reason = Some("x".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES + 1))
        },
    ] {
        let mut value = record();
        mutate(&mut value);
        assert!(LedgerReply::VariableHistoryPurged {
            record: Box::new(value)
        }
        .validate()
        .is_err());
    }
}
