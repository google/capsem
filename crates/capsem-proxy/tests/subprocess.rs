use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use capsem_proto::proxy_control::{
    decode_proxy_control_event, encode_proxy_control_request, ProxyCapability, ProxyChannelGrant, ProxyControlEvent,
    ProxyControlRejection, ProxyControlRequest, ProxyGeneration, PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS,
};
use capsem_proto::proxy_policy::{ProxyPolicyRequest, ProxyPolicyResponse};

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
async fn worker_rejects_http_traffic_until_runtime_is_ready() {
    let (mut child, requests, events) = spawn();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Ready { generation: GENERATION }
    );
    let (mut peer, granted) = UnixStream::pair().unwrap();
    let grant = ProxyChannelGrant::new(GENERATION, 1, ProxyCapability::HttpTraffic).unwrap();
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
        ProxyControlEvent::Rejected {
            generation: GENERATION,
            grant_id: 1,
            reason: ProxyControlRejection::NotReady,
        }
    );
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);

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
    let child = child.take();
    assert!(tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
        .status
        .success());
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
        ProxyCapability::Upstream,
        ProxyCapability::Credential,
        ProxyCapability::PrivateNames,
        ProxyCapability::Mcp,
        ProxyCapability::Telemetry,
        ProxyCapability::Policy,
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
    let duplicate_grant = ProxyChannelGrant::new(GENERATION, 10, ProxyCapability::Telemetry).unwrap();
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
            grant_id: 10,
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
async fn ledger_grant_is_authenticated_before_adoption() {
    let (mut child, requests, events) = spawn();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Ready { generation: GENERATION }
    );
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(4);
    let event_pump = tokio::spawn(async move {
        while let Ok(frame) = events.recv().await {
            let event = decode_proxy_control_event(&frame.bytes).unwrap();
            if event_tx.send(event).await.is_err() {
                return;
            }
        }
    });
    let (server_stream, granted) = UnixStream::pair().unwrap();
    let ledger_grant = LedgerChannelGrant::new(LedgerGeneration::new([4; 16]), 17, LedgerClientRole::Proxy).unwrap();
    let grant = ProxyChannelGrant::with_ledger(GENERATION, 1, ledger_grant).unwrap();
    requests
        .send(
            &encode_proxy_control_request(ProxyControlRequest::Attach(grant)),
            &[granted.as_raw_fd()],
        )
        .await
        .unwrap();
    drop(granted);

    assert!(tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .is_err());
    let ledger_dir = tempfile::tempdir().unwrap();
    let ledger = std::sync::Arc::new(
        capsem_logger::ledger_server::LedgerServer::open(&ledger_dir.path().join("session.db")).unwrap(),
    );
    let serving = tokio::spawn(async move { ledger.serve_client(server_stream, ledger_grant).await });
    let adopted = match tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
        Ok(Some(event)) => event,
        _ => {
            let child = child.take();
            let output = tokio::task::spawn_blocking(move || child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
            panic!(
                "proxy ledger adoption failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    };
    assert_eq!(
        adopted,
        ProxyControlEvent::Adopted {
            generation: GENERATION,
            grant_id: 1,
        }
    );

    requests
        .send(
            &encode_proxy_control_request(ProxyControlRequest::Shutdown { generation: GENERATION }),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .unwrap()
            .unwrap(),
        ProxyControlEvent::Stopped { generation: GENERATION }
    );
    event_pump.await.unwrap();
    assert!(serving.await.unwrap().is_ok());
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

#[tokio::test]
async fn policy_capability_compiles_exact_bytes_and_rejects_bad_revisions() {
    let (mut child, requests, events) = spawn();
    assert_eq!(
        event(&events).await,
        ProxyControlEvent::Ready { generation: GENERATION }
    );
    let (peer, granted) = UnixStream::pair().unwrap();
    let grant = ProxyChannelGrant::new(GENERATION, 1, ProxyCapability::Policy).unwrap();
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
            grant_id: 1,
        }
    );

    let (policy_tx, policy_rx) =
        capsem_foundation::ipc_channel::channel_from_std::<ProxyPolicyRequest, ProxyPolicyResponse>(peer).unwrap();
    let active_policy = br#"
[network]
[network.dns]
upstreams = ["127.0.0.1:5353"]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "worker.example"'
[corp_rules]
[mcp.server_enabled]
local = false
"#;
    policy_tx
        .send(ProxyPolicyRequest::apply(1, active_policy.to_vec()))
        .await
        .unwrap();
    assert_eq!(
        policy_rx.recv().await.unwrap(),
        ProxyPolicyResponse::Applied {
            request_id: 1,
            active_policy_digest: capsem_core::net::policy_config::active_policy_digest(active_policy),
        }
    );

    policy_tx
        .send(ProxyPolicyRequest::apply(2, b"[network]\nunknown = true".to_vec()))
        .await
        .unwrap();
    let ProxyPolicyResponse::Rejected { request_id, error } = policy_rx.recv().await.unwrap() else {
        panic!("invalid active policy was accepted")
    };
    assert_eq!(request_id, 2);
    assert!(error.contains("parse active policy"), "{error}");

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
