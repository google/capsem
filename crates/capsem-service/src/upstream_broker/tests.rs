use std::collections::BTreeMap;
use std::fs::File;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::fs::{symlink, PermissionsExt as _};
use std::path::PathBuf;

use capsem_core::net::policy::{UpstreamOverride, UpstreamOverrideProtocol};
use capsem_core::net::policy_config::{ActivePolicyFile, SettingsFile};
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_response, encode_upstream_grant_request, ProxyTrafficService, UpstreamGrantResponse,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;
use crate::instance::WorkerAuthority;
use crate::ledger_worker::LedgerClient;

struct TestClient {
    requests: WireSender,
    responses: WireReceiver,
}

impl TestClient {
    fn new(socket: UnixStream) -> Self {
        Self {
            requests: WireSender::new(socket.try_clone().unwrap()).unwrap(),
            responses: WireReceiver::new(socket).unwrap(),
        }
    }

    async fn send(&self, request: &UpstreamGrantRequest) {
        let bytes = encode_upstream_grant_request(request).unwrap();
        self.requests.send(&bytes, &[]).await.unwrap();
    }

    async fn request_with_descriptor(
        &self,
        request: &UpstreamGrantRequest,
        descriptor: &impl std::os::fd::AsRawFd,
    ) -> UpstreamGrantResponse {
        let bytes = encode_upstream_grant_request(request).unwrap();
        self.requests.send(&bytes, &[descriptor.as_raw_fd()]).await.unwrap();
        let frame = self.responses.recv().await.unwrap();
        assert!(frame.fds.is_empty());
        decode_upstream_grant_response(&frame.bytes).unwrap()
    }

    async fn request(&self, request: &UpstreamGrantRequest) -> (UpstreamGrantResponse, Vec<OwnedFd>) {
        self.send(request).await;
        let frame = self.responses.recv().await.unwrap();
        let response = decode_upstream_grant_response(&frame.bytes).unwrap();
        assert_eq!(frame.fds.len(), response.expected_descriptor_count());
        (response, frame.fds)
    }
}

#[tokio::test]
async fn typed_guest_traffic_is_adopted_by_the_registered_proxy_generation() {
    let policy = test_policy("policy-a", None, vec![]);
    let (pending, _publisher) = PendingBroker::pair(policy).unwrap();
    let pending = pending.with_proxy(crate::proxy_worker::ProxyWorker::test_stub());
    let client = TestClient::new(pending.worker);
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(run(
        pending.coordinator,
        pending.initial_policy,
        pending.updates,
        authority.grant(),
        pending.session_dir,
        pending.ledger,
        pending.proxy,
    ));
    let (traffic, _peer) = UnixStream::pair().unwrap();

    let response = client
        .request_with_descriptor(
            &UpstreamGrantRequest::AttachProxyTraffic {
                request_id: 1,
                service: ProxyTrafficService::Http,
            },
            &traffic,
        )
        .await;
    assert_eq!(response, UpstreamGrantResponse::ProxyTrafficAdopted { request_id: 1 });

    authority.revoke();
    assert!(task.await.unwrap().unwrap_err().contains("revoked"));
}

fn test_policy(digest: &str, tcp_override: Option<SocketAddr>, dns_upstreams: Vec<SocketAddr>) -> Arc<BrokerPolicy> {
    let active = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    let mut runtime = active.compile_runtime().unwrap();
    runtime.dns_upstreams = dns_upstreams;
    runtime.network.upstream_overrides = tcp_override.map_or_else(BTreeMap::new, |dial| {
        BTreeMap::from([(
            "policy.example:443".into(),
            UpstreamOverride {
                dial: dial.to_string(),
                protocol: UpstreamOverrideProtocol::Http,
            },
        )])
    });
    BrokerPolicy::new(policy_digest(digest), Arc::new(runtime))
}

fn policy_digest(label: &str) -> String {
    format!("blake3:{}", blake3::hash(label.as_bytes()).to_hex())
}

fn start_broker(
    policy: Arc<BrokerPolicy>,
) -> (
    TestClient,
    PolicyPublisher,
    WorkerAuthority,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (pending, publisher) = PendingBroker::pair(policy).unwrap();
    let client = TestClient::new(pending.worker);
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(run(
        pending.coordinator,
        pending.initial_policy,
        pending.updates,
        authority.grant(),
        pending.session_dir,
        pending.ledger,
        pending.proxy,
    ));
    (client, publisher, authority, task)
}

fn start_broker_with_ledger(
    policy: Arc<BrokerPolicy>,
    ledger: LedgerClient,
) -> (
    TestClient,
    PolicyPublisher,
    WorkerAuthority,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (pending, publisher) = PendingBroker::pair(policy).unwrap();
    let pending = pending.with_ledger(ledger);
    let client = TestClient::new(pending.worker);
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(run(
        pending.coordinator,
        pending.initial_policy,
        pending.updates,
        authority.grant(),
        pending.session_dir,
        pending.ledger,
        pending.proxy,
    ));
    (client, publisher, authority, task)
}

fn start_broker_for_session(
    policy: Arc<BrokerPolicy>,
    session_dir: PathBuf,
) -> (
    TestClient,
    PolicyPublisher,
    WorkerAuthority,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (pending, publisher) = PendingBroker::pair_for_session(policy, session_dir).unwrap();
    let client = TestClient::new(pending.worker);
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(run(
        pending.coordinator,
        pending.initial_policy,
        pending.updates,
        authority.grant(),
        pending.session_dir,
        pending.ledger,
        pending.proxy,
    ));
    (client, publisher, authority, task)
}

#[tokio::test]
async fn ledger_grant_is_exact_single_use_and_releases_the_broker_copy_after_adoption() {
    let grant = LedgerChannelGrant::new(LedgerGeneration::new([0x4d; 16]), 77, LedgerClientRole::VmOwner).unwrap();
    let (ledger, worker_peer) = LedgerClient::test_pair(grant).unwrap();
    let (client, _publisher, authority, task) = start_broker_with_ledger(test_policy("policy-a", None, vec![]), ledger);

    let (response, mut fds) = client
        .request(&UpstreamGrantRequest::OpenLedger { request_id: 1 })
        .await;
    assert_eq!(response, UpstreamGrantResponse::LedgerGranted { request_id: 1, grant });
    let granted = UnixStream::from(fds.pop().unwrap());
    client
        .send(&UpstreamGrantRequest::Adopted {
            grant_id: grant.client_id(),
        })
        .await;

    let (response, fds) = client
        .request(&UpstreamGrantRequest::OpenLedger { request_id: 2 })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 2,
            reason: UpstreamGrantDenial::InvalidResource,
        }
    );

    drop(granted);
    worker_peer.set_nonblocking(true).unwrap();
    let mut worker_peer = tokio::net::UnixStream::from_std(worker_peer).unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), worker_peer.read(&mut byte))
            .await
            .expect("ledger peer observes descriptor close")
            .unwrap(),
        0
    );
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn ledger_request_without_a_coordinator_grant_is_denied() {
    let (client, _publisher, authority, task) = start_broker(test_policy("policy-a", None, vec![]));
    let (response, fds) = client
        .request(&UpstreamGrantRequest::OpenLedger { request_id: 1 })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 1,
            reason: UpstreamGrantDenial::NotConfigured,
        }
    );
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn policy_updates_do_not_revoke_a_pending_ledger_grant() {
    let grant = LedgerChannelGrant::new(LedgerGeneration::new([0x37; 16]), 91, LedgerClientRole::VmOwner).unwrap();
    let (ledger, worker_peer) = LedgerClient::test_pair(grant).unwrap();
    let (client, publisher, authority, task) = start_broker_with_ledger(test_policy("policy-a", None, vec![]), ledger);
    let (response, mut fds) = client
        .request(&UpstreamGrantRequest::OpenLedger { request_id: 1 })
        .await;
    assert_eq!(response, UpstreamGrantResponse::LedgerGranted { request_id: 1, grant });

    publisher.publish(test_policy("policy-b", None, vec![])).await.unwrap();
    client
        .send(&UpstreamGrantRequest::Adopted {
            grant_id: grant.client_id(),
        })
        .await;

    drop(fds.pop().unwrap());
    worker_peer.set_nonblocking(true).unwrap();
    let mut worker_peer = tokio::net::UnixStream::from_std(worker_peer).unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), worker_peer.read(&mut byte))
            .await
            .expect("ledger peer observes descriptor close after policy update")
            .unwrap(),
        0
    );
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn guest_mode_requests_are_confined_to_the_session_share() {
    let session = tempfile::tempdir().unwrap();
    let guest = session.path().join(capsem_core::GUEST_SHARE_DIR);
    std::fs::create_dir_all(guest.join("workspace/nested")).unwrap();
    std::fs::write(guest.join("workspace/nested/tool"), b"tool").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    std::fs::write(&secret, b"secret").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&secret, guest.join("workspace/link")).unwrap();

    let (client, _publisher, authority, task) =
        start_broker_for_session(test_policy("policy-a", None, Vec::new()), session.path().to_path_buf());
    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestMode {
            request_id: 1,
            relative_path: b"workspace/nested/tool".to_vec(),
            mode: 0o751,
        })
        .await;
    assert_eq!(response, UpstreamGrantResponse::GuestModeSet { request_id: 1 });
    assert!(fds.is_empty());
    assert_eq!(
        std::fs::metadata(guest.join("workspace/nested/tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o751
    );

    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestMode {
            request_id: 2,
            relative_path: b"workspace/link".to_vec(),
            mode: 0o777,
        })
        .await;
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 2,
            reason: UpstreamGrantDenial::NotAllowed,
        }
    );
    assert!(fds.is_empty());
    assert_eq!(std::fs::metadata(secret).unwrap().permissions().mode() & 0o7777, 0o600);
    stop_broker(authority, task).await;
}

async fn resolve_override(client: &TestClient, request_id: u64) -> u64 {
    let (response, fds) = client
        .request(&UpstreamGrantRequest::ResolveTcp {
            request_id,
            protocol: UpstreamProtocol::Tls,
            host: "policy.example".into(),
            port: 443,
        })
        .await;
    assert!(fds.is_empty());
    let UpstreamGrantResponse::TcpResolved {
        request_id: response_id,
        selection_id,
        protocol,
        judged_ip,
        policy_digest: response_digest,
    } = response
    else {
        panic!("expected a resolved target, got {response:?}");
    };
    assert_eq!(response_id, request_id);
    assert_eq!(protocol, UpstreamProtocol::Http);
    assert_eq!(judged_ip, None);
    assert_eq!(response_digest, policy_digest("policy-a"));
    selection_id
}

async fn connect_selection(client: &TestClient, request_id: u64, selection_id: u64) -> (u64, OwnedFd) {
    let (response, mut fds) = client
        .request(&UpstreamGrantRequest::ConnectTcp {
            request_id,
            selection_id,
        })
        .await;
    let UpstreamGrantResponse::DescriptorGranted {
        request_id: response_id,
        grant_id,
        kind,
        policy_digest: response_digest,
    } = response
    else {
        panic!("expected a descriptor grant, got {response:?}");
    };
    assert_eq!(response_id, request_id);
    assert_eq!(kind, UpstreamDescriptorKind::Tcp);
    assert_eq!(response_digest, policy_digest("policy-a"));
    (grant_id, fds.pop().unwrap())
}

async fn stop_broker(authority: WorkerAuthority, task: tokio::task::JoinHandle<Result<(), String>>) {
    authority.revoke();
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("broker stops when authority is revoked")
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "worker authority revoked");
}

fn tcp_stream(descriptor: OwnedFd) -> tokio::net::TcpStream {
    let stream = std::net::TcpStream::from(descriptor);
    stream.set_nonblocking(true).unwrap();
    tokio::net::TcpStream::from_std(stream).unwrap()
}

#[tokio::test]
async fn trusted_override_grants_only_its_connected_tcp_stream() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]));

    let selection_id = resolve_override(&client, 1).await;
    let (grant_id, descriptor) = connect_selection(&client, 2, selection_id).await;
    let (mut upstream, peer) = listener.accept().await.unwrap();
    assert_eq!(peer.ip().to_string(), "127.0.0.1");
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;

    let mut granted = tcp_stream(descriptor);
    granted.write_all(b"brokered").await.unwrap();
    let mut received = [0_u8; 8];
    upstream.read_exact(&mut received).await.unwrap();
    assert_eq!(&received, b"brokered");

    client
        .send(&UpstreamGrantRequest::Release { resource_id: grant_id })
        .await;
    let mut byte = [0_u8; 1];
    let revoked = tokio::time::timeout(Duration::from_secs(1), upstream.read(&mut byte))
        .await
        .expect("release revokes the peer connection");
    assert!(
        matches!(revoked, Err(_) | Ok(0)),
        "peer stayed usable after release: {revoked:?}"
    );
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn tcp_selection_is_single_use() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]));
    let selection_id = resolve_override(&client, 1).await;
    let (grant_id, descriptor) = connect_selection(&client, 2, selection_id).await;
    let (_upstream, _) = listener.accept().await.unwrap();
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;

    let (response, fds) = client
        .request(&UpstreamGrantRequest::ConnectTcp {
            request_id: 3,
            selection_id,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 3,
            reason: UpstreamGrantDenial::InvalidResource,
        }
    );
    assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept())
        .await
        .is_err());
    client
        .send(&UpstreamGrantRequest::Release { resource_id: grant_id })
        .await;
    drop(descriptor);
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn unresolved_selection_count_is_bounded() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]));
    let mut selections = Vec::new();
    for request_id in 1..=MAX_SELECTIONS as u64 {
        selections.push(resolve_override(&client, request_id).await);
    }
    let request_id = MAX_SELECTIONS as u64 + 1;
    let (response, fds) = client
        .request(&UpstreamGrantRequest::ResolveTcp {
            request_id,
            protocol: UpstreamProtocol::Tls,
            host: "policy.example".into(),
            port: 443,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id,
            reason: UpstreamGrantDenial::Capacity,
        }
    );
    client
        .send(&UpstreamGrantRequest::Release {
            resource_id: selections[0],
        })
        .await;
    assert_ne!(resolve_override(&client, request_id + 1).await, 0);
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn dns_grant_uses_only_the_configured_index() {
    let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", None, vec![upstream.local_addr().unwrap()]));

    let (response, mut fds) = client
        .request(&UpstreamGrantRequest::OpenDns {
            request_id: 1,
            upstream_index: 0,
        })
        .await;
    let UpstreamGrantResponse::DescriptorGranted {
        grant_id,
        kind,
        policy_digest: response_digest,
        ..
    } = response
    else {
        panic!("expected a DNS descriptor grant, got {response:?}");
    };
    assert_eq!(kind, UpstreamDescriptorKind::DnsUdp);
    assert_eq!(response_digest, policy_digest("policy-a"));
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;
    let socket = std::net::UdpSocket::from(fds.pop().unwrap());
    socket.set_nonblocking(true).unwrap();
    let granted = tokio::net::UdpSocket::from_std(socket).unwrap();
    granted.send(b"query").await.unwrap();
    let mut received = [0_u8; 5];
    let (count, _peer) = upstream.recv_from(&mut received).await.unwrap();
    assert_eq!(&received[..count], b"query");

    let (denial, fds) = client
        .request(&UpstreamGrantRequest::OpenDns {
            request_id: 2,
            upstream_index: 1,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        denial,
        UpstreamGrantResponse::Denied {
            request_id: 2,
            reason: UpstreamGrantDenial::NotConfigured,
        }
    );
    client
        .send(&UpstreamGrantRequest::Release { resource_id: grant_id })
        .await;
    let (barrier, barrier_fds) = client
        .request(&UpstreamGrantRequest::OpenDns {
            request_id: 3,
            upstream_index: 1,
        })
        .await;
    assert!(barrier_fds.is_empty());
    assert!(matches!(
        barrier,
        UpstreamGrantResponse::Denied {
            request_id: 3,
            reason: UpstreamGrantDenial::NotConfigured
        }
    ));
    assert!(granted.send(b"after-release").await.is_err());
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn policy_publication_revokes_grants_and_invalidates_selections() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, publisher, authority, task) = start_broker(test_policy("policy-a", Some(address), vec![]));
    let live_selection = resolve_override(&client, 1).await;
    let (grant_id, descriptor) = connect_selection(&client, 2, live_selection).await;
    let (mut upstream, _) = listener.accept().await.unwrap();
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;
    let stale_selection = resolve_override(&client, 3).await;

    publisher
        .publish(test_policy("policy-b", Some(address), vec![]))
        .await
        .unwrap();
    let (denial, fds) = client
        .request(&UpstreamGrantRequest::ConnectTcp {
            request_id: 4,
            selection_id: stale_selection,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        denial,
        UpstreamGrantResponse::Denied {
            request_id: 4,
            reason: UpstreamGrantDenial::InvalidResource,
        }
    );
    let mut byte = [0_u8; 1];
    let revoked = tokio::time::timeout(Duration::from_secs(1), upstream.read(&mut byte))
        .await
        .expect("policy publication revokes an old grant");
    assert!(
        matches!(revoked, Err(_) | Ok(0)),
        "old policy grant stayed usable: {revoked:?}"
    );
    drop(descriptor);
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn generation_revocation_resets_every_live_grant() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]));
    let selection_id = resolve_override(&client, 1).await;
    let (grant_id, descriptor) = connect_selection(&client, 2, selection_id).await;
    let (mut upstream, _) = listener.accept().await.unwrap();
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;

    authority.revoke();
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("revoked generation stops its broker")
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "worker authority revoked");
    let mut byte = [0_u8; 1];
    let revoked = tokio::time::timeout(Duration::from_secs(1), upstream.read(&mut byte))
        .await
        .expect("generation revocation resets its granted connection");
    assert!(matches!(revoked, Err(_) | Ok(0)));
    drop(descriptor);
}

#[tokio::test]
async fn next_request_before_adoption_terminates_and_revokes_the_channel() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, _authority, task) =
        start_broker(test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]));
    let selection_id = resolve_override(&client, 1).await;
    let (_grant_id, descriptor) = connect_selection(&client, 2, selection_id).await;
    let (mut upstream, _) = listener.accept().await.unwrap();

    client
        .send(&UpstreamGrantRequest::Release {
            resource_id: selection_id,
        })
        .await;
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("was not adopted"), "{error}");
    let mut byte = [0_u8; 1];
    let revoked = tokio::time::timeout(Duration::from_secs(1), upstream.read(&mut byte))
        .await
        .expect("protocol violation revokes the unadopted grant");
    assert!(matches!(revoked, Err(_) | Ok(0)));
    drop(descriptor);
}

#[tokio::test]
async fn worker_supplied_descriptor_terminates_before_resolution() {
    let (client, _publisher, _authority, task) = start_broker(test_policy("policy-a", None, vec![]));
    let request = UpstreamGrantRequest::ResolveTcp {
        request_id: 1,
        protocol: UpstreamProtocol::Tls,
        host: "example.com".into(),
        port: 443,
    };
    let bytes = encode_upstream_grant_request(&request).unwrap();
    let descriptor = File::open("/dev/null").unwrap();
    client.requests.send(&bytes, &[descriptor.as_raw_fd()]).await.unwrap();

    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "worker sent an upstream descriptor");
    assert!(client.responses.recv().await.is_err());
}

#[tokio::test]
async fn disallowed_plain_http_port_is_denied_before_dial() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(![80, 3128, 3713, 8080, 11434].contains(&port));
    let (client, _publisher, authority, task) = start_broker(test_policy("policy-a", None, vec![]));

    let (response, fds) = client
        .request(&UpstreamGrantRequest::ResolveTcp {
            request_id: 1,
            protocol: UpstreamProtocol::Http,
            host: "127.0.0.1".into(),
            port,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 1,
            reason: UpstreamGrantDenial::NotAllowed,
        }
    );
    assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept())
        .await
        .is_err());
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn opaque_selections_cannot_cross_worker_generations() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = || test_policy("policy-a", Some(listener.local_addr().unwrap()), vec![]);
    let (first, _first_publisher, first_authority, first_task) = start_broker(policy());
    let (second, _second_publisher, second_authority, second_task) = start_broker(policy());
    let selection_id = resolve_override(&first, 1).await;

    let (response, fds) = second
        .request(&UpstreamGrantRequest::ConnectTcp {
            request_id: 1,
            selection_id,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 1,
            reason: UpstreamGrantDenial::InvalidResource,
        }
    );
    assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept())
        .await
        .is_err());
    first
        .send(&UpstreamGrantRequest::Release {
            resource_id: selection_id,
        })
        .await;
    stop_broker(first_authority, first_task).await;
    stop_broker(second_authority, second_task).await;
}

#[tokio::test]
async fn active_descriptor_capacity_is_bounded_and_recovers_after_release() {
    let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (client, _publisher, authority, task) =
        start_broker(test_policy("policy-a", None, vec![upstream.local_addr().unwrap()]));
    let mut descriptors = Vec::with_capacity(MAX_ACTIVE_GRANTS);
    let mut grant_ids = Vec::with_capacity(MAX_ACTIVE_GRANTS);
    for request_id in 1..=MAX_ACTIVE_GRANTS as u64 {
        let (response, mut fds) = client
            .request(&UpstreamGrantRequest::OpenDns {
                request_id,
                upstream_index: 0,
            })
            .await;
        let UpstreamGrantResponse::DescriptorGranted { grant_id, .. } = response else {
            panic!("expected descriptor grant, got {response:?}");
        };
        client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;
        grant_ids.push(grant_id);
        descriptors.push(fds.pop().unwrap());
    }

    let denied_id = MAX_ACTIVE_GRANTS as u64 + 1;
    let (response, fds) = client
        .request(&UpstreamGrantRequest::OpenDns {
            request_id: denied_id,
            upstream_index: 0,
        })
        .await;
    assert!(fds.is_empty());
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: denied_id,
            reason: UpstreamGrantDenial::Capacity,
        }
    );

    for grant_id in grant_ids {
        client
            .send(&UpstreamGrantRequest::Release { resource_id: grant_id })
            .await;
    }
    drop(descriptors);
    let (response, mut fds) = client
        .request(&UpstreamGrantRequest::OpenDns {
            request_id: denied_id + 1,
            upstream_index: 0,
        })
        .await;
    let UpstreamGrantResponse::DescriptorGranted { grant_id, .. } = response else {
        panic!("capacity did not recover: {response:?}");
    };
    client.send(&UpstreamGrantRequest::Adopted { grant_id }).await;
    client
        .send(&UpstreamGrantRequest::Release { resource_id: grant_id })
        .await;
    drop(fds.pop().unwrap());
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn malformed_frame_closes_the_generation_before_any_response() {
    let (client, _publisher, _authority, task) = start_broker(test_policy("policy-a", None, vec![]));
    let mut malformed = [0_u8; UPSTREAM_GRANT_FRAME_SIZE];
    malformed[..2].copy_from_slice(b"XX");
    client.requests.send(&malformed, &[]).await.unwrap();

    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("invalid upstream grant magic"), "{error}");
    assert!(client.responses.recv().await.is_err());
}
