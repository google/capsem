use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::ledger::LedgerGeneration;
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
