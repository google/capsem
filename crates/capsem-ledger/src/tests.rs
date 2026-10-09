use std::os::fd::AsRawFd;
use std::time::SystemTime;

use capsem_foundation::ipc_channel;
use capsem_logger::ledger_protocol::{LedgerClientMessage, LedgerCommand, LedgerReply, LedgerServerMessage};
use capsem_logger::{Decision, NetEvent, WriteOp};
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerHello, LedgerOutcome, LedgerRequest};
use capsem_proto::ledger_commitment::{LedgerCommitment, ZERO_COMMITMENT_HASH};
use capsem_proto::ledger_control::{
    decode_ledger_control_event, encode_ledger_control_request, LedgerControlEvent, LedgerControlRequest,
};

use super::*;

const GENERATION: LedgerGeneration = LedgerGeneration::new([5; 16]);

fn event() -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some("0123456789ab".into()),
        timestamp: SystemTime::UNIX_EPOCH,
        domain: "worker.test".into(),
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

fn commitment(grant: LedgerChannelGrant, event: &WriteOp) -> LedgerCommitment {
    LedgerCommitment::new(
        grant,
        1,
        1,
        event.kind(),
        capsem_logger::commitment_event_hash(event).unwrap(),
        ZERO_COMMITMENT_HASH,
    )
    .unwrap()
}

async fn control_event(receiver: &ControlReceiver) -> LedgerControlEvent {
    let frame = receiver.recv().await.unwrap();
    assert!(frame.fds.is_empty());
    decode_ledger_control_event(&frame.bytes).unwrap()
}

#[tokio::test]
async fn worker_adopts_one_client_flushes_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&database).unwrap());
    let (coordinator, worker) = UnixStream::pair().unwrap();
    let requests = ControlSender::new(coordinator.try_clone().unwrap()).unwrap();
    let events = ControlReceiver::new(coordinator).unwrap();
    let task = tokio::spawn(run_control(worker, server, GENERATION, 1));
    assert_eq!(
        control_event(&events).await,
        LedgerControlEvent::Ready { generation: GENERATION }
    );

    let grant = LedgerChannelGrant::new(GENERATION, 11, LedgerClientRole::VmOwner).unwrap();
    let (client, ledger_end) = UnixStream::pair().unwrap();
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
            &[ledger_end.as_raw_fd()],
        )
        .await
        .unwrap();
    assert_eq!(
        control_event(&events).await,
        LedgerControlEvent::Adopted {
            generation: GENERATION,
            client_id: 11,
        }
    );
    drop(ledger_end);

    let (sender, receiver) = ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(client).unwrap();
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
    let admitted = event();
    for (request_id, operation) in [
        (
            1,
            LedgerCommand::Admit {
                commitment: commitment(grant, &admitted),
                event: Box::new(admitted),
            },
        ),
        (2, LedgerCommand::Flush),
    ] {
        sender
            .send(LedgerClientMessage::Request {
                request: LedgerRequest::new(request_id, operation).unwrap(),
            })
            .await
            .unwrap();
        let LedgerServerMessage::Response { response } = receiver.recv().await.unwrap() else {
            panic!("expected ledger response")
        };
        assert!(matches!(response.outcome(), LedgerOutcome::Success { .. }));
        assert!(matches!(
            response.outcome(),
            LedgerOutcome::Success {
                reply: LedgerReply::Accepted { .. } | LedgerReply::Durable { .. }
            }
        ));
    }

    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Shutdown { generation: GENERATION }),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        control_event(&events).await,
        LedgerControlEvent::Stopped { generation: GENERATION }
    );
    task.await.unwrap().unwrap();

    let reader = capsem_logger::DbHandle::open_external_reader(&database).unwrap();
    reader.ready().await.unwrap();
    let rows = reader.query("SELECT domain FROM net_events", &[]).await.unwrap();
    assert!(rows.contains("worker.test"));
}

#[tokio::test]
async fn duplicate_client_id_is_rejected_without_adopting_its_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(LedgerServer::open(&dir.path().join("session.db")).unwrap());
    let (coordinator, worker) = UnixStream::pair().unwrap();
    let requests = ControlSender::new(coordinator.try_clone().unwrap()).unwrap();
    let events = ControlReceiver::new(coordinator).unwrap();
    let task = tokio::spawn(run_control(worker, server, GENERATION, 2));
    assert!(matches!(control_event(&events).await, LedgerControlEvent::Ready { .. }));

    let grant = LedgerChannelGrant::new(GENERATION, 12, LedgerClientRole::Reader).unwrap();
    let (first_client, first_worker) = UnixStream::pair().unwrap();
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
            &[first_worker.as_raw_fd()],
        )
        .await
        .unwrap();
    assert!(matches!(
        control_event(&events).await,
        LedgerControlEvent::Adopted { .. }
    ));
    drop(first_worker);

    let (_duplicate_client, duplicate_worker) = UnixStream::pair().unwrap();
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
            &[duplicate_worker.as_raw_fd()],
        )
        .await
        .unwrap();
    assert!(matches!(
        control_event(&events).await,
        LedgerControlEvent::Rejected {
            reason: LedgerControlRejection::DuplicateClient,
            ..
        }
    ));
    drop(duplicate_worker);

    drop(first_client);
    assert_eq!(
        control_event(&events).await,
        LedgerControlEvent::Closed {
            generation: GENERATION,
            client_id: 12,
            reason: LedgerClientCloseReason::ProtocolError,
        }
    );
    let (_replayed_client, replayed_worker) = UnixStream::pair().unwrap();
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
            &[replayed_worker.as_raw_fd()],
        )
        .await
        .unwrap();
    assert!(matches!(
        control_event(&events).await,
        LedgerControlEvent::Rejected {
            reason: LedgerControlRejection::DuplicateClient,
            ..
        }
    ));
    drop(replayed_worker);
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Shutdown { generation: GENERATION }),
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(
        control_event(&events).await,
        LedgerControlEvent::Stopped { .. }
    ));
    task.await.unwrap().unwrap();
}

#[test]
fn generation_parser_is_exact_and_nonzero() {
    assert_eq!(
        parse_generation("01010101010101010101010101010101").unwrap().as_bytes(),
        [1; 16]
    );
    for invalid in [
        "",
        "00",
        "00000000000000000000000000000000",
        "0101010101010101010101010101010G",
        "0101010101010101010101010101010A",
    ] {
        assert!(parse_generation(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn database_path_is_one_absolute_session_directory() {
    assert_eq!(
        session_directory(Path::new("/run/capsem/sessions/one/session.db")).unwrap(),
        Path::new("/run/capsem/sessions/one")
    );
    for invalid in ["session.db", "/session.db", "/run/capsem/other.db"] {
        assert!(session_directory(Path::new(invalid)).is_err(), "accepted {invalid}");
    }
}

#[test]
fn inherited_authority_is_removed_and_confinement_precedes_readiness() {
    let source = include_str!("main.rs");
    let clear = source.find("clear_inherited_environment();").unwrap();
    let close = source.find("close_inherited_descriptors()?").unwrap();
    let telemetry = source.find("telemetry::init").unwrap();
    let confine = source.find("worker_sandbox::confine").unwrap();
    let attest = source.find("attest_confinement(&session_dir").unwrap();
    let run = source.find("runtime.block_on(run_control").unwrap();
    assert!(clear < close && close < telemetry && telemetry < confine && confine < attest && attest < run);
}
