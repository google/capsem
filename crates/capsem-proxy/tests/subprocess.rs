use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::proxy_control::{
    decode_proxy_control_event, encode_proxy_control_request, ProxyCapability, ProxyChannelGrant, ProxyControlEvent,
    ProxyControlRejection, ProxyControlRequest, ProxyGeneration, PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS,
};

type ControlSender = DescriptorSender<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlReceiver = DescriptorReceiver<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;

const GENERATION: ProxyGeneration = ProxyGeneration::new([7; 16]);

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

async fn event(receiver: &ControlReceiver) -> ProxyControlEvent {
    let frame = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("proxy control event timed out")
        .expect("proxy control event");
    assert!(frame.fds.is_empty());
    decode_proxy_control_event(&frame.bytes).expect("valid proxy control event")
}

fn spawn() -> (ChildGuard, ControlSender, ControlReceiver) {
    let (coordinator, child_control) = UnixStream::pair().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_capsem-proxy"))
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--generation")
        .arg("07070707070707070707070707070707")
        .env("CAPSEM_PROXY_SECRET", "must-not-survive")
        .stdin(Stdio::from(OwnedFd::from(child_control)))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = ControlSender::new(coordinator.try_clone().unwrap()).unwrap();
    let events = ControlReceiver::new(coordinator).unwrap();
    (ChildGuard(Some(child)), requests, events)
}

#[tokio::test]
async fn worker_confines_before_ready_and_adopts_scoped_descriptors() {
    let (mut child, requests, events) = spawn();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Ready { generation: GENERATION }
    );

    #[cfg(target_os = "linux")]
    {
        let environ = std::fs::read(format!("/proc/{}/environ", child.0.as_ref().unwrap().id())).unwrap();
        assert!(!environ
            .windows(b"CAPSEM_PROXY_SECRET".len())
            .any(|value| value == b"CAPSEM_PROXY_SECRET"));
    }

    let mut peers = Vec::new();
    for (index, capability) in [
        ProxyCapability::Traffic,
        ProxyCapability::Upstream,
        ProxyCapability::Credential,
        ProxyCapability::Ledger,
        ProxyCapability::PrivateNames,
        ProxyCapability::Mcp,
        ProxyCapability::Telemetry,
    ]
    .into_iter()
    .enumerate()
    {
        let grant_id = index as u64 + 1;
        let (peer, granted) = UnixStream::pair().unwrap();
        let grant = ProxyChannelGrant::new(GENERATION, grant_id, capability).unwrap();
        requests
            .send(
                &encode_proxy_control_request(ProxyControlRequest::Attach(grant)),
                &[granted.as_raw_fd()],
            )
            .await
            .unwrap();
        drop(granted);
        assert_eq!(
            event(&events).await,
            ProxyControlEvent::Adopted {
                generation: GENERATION,
                grant_id,
            }
        );
        peers.push(peer);
    }

    let (duplicate_peer, duplicate) = UnixStream::pair().unwrap();
    let duplicate_grant = ProxyChannelGrant::new(GENERATION, 8, ProxyCapability::Ledger).unwrap();
    requests
        .send(
            &encode_proxy_control_request(ProxyControlRequest::Attach(duplicate_grant)),
            &[duplicate.as_raw_fd()],
        )
        .await
        .unwrap();
    drop(duplicate);
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Rejected {
            generation: GENERATION,
            grant_id: 8,
            reason: ProxyControlRejection::DuplicateCapability,
        }
    );
    drop(duplicate_peer);

    requests
        .send(
            &encode_proxy_control_request(ProxyControlRequest::Shutdown { generation: GENERATION }),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Stopped { generation: GENERATION }
    );
    drop(peers);

    let child = child.take();
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "proxy worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn stale_generation_terminates_worker_and_releases_grants() {
    let (mut child, requests, events) = spawn();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Ready { generation: GENERATION }
    );
    let stale = ProxyGeneration::new([8; 16]);
    let (peer, granted) = UnixStream::pair().unwrap();
    let grant = ProxyChannelGrant::new(stale, 1, ProxyCapability::Upstream).unwrap();
    requests
        .send(
            &encode_proxy_control_request(ProxyControlRequest::Attach(grant)),
            &[granted.as_raw_fd()],
        )
        .await
        .unwrap();
    drop(granted);

    let child = child.take();
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || child.wait_with_output()),
    )
    .await
    .expect("stale-generation worker did not exit")
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale generation"));
    drop(peer);
}
