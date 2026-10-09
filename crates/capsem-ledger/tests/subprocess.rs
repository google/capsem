use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime};

use capsem_foundation::ipc_channel;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_logger::ledger_protocol::{LedgerClientMessage, LedgerCommand, LedgerReply, LedgerServerMessage};
use capsem_logger::{Decision, NetEvent, WriteOp};
use capsem_proto::ledger::{
    LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerHello, LedgerOutcome, LedgerRequest,
};
use capsem_proto::ledger_commitment::{LedgerCommitment, ZERO_COMMITMENT_HASH};
use capsem_proto::ledger_control::{
    decode_ledger_control_event, encode_ledger_control_request, LedgerControlEvent, LedgerControlRequest,
    LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS,
};

type ControlSender = DescriptorSender<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;
type ControlReceiver = DescriptorReceiver<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;

const GENERATION: LedgerGeneration = LedgerGeneration::new([6; 16]);

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn take(&mut self) -> Child {
        self.0.take().expect("child present")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn event(receiver: &ControlReceiver) -> LedgerControlEvent {
    let frame = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("ledger control event timed out")
        .expect("ledger control event");
    assert!(frame.fds.is_empty());
    decode_ledger_control_event(&frame.bytes).expect("valid ledger control event")
}

fn body_event() -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some("0123456789ab".into()),
        timestamp: SystemTime::UNIX_EPOCH,
        domain: "confined-zstd.test".into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("POST".into()),
        path: Some("/archive".into()),
        query: None,
        status_code: Some(200),
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 1,
        matched_rule: None,
        request_headers: Some("content-type: application/json".into()),
        response_headers: None,
        request_body: Some(br#"{"body":"written after confinement"}"#.to_vec()),
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

#[tokio::test]
async fn executable_opens_one_ledger_and_reports_durable_stop() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let (coordinator, child_control) = UnixStream::pair().unwrap();
    let mut child = ChildGuard(Some(
        Command::new(env!("CARGO_BIN_EXE_capsem-ledger"))
            .arg("--parent-pid")
            .arg(std::process::id().to_string())
            .arg("--database")
            .arg(&database)
            .arg("--generation")
            .arg("06060606060606060606060606060606")
            .env("CAPSEM_LEDGER_SECRET", "must-not-survive")
            .stdin(Stdio::from(OwnedFd::from(child_control)))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let requests = ControlSender::new(coordinator.try_clone().unwrap()).unwrap();
    let events = ControlReceiver::new(coordinator).unwrap();
    assert_eq!(
        event(&events).await,
        LedgerControlEvent::Ready { generation: GENERATION }
    );
    #[cfg(target_os = "linux")]
    {
        let environ = std::fs::read(format!("/proc/{}/environ", child.0.as_ref().unwrap().id())).unwrap();
        assert!(!environ
            .windows(b"CAPSEM_LEDGER_SECRET".len())
            .any(|value| value == b"CAPSEM_LEDGER_SECRET"));
    }
    let grant = LedgerChannelGrant::new(GENERATION, 1, LedgerClientRole::VmOwner).unwrap();
    let (client, ledger_end) = UnixStream::pair().unwrap();
    requests
        .send(
            &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
            &[ledger_end.as_raw_fd()],
        )
        .await
        .unwrap();
    assert_eq!(
        event(&events).await,
        LedgerControlEvent::Adopted {
            generation: GENERATION,
            client_id: 1,
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
    let admitted = body_event();
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
        event(&events).await,
        LedgerControlEvent::Stopped { generation: GENERATION }
    );

    let child = child.take();
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "ledger worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(database.is_file());
    let archive_dir = database.with_extension("bodies");
    let generation = std::fs::read_dir(&archive_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("cbl"))
        .expect("one archive generation");
    let archive = std::fs::read(generation).unwrap();
    assert_eq!(
        archive[capsem_archive::FILE_HEADER_BYTES + 4],
        capsem_archive::CODEC_ZSTD,
        "body admitted after worker readiness must use the confined zstd codec"
    );
}

#[tokio::test]
async fn executable_refuses_a_symlinked_session_before_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let actual = dir.path().join("actual");
    let linked = dir.path().join("linked");
    std::fs::create_dir(&actual).unwrap();
    std::os::unix::fs::symlink(&actual, &linked).unwrap();
    let (coordinator, child_control) = UnixStream::pair().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_capsem-ledger"))
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--database")
        .arg(linked.join("session.db"))
        .arg("--generation")
        .arg("06060606060606060606060606060606")
        .stdin(Stdio::from(OwnedFd::from(child_control)))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(coordinator);
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || child.wait_with_output()),
    )
    .await
    .expect("unconfined ledger worker did not exit")
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("open session ledger") || stderr.contains("confine ledger worker before readiness"),
        "unexpected refusal: {}",
        stderr
    );
}
