use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::SystemTime;

use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};

use crate::ledger_protocol::{LedgerClientMessage, LedgerServerMessage};
use crate::ledger_server::LedgerServer;
use crate::{Decision, NetEvent, WriteOp};

fn event(domain: &str) -> WriteOp {
    event_with_id(domain, "0123456789ab")
}

fn event_with_id(domain: &str, event_id: &str) -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some(event_id.into()),
        timestamp: SystemTime::UNIX_EPOCH,
        domain: domain.into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("GET".into()),
        path: Some("/v1".into()),
        query: None,
        status_code: Some(200),
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 1,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body: None,
        response_body: None,
        conn_type: Some("https".into()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    })
}

async fn remote_writer(
    role: LedgerClientRole,
) -> (
    tempfile::TempDir,
    crate::DbWriter,
    tokio::task::JoinHandle<Result<crate::ledger_server::LedgerClientExit, String>>,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&path).unwrap());
    let generation = LedgerGeneration::new([5; 16]);
    let grant = LedgerChannelGrant::new(generation, 9, role).unwrap();
    let (client, worker) = UnixStream::pair().unwrap();
    let (commitment, _commitment_task) = super::test_commitment_channel(grant);
    let task = tokio::spawn(async move { server.serve_client(worker, grant).await });
    let writer = tokio::task::spawn_blocking({
        let path = path.clone();
        move || crate::DbWriter::from_ledger_channel(client, commitment, grant, &path, 8)
    })
    .await
    .unwrap()
    .unwrap();
    (directory, writer, task)
}

#[tokio::test]
async fn descriptor_writer_waits_for_admission_and_flushes_through_the_owner() {
    let (directory, writer, task) = remote_writer(LedgerClientRole::VmOwner).await;
    writer.write_checked(event("remote.example")).await.unwrap();
    writer.flush_checked().await.unwrap();
    let path = directory.path().join("session.db");
    let rows: i64 = rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM net_events WHERE domain = 'remote.example'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        crate::ledger_server::LedgerClientExit::Disconnected
    );
}

#[tokio::test]
async fn descriptor_writer_reports_role_refusal_to_the_producer() {
    let (_directory, writer, task) = remote_writer(LedgerClientRole::Reader).await;
    let error = writer.write_checked(event("denied.example")).await.unwrap_err();
    assert!(
        error.contains("cannot perform") || error.contains("unauthorized") || error.contains("not a producer"),
        "{error}"
    );
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        crate::ledger_server::LedgerClientExit::Disconnected
    );
}

#[tokio::test]
async fn descriptor_writer_preserves_blocking_and_try_admission_order() {
    let (directory, writer, task) = remote_writer(LedgerClientRole::VmOwner).await;
    let writer = Arc::new(writer);
    let blocking = Arc::clone(&writer);
    tokio::task::spawn_blocking(move || {
        blocking
            .write_blocking_checked(event_with_id("blocking.example", "0123456789ac"))
            .unwrap();
    })
    .await
    .unwrap();
    assert!(writer.try_write(event_with_id("try.example", "0123456789ad")));
    writer.flush_checked().await.unwrap();
    let path = directory.path().join("session.db");
    let rows: i64 = rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM net_events WHERE domain IN ('blocking.example', 'try.example')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2);
    let writer = Arc::try_unwrap(writer).ok().unwrap();
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        crate::ledger_server::LedgerClientExit::Disconnected
    );
}

#[tokio::test]
async fn a_stalled_owner_fails_the_pending_operation_and_bounded_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let grant = LedgerChannelGrant::new(LedgerGeneration::new([6; 16]), 10, LedgerClientRole::VmOwner).unwrap();
    let (client, worker) = UnixStream::pair().unwrap();
    let (commitment, _commitment_task) = super::test_commitment_channel(grant);
    let server = tokio::spawn(async move {
        let (sender, receiver) =
            capsem_foundation::ipc_channel::channel_from_std::<LedgerServerMessage, LedgerClientMessage>(worker)
                .unwrap();
        let LedgerClientMessage::Hello { hello } = receiver.recv().await.unwrap() else {
            panic!("expected hello")
        };
        grant.validate_hello(&hello).unwrap();
        sender
            .send(LedgerServerMessage::Welcome {
                welcome: capsem_proto::ledger::LedgerWelcome::for_grant(&grant),
            })
            .await
            .unwrap();
        let _request = receiver.recv().await.unwrap();
        std::future::pending::<()>().await;
    });
    let writer =
        tokio::task::spawn_blocking(move || crate::DbWriter::from_ledger_channel(client, commitment, grant, &path, 2))
            .await
            .unwrap()
            .unwrap();
    let started = std::time::Instant::now();
    let error = writer.write_checked(event("stalled.example")).await.unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    server.abort();
}
