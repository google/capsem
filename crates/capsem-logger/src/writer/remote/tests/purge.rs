use super::*;
use crate::events::environment::purge::*;
use crate::ledger_protocol::{LedgerCommand, LedgerReply, LedgerResponse};
use std::time::Duration;

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
    let authority = LedgerChannelGrant::new(LedgerGeneration::new([9; 16]), 2, LedgerClientRole::Maintainer).unwrap();
    let (client, worker) = UnixStream::pair().unwrap();
    let (accepted, received) = tokio::sync::oneshot::channel();
    let (resume, proceed) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        let (sender, receiver) =
            capsem_foundation::ipc_channel::channel_from_std::<LedgerServerMessage, LedgerClientMessage>(worker)
                .unwrap();
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
        tokio::time::sleep(super::super::OPERATION_TIMEOUT + Duration::from_millis(100)).await;
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
            panic!("purge retry")
        };
        request.authorize(&authority).unwrap();
        assert!(matches!(
            request.operation(),
            LedgerCommand::PurgeVariableHistory { .. }
        ));
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
        assert!(
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .is_err(),
            "maintainer must close without sending an unauthorized producer flush"
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let writer = Arc::new(
        tokio::task::spawn_blocking(move || crate::DbWriter::from_ledger_channel(client, authority, &path, 8))
            .await
            .unwrap()
            .unwrap(),
    );
    let caller = Arc::clone(&writer);
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
    assert_eq!(
        writer
            .purge_variable_history(EnvironmentVariablePurgeExecution {
                purge_id: record().intent.purge_id,
                timestamp_unix_ms: 21
            })
            .await
            .unwrap(),
        record()
    );
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn variable_purge_remote_writer_survives_slow_durable_work_and_closes_without_producer_flush() {
    delayed_purge(false).await;
}
#[tokio::test]
async fn variable_purge_remote_writer_consumes_reply_after_caller_cancellation() {
    delayed_purge(true).await;
}
