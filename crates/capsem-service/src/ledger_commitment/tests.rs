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

fn commitment(grant: LedgerChannelGrant, producer_sequence: u64, global_sequence: u64) -> LedgerCommitment {
    LedgerCommitment::new(
        grant,
        producer_sequence,
        global_sequence,
        "net_event",
        [producer_sequence as u8; 32],
        ZERO_COMMITMENT_HASH,
    )
    .unwrap()
}

#[tokio::test]
async fn reservations_can_only_cancel_the_unanchored_tail_and_abandon_revokes_all_pending() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-cancel").await.unwrap();
    let producer = grant(5, 6);

    assert!(authority.cancel(producer, 1).await.is_err());
    let first = authority
        .reserve(producer, 1, "net_event".into(), [1; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    let first_commitment =
        LedgerCommitment::new(producer, 1, first, "net_event", [1; 32], ZERO_COMMITMENT_HASH).unwrap();
    let second = authority
        .reserve(
            producer,
            2,
            "net_event".into(),
            [2; 32],
            first_commitment.commitment_hash(),
        )
        .await
        .unwrap();
    assert!(authority.cancel(producer, first).await.is_err());
    authority.cancel(producer, second).await.unwrap();
    authority.cancel(producer, first).await.unwrap();
    assert!(authority.state.lock().await.pending.is_empty());

    let third = authority
        .reserve(producer, 1, "net_event".into(), [3; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    authority.abandon(producer).await;
    let state = authority.state.lock().await;
    assert!(state.pending.is_empty());
    assert!(!state.global_sequences.contains(&third));
    drop(state);
}

#[tokio::test]
async fn reservation_and_checkpoint_sequence_exhaustion_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-overflow")
        .await
        .unwrap();
    let producer = grant(6, 7);
    authority.state.lock().await.next_global_sequence = u64::MAX;
    assert!(authority
        .reserve(producer, 1, "net_event".into(), [1; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap_err()
        .to_string()
        .contains("global commitment sequence exhausted"));

    let key = ProducerKey::from_grant(producer);
    {
        let mut state = authority.state.lock().await;
        state.next_global_sequence = 0;
        state.anchored.insert(
            key,
            ProducerTail {
                sequence: u64::MAX,
                hash: [9; 32],
            },
        );
    }
    assert!(authority
        .reserve(producer, 1, "net_event".into(), [1; 32], [9; 32])
        .await
        .unwrap_err()
        .to_string()
        .contains("producer sequence exhausted"));

    {
        let mut state = authority.state.lock().await;
        state.anchored.remove(&key);
        state.next_checkpoint_sequence = u64::MAX;
    }
    let global = authority
        .reserve(producer, 1, "net_event".into(), [4; 32], ZERO_COMMITMENT_HASH)
        .await
        .unwrap();
    let reserved = LedgerCommitment::new(producer, 1, global, "net_event", [4; 32], ZERO_COMMITMENT_HASH).unwrap();
    assert!(authority
        .anchor(producer, vec![reserved])
        .await
        .unwrap_err()
        .to_string()
        .contains("checkpoint sequence exhausted"));
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").unwrap();
            hex
        })
}

fn commitment_row(commitment: &LedgerCommitment, role: &str) -> Vec<LedgerValue> {
    vec![
        LedgerValue::Integer(commitment.global_sequence() as i64),
        LedgerValue::Text(hex(&commitment.generation().as_bytes())),
        LedgerValue::Integer(commitment.client_id() as i64),
        LedgerValue::Text(role.to_string()),
        LedgerValue::Integer(commitment.producer_sequence() as i64),
        LedgerValue::Text(commitment.event_kind().to_string()),
        LedgerValue::Text(hex(&commitment.event_hash())),
        LedgerValue::Text(hex(&commitment.previous_hash())),
        LedgerValue::Text(hex(&commitment.commitment_hash())),
    ]
}

#[test]
fn typed_commitment_rows_reject_malformed_or_forged_fields() {
    let producer = grant(7, 8);
    let expected = commitment(producer, 1, 1);
    assert_eq!(
        decode_commitment_row(&commitment_row(&expected, "proxy")).unwrap(),
        expected
    );
    assert!(decode_commitment_row(&[])
        .unwrap_err()
        .to_string()
        .contains("invalid typed row"));

    for (index, replacement, message) in [
        (1, LedgerValue::Text("00".into()), "wrong length"),
        (2, LedgerValue::Integer(-1), "client id is negative"),
        (3, LedgerValue::Text("reader".into()), "invalid producer role"),
        (4, LedgerValue::Integer(-1), "producer sequence is negative"),
        (6, LedgerValue::Text("gg".repeat(32)), "hex field is invalid"),
        (8, LedgerValue::Text("00".repeat(32)), "hash does not match"),
    ] {
        let mut row = commitment_row(&expected, "proxy");
        row[index] = replacement;
        assert!(decode_commitment_row(&row).unwrap_err().to_string().contains(message));
    }
    let mut row = commitment_row(&expected, "proxy");
    row[0] = LedgerValue::Integer(-1);
    assert!(decode_commitment_row(&row)
        .unwrap_err()
        .to_string()
        .contains("global sequence is negative"));

    for (role, role_name) in [
        (LedgerClientRole::VmOwner, "vm_owner"),
        (LedgerClientRole::Coordinator, "coordinator"),
    ] {
        let role_grant = LedgerChannelGrant::new(LedgerGeneration::new([8; 16]), 9, role).unwrap();
        let value = commitment(role_grant, 1, 2);
        assert_eq!(
            decode_commitment_row(&commitment_row(&value, role_name)).unwrap(),
            value
        );
    }
}

fn write_frame(path: &Path, payload: &[u8], checksum: [u8; 32]) {
    std::fs::write(
        path,
        [
            CHECKPOINT_MAGIC.as_slice(),
            &(payload.len() as u32).to_be_bytes(),
            payload,
            &checksum,
        ]
        .concat(),
    )
    .unwrap();
}

#[test]
fn checkpoint_loader_rejects_invalid_frames_and_state_transitions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("checkpoints.log");
    initialize(&path).unwrap();

    for length in [0_u32, (MAX_CHECKPOINT_FRAME_BYTES as u32) + 1] {
        std::fs::write(&path, [CHECKPOINT_MAGIC.as_slice(), &length.to_be_bytes()].concat()).unwrap();
        assert!(read_records(&path)
            .unwrap_err()
            .to_string()
            .contains("frame length is invalid"));
    }
    write_frame(&path, b"invalid", [0; 32]);
    assert!(read_records(&path)
        .unwrap_err()
        .to_string()
        .contains("checksum is corrupt"));
    let payload = b"invalid";
    write_frame(&path, payload, *blake3::hash(payload).as_bytes());
    assert!(read_records(&path)
        .unwrap_err()
        .to_string()
        .contains("decode commitment checkpoint"));

    let producer = grant(9, 10);
    for (record, message) in [
        (
            CheckpointRecord {
                checkpoint_sequence: 2,
                commitments: vec![commitment(producer, 1, 1)],
            },
            "sequence is not contiguous",
        ),
        (
            CheckpointRecord {
                checkpoint_sequence: 1,
                commitments: Vec::new(),
            },
            "invalid batch size",
        ),
        (
            CheckpointRecord {
                checkpoint_sequence: 1,
                commitments: vec![commitment(producer, 2, 1)],
            },
            "breaks a producer chain",
        ),
    ] {
        std::fs::write(&path, []).unwrap();
        append(&path, &record).unwrap();
        assert!(load(&path).err().unwrap().to_string().contains(message));
    }

    let other = grant(10, 11);
    std::fs::write(&path, []).unwrap();
    append(
        &path,
        &CheckpointRecord {
            checkpoint_sequence: 1,
            commitments: vec![commitment(producer, 1, 1), commitment(other, 1, 1)],
        },
    )
    .unwrap();
    assert!(load(&path)
        .err()
        .unwrap()
        .to_string()
        .contains("repeats a global sequence"));
}

#[tokio::test]
async fn commitment_channel_enforces_handshake_ids_and_executes_all_commands() {
    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-channel").await.unwrap();
    let producer = grant(11, 12);
    let (client, server) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(serve(Arc::clone(&authority), server, producer));
    let (requests, responses) =
        ipc_channel::channel_from_std::<CommitmentClientMessage, CommitmentServerMessage>(client).unwrap();
    requests
        .send(CommitmentClientMessage::Hello {
            hello: LedgerHello::for_grant(&producer),
        })
        .await
        .unwrap();
    assert!(matches!(
        responses.recv().await.unwrap(),
        CommitmentServerMessage::Welcome { .. }
    ));

    requests
        .send(CommitmentClientMessage::Request {
            request_id: 1,
            command: CommitmentCommand::Reserve {
                producer_sequence: 1,
                event_kind: "net_event".into(),
                event_hash: [4; 32],
                previous_hash: ZERO_COMMITMENT_HASH,
            },
        })
        .await
        .unwrap();
    let global_sequence = match responses.recv().await.unwrap() {
        CommitmentServerMessage::Response {
            request_id: 1,
            reply: CommitmentReply::Reserved { global_sequence },
        } => global_sequence,
        response => panic!("unexpected reserve response: {response:?}"),
    };

    requests
        .send(CommitmentClientMessage::Request {
            request_id: 2,
            command: CommitmentCommand::Cancel {
                global_sequence: global_sequence + 1,
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        responses.recv().await.unwrap(),
        CommitmentServerMessage::Response {
            request_id: 2,
            reply: CommitmentReply::Failed { .. },
        }
    ));

    let reserved =
        LedgerCommitment::new(producer, 1, global_sequence, "net_event", [4; 32], ZERO_COMMITMENT_HASH).unwrap();
    requests
        .send(CommitmentClientMessage::Request {
            request_id: 3,
            command: CommitmentCommand::Anchor {
                commitments: vec![reserved],
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        responses.recv().await.unwrap(),
        CommitmentServerMessage::Response {
            request_id: 3,
            reply: CommitmentReply::Anchored { checkpoint_sequence: 1 },
        }
    ));

    let anchored_hash = authority.state.lock().await.anchored[&ProducerKey::from_grant(producer)].hash;
    requests
        .send(CommitmentClientMessage::Request {
            request_id: 4,
            command: CommitmentCommand::Reserve {
                producer_sequence: 2,
                event_kind: "net_event".into(),
                event_hash: [5; 32],
                previous_hash: anchored_hash,
            },
        })
        .await
        .unwrap();
    let tail = match responses.recv().await.unwrap() {
        CommitmentServerMessage::Response {
            request_id: 4,
            reply: CommitmentReply::Reserved { global_sequence },
        } => global_sequence,
        response => panic!("unexpected reserve response: {response:?}"),
    };
    requests
        .send(CommitmentClientMessage::Request {
            request_id: 5,
            command: CommitmentCommand::Cancel { global_sequence: tail },
        })
        .await
        .unwrap();
    assert!(matches!(
        responses.recv().await.unwrap(),
        CommitmentServerMessage::Response {
            request_id: 5,
            reply: CommitmentReply::Canceled,
        }
    ));

    requests
        .send(CommitmentClientMessage::Hello {
            hello: LedgerHello::for_grant(&producer),
        })
        .await
        .unwrap();
    assert!(serving
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("repeated hello"));

    let (client, server) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(serve(authority, server, producer));
    let (requests, responses) =
        ipc_channel::channel_from_std::<CommitmentClientMessage, CommitmentServerMessage>(client).unwrap();
    requests
        .send(CommitmentClientMessage::Request {
            request_id: 0,
            command: CommitmentCommand::Cancel { global_sequence: 1 },
        })
        .await
        .unwrap();
    drop(responses);
    assert!(serving.await.unwrap().unwrap_err().to_string().contains("before hello"));

    let root = tempfile::tempdir().unwrap();
    let authority = CommitmentAuthority::open(root.path(), "session-zero-id").await.unwrap();
    let (client, server) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(serve(authority, server, producer));
    let (requests, responses) =
        ipc_channel::channel_from_std::<CommitmentClientMessage, CommitmentServerMessage>(client).unwrap();
    requests
        .send(CommitmentClientMessage::Hello {
            hello: LedgerHello::for_grant(&producer),
        })
        .await
        .unwrap();
    assert!(matches!(
        responses.recv().await.unwrap(),
        CommitmentServerMessage::Welcome { .. }
    ));
    requests
        .send(CommitmentClientMessage::Request {
            request_id: 0,
            command: CommitmentCommand::Cancel { global_sequence: 1 },
        })
        .await
        .unwrap();
    assert!(serving
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("request id must not be zero"));
}
