use super::*;
use capsem_proto::ledger::{LedgerClientRole, LedgerGeneration};
use capsem_proto::ledger::{LedgerHello, LedgerRequest};
use std::sync::Arc;

use capsem_logger::ledger_protocol::{LedgerClientMessage, LedgerCommand, LedgerServerMessage};
use capsem_logger::{Decision, NetEvent, WriteOp};

fn grant(generation: u8, client_id: u64) -> LedgerChannelGrant {
    LedgerChannelGrant::new(
        LedgerGeneration::new([generation; 16]),
        client_id,
        LedgerClientRole::Proxy,
    )
    .unwrap()
}

fn event(domain: &str) -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some("0123456789ab".into()),
        timestamp: std::time::SystemTime::UNIX_EPOCH,
        domain: domain.into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("GET".into()),
        path: Some("/".into()),
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

async fn write_committed(
    server: Arc<capsem_logger::ledger_server::LedgerServer>,
    grant: LedgerChannelGrant,
    event: WriteOp,
    commitment: LedgerCommitment,
) {
    let (client, worker) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move { server.serve_client(worker, grant).await });
    let (sender, receiver) =
        capsem_foundation::ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(client).unwrap();
    sender
        .send(LedgerClientMessage::Hello {
            hello: LedgerHello::for_grant(&grant),
        })
        .await
        .unwrap();
    assert!(matches!(
        receiver.recv().await.unwrap(),
        LedgerServerMessage::Welcome { .. }
    ));
    for (request_id, command) in [
        (
            1,
            LedgerCommand::Admit {
                event: Box::new(event),
                commitment,
            },
        ),
        (2, LedgerCommand::Flush),
    ] {
        sender
            .send(LedgerClientMessage::Request {
                request: LedgerRequest::new(request_id, command).unwrap(),
            })
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await.unwrap(),
            LedgerServerMessage::Response { .. }
        ));
    }
    drop(sender);
    drop(receiver);
    task.await.unwrap().unwrap();
}

async fn reader(
    server: Arc<capsem_logger::ledger_server::LedgerServer>,
    path: &Path,
) -> capsem_logger::ledger_client::LedgerClient {
    let grant = LedgerChannelGrant::new(LedgerGeneration::new([9; 16]), 99, LedgerClientRole::Reader).unwrap();
    let (client, worker) = UnixStream::pair().unwrap();
    tokio::spawn(async move { server.serve_client(worker, grant).await });
    capsem_logger::ledger_client::LedgerClient::connect(client, grant, path.to_path_buf())
        .await
        .unwrap()
}

#[tokio::test]
async fn durable_checkpoints_survive_restart_and_continue_global_order() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-a").await.unwrap();
    let first = grant(1, 7);
    let global = authority
        .reserve(first, 1, "net_event".into(), [3; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    let commitment = LedgerCommitment::new(first, 1, global, "net_event", [3; 32], ZERO_COMMITMENT_HASH).unwrap();
    authority.anchor(first, vec![commitment.clone()]).await.unwrap();
    drop(authority);

    let restarted = CommitmentAuthority::open(root.path(), "session-a").await.unwrap();
    assert_eq!(restarted.anchored().await, vec![commitment]);
    let second = grant(2, 1);
    assert_eq!(
        restarted
            .reserve(second, 1, "dns_event".into(), [4; 32], ZERO_COMMITMENT_HASH)
            .await
            .unwrap(),
        global + 1
    );
}

#[tokio::test]
async fn multiple_producers_share_global_order_and_unanchored_tail_is_not_a_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-tail").await.unwrap();
    let first = grant(1, 7);
    let second = grant(1, 8);

    let first_global = authority
        .reserve(first, 1, "net_event".into(), [1; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    let second_global = authority
        .reserve(second, 1, "dns_event".into(), [2; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    assert_eq!(second_global, first_global + 1);

    let first_commitment =
        LedgerCommitment::new(first, 1, first_global, "net_event", [1; 32], ZERO_COMMITMENT_HASH).unwrap();
    authority.anchor(first, vec![first_commitment.clone()]).await.unwrap();
    drop(authority);

    let restarted = CommitmentAuthority::open(root.path(), "session-tail").await.unwrap();
    assert_eq!(restarted.anchored().await, vec![first_commitment]);
    assert_eq!(
        restarted
            .reserve(second, 1, "dns_event".into(), [2; 32], ZERO_COMMITMENT_HASH)
            .await
            .unwrap(),
        first_global + 1
    );
}

#[tokio::test]
async fn substitution_reorder_and_stale_authority_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-b").await.unwrap();
    let owner = grant(3, 4);
    let global = authority
        .reserve(owner, 1, "net_event".into(), [8; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    let expected = LedgerCommitment::new(owner, 1, global, "net_event", [8; 32], ZERO_COMMITMENT_HASH).unwrap();

    let stale = LedgerCommitment::new(grant(2, 4), 1, global, "net_event", [8; 32], ZERO_COMMITMENT_HASH).unwrap();
    assert!(authority.anchor(owner, vec![stale]).await.is_err());
    let substituted = LedgerCommitment::new(owner, 1, global, "dns_event", [8; 32], ZERO_COMMITMENT_HASH).unwrap();
    assert!(authority.anchor(owner, vec![substituted]).await.is_err());
    assert!(authority
        .reserve(owner, 3, "net_event".into(), [9; 32], expected.commitment_hash())
        .await
        .is_err());
    authority.anchor(owner, vec![expected]).await.unwrap();
}

#[tokio::test]
async fn incomplete_unacknowledged_tail_is_recovered_but_corruption_is_loud() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-c").await.unwrap();
    let path = authority.path.clone();
    drop(authority);
    std::fs::write(&path, [CHECKPOINT_MAGIC.as_slice(), &[0, 0]].concat()).unwrap();
    CommitmentAuthority::open(root.path(), "session-c").await.unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

    std::fs::write(&path, [b"NOPE".as_slice(), &[0, 0, 0, 1], &[0], &[0; 32]].concat()).unwrap();
    assert!(CommitmentAuthority::open(root.path(), "session-c").await.is_err());
}

#[tokio::test]
async fn typed_verification_accepts_exact_rows_and_rejects_anchored_omission_and_substitution() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.db");
    let server = Arc::new(capsem_logger::ledger_server::LedgerServer::open(&path).unwrap());
    let authority = CommitmentAuthority::open(&root.path().join("outside"), "session-d")
        .await
        .unwrap();
    let producer = grant(4, 5);
    let expected_event = event("expected.example");
    let expected_hash = capsem_logger::commitment_event_hash(&expected_event).unwrap();
    let global = authority
        .reserve(
            producer,
            1,
            expected_event.kind().into(),
            expected_hash,
            ZERO_COMMITMENT_HASH,
        )
        .await
        .unwrap();
    let expected = LedgerCommitment::new(
        producer,
        1,
        global,
        expected_event.kind(),
        expected_hash,
        ZERO_COMMITMENT_HASH,
    )
    .unwrap();
    authority.anchor(producer, vec![expected.clone()]).await.unwrap();

    let client = reader(Arc::clone(&server), &path).await;
    assert!(authority
        .verify_ledger(&client)
        .await
        .unwrap_err()
        .to_string()
        .contains("omitted anchored"));
    drop(client);

    let substituted_event = event("substituted.example");
    let substituted = LedgerCommitment::new(
        producer,
        1,
        global,
        substituted_event.kind(),
        capsem_logger::commitment_event_hash(&substituted_event).unwrap(),
        ZERO_COMMITMENT_HASH,
    )
    .unwrap();
    write_committed(Arc::clone(&server), producer, substituted_event, substituted).await;
    let client = reader(Arc::clone(&server), &path).await;
    assert!(authority
        .verify_ledger(&client)
        .await
        .unwrap_err()
        .to_string()
        .contains("substituted or altered"));
    drop(client);

    let exact_root = tempfile::tempdir().unwrap();
    let exact_path = exact_root.path().join("session.db");
    let exact_server = Arc::new(capsem_logger::ledger_server::LedgerServer::open(&exact_path).unwrap());
    write_committed(Arc::clone(&exact_server), producer, expected_event, expected.clone()).await;
    let exact_authority = CommitmentAuthority::open(&exact_root.path().join("outside"), "session-e")
        .await
        .unwrap();
    let reserved = exact_authority
        .reserve(
            producer,
            1,
            expected.event_kind().into(),
            expected.event_hash(),
            ZERO_COMMITMENT_HASH,
        )
        .await
        .unwrap();
    assert_eq!(reserved, expected.global_sequence());
    exact_authority.anchor(producer, vec![expected]).await.unwrap();
    let client = reader(exact_server, &exact_path).await;
    exact_authority.verify_ledger(&client).await.unwrap();
}
