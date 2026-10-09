use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, RawFd};
use std::sync::Arc;

use crate::net::dns::{DnsResolver, DnsUpstreamGrants};
use crate::net::mitm_proxy::{TcpUpstreamGrants, UpstreamTarget};
use crate::GuestMetadataAuthority as _;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_request, encode_upstream_grant_response, ProxyTrafficService, UpstreamDescriptorKind,
    UpstreamGrantDenial, UpstreamGrantRequest, UpstreamGrantResponse,
};

use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const POLICY_A: &str = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const POLICY_B: &str = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
async fn ledger_channel_preserves_grant_and_is_adopted_before_use() {
    let grant = LedgerChannelGrant::new(LedgerGeneration::new([0x6c; 16]), 44, LedgerClientRole::VmOwner).unwrap();
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let (ledger, mut ledger_peer) = UnixStream::pair().unwrap();
    let (commitment, mut commitment_peer) = UnixStream::pair().unwrap();
    let broker_task = tokio::spawn(async move {
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenLedger { request_id: 1 }
        );
        send_response_many(
            &responses,
            &UpstreamGrantResponse::LedgerGranted { request_id: 1, grant },
            &[ledger.as_raw_fd(), commitment.as_raw_fd()],
        )
        .await;
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Adopted {
                grant_id: grant.client_id(),
            }
        );
    });

    let (mut stream, mut commitment_stream, received) = client.open_ledger().await.unwrap();
    assert_eq!(received, grant);
    stream.write_all(b"hello").unwrap();
    let mut message = [0_u8; 5];
    ledger_peer.read_exact(&mut message).unwrap();
    assert_eq!(&message, b"hello");
    commitment_stream.write_all(b"proof").unwrap();
    commitment_peer.read_exact(&mut message).unwrap();
    assert_eq!(&message, b"proof");
    broker_task.await.unwrap();
}

#[tokio::test]
async fn proxy_traffic_descriptor_is_surrendered_before_adoption_returns() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let (traffic, mut guest) = UnixStream::pair().unwrap();
    let broker_task = tokio::spawn(async move {
        let frame = requests.recv().await.unwrap();
        assert_eq!(frame.fds.len(), 1);
        assert_eq!(
            decode_upstream_grant_request(&frame.bytes).unwrap(),
            UpstreamGrantRequest::AttachProxyTraffic {
                request_id: 1,
                service: ProxyTrafficService::Dns,
            }
        );
        let mut adopted = UnixStream::from(frame.fds.into_iter().next().unwrap());
        adopted.write_all(b"owned").unwrap();
        send_response(
            &responses,
            &UpstreamGrantResponse::ProxyTrafficAdopted { request_id: 1 },
            None,
        )
        .await;
    });

    client
        .attach_proxy_traffic(ProxyTrafficService::Dns, traffic.into())
        .await
        .unwrap();
    let mut proof = [0; 5];
    guest.read_exact(&mut proof).unwrap();
    assert_eq!(&proof, b"owned");
    broker_task.await.unwrap();
}

#[tokio::test]
async fn proxy_mcp_descriptor_is_surrendered_before_adoption_returns() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let (capability, mut proxy) = UnixStream::pair().unwrap();
    let broker_task = tokio::spawn(async move {
        let frame = requests.recv().await.unwrap();
        assert_eq!(frame.fds.len(), 1);
        assert_eq!(
            decode_upstream_grant_request(&frame.bytes).unwrap(),
            UpstreamGrantRequest::AttachProxyMcp { request_id: 1 }
        );
        let mut adopted = UnixStream::from(frame.fds.into_iter().next().unwrap());
        adopted.write_all(b"mcp").unwrap();
        send_response(
            &responses,
            &UpstreamGrantResponse::ProxyMcpAdopted { request_id: 1 },
            None,
        )
        .await;
    });

    client.attach_proxy_mcp(capability.into()).await.unwrap();
    let mut proof = [0; 3];
    proxy.read_exact(&mut proof).unwrap();
    assert_eq!(&proof, b"mcp");
    broker_task.await.unwrap();
}

#[tokio::test]
async fn ledger_denial_and_missing_descriptor_fail_closed() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let denied = tokio::spawn(async move {
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenLedger { request_id: 1 }
        );
        send_response(
            &responses,
            &UpstreamGrantResponse::Denied {
                request_id: 1,
                reason: UpstreamGrantDenial::NotConfigured,
            },
            None,
        )
        .await;
    });
    let error = client.open_ledger().await.unwrap_err().to_string();
    assert!(error.contains("NotConfigured"), "{error}");
    denied.await.unwrap();

    let grant = LedgerChannelGrant::new(LedgerGeneration::new([0x7d; 16]), 45, LedgerClientRole::VmOwner).unwrap();
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let malformed = tokio::spawn(async move {
        assert!(matches!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenLedger { request_id: 1 }
        ));
        send_response(
            &responses,
            &UpstreamGrantResponse::LedgerGranted { request_id: 1, grant },
            None,
        )
        .await;
    });
    let error = client.open_ledger().await.unwrap_err().to_string();
    assert!(error.contains("carried 0 descriptors"), "{error}");
    malformed.await.unwrap();
}

async fn open_error(client: &UpstreamGrantClient) -> String {
    match client.open(0, POLICY_A).await {
        Ok(_) => panic!("unexpected DNS grant"),
        Err(error) => error.to_string(),
    }
}

fn channels(socket: UnixStream) -> (WireReceiver, WireSender) {
    let receiver = WireReceiver::new(socket.try_clone().unwrap()).unwrap();
    let sender = WireSender::new(socket).unwrap();
    (receiver, sender)
}

async fn receive_request(receiver: &WireReceiver) -> UpstreamGrantRequest {
    let frame = receiver.recv().await.unwrap();
    assert!(frame.fds.is_empty());
    decode_upstream_grant_request(&frame.bytes).unwrap()
}

async fn send_response(sender: &WireSender, response: &UpstreamGrantResponse, descriptor: Option<RawFd>) {
    let fds = descriptor.map_or_else(Vec::new, |descriptor| vec![descriptor]);
    send_response_many(sender, response, &fds).await;
}

async fn send_response_many(sender: &WireSender, response: &UpstreamGrantResponse, fds: &[RawFd]) {
    let bytes = encode_upstream_grant_response(response).unwrap();
    sender.send(&bytes, fds).await.unwrap();
}

fn query() -> Vec<u8> {
    let mut query = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    query.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0]);
    query.extend_from_slice(&[0, 1, 0, 1]);
    query
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_mode_changes_use_the_serialized_coordinator_channel() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = Arc::new(UpstreamGrantClient::start(worker).unwrap());
    let (requests, responses) = channels(broker);
    let broker_task = tokio::spawn(async move {
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::SetGuestMode {
                request_id: 1,
                relative_path: b"workspace/pyvenv/bin/activate".to_vec(),
                mode: 0o755,
            }
        );
        send_response(&responses, &UpstreamGrantResponse::GuestModeSet { request_id: 1 }, None).await;
    });
    let client_for_mode = Arc::clone(&client);
    tokio::task::spawn_blocking(move || {
        client_for_mode
            .set_mode(b"workspace/pyvenv/bin/activate", 0o755)
            .unwrap();
    })
    .await
    .unwrap();
    broker_task.await.unwrap();
}

#[tokio::test]
async fn dns_query_uses_connected_coordinator_descriptor_and_releases_it() {
    let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = Arc::new(UpstreamGrantClient::start(worker).unwrap());
    let (requests, responses) = channels(broker);
    let expected_query = query();
    let broker_task = tokio::spawn(async move {
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenDns {
                request_id: 1,
                upstream_index: 0,
            }
        );
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(upstream_address).unwrap();
        send_response(
            &responses,
            &UpstreamGrantResponse::DescriptorGranted {
                request_id: 1,
                grant_id: 41,
                kind: UpstreamDescriptorKind::DnsUdp,
                policy_digest: POLICY_A.into(),
            },
            Some(socket.as_raw_fd()),
        )
        .await;
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Adopted { grant_id: 41 }
        );
        let mut received = [0_u8; 512];
        let (count, peer) = upstream.recv_from(&mut received).await.unwrap();
        assert_eq!(&received[..count], expected_query);
        let mut answer = received[..count].to_vec();
        answer[2] = 0x81;
        answer[3] = 0x80;
        upstream.send_to(&answer, peer).await.unwrap();
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Release { resource_id: 41 }
        );
    });

    let resolver = DnsResolver::with_grants(vec![upstream_address], client);
    let (answer, _) = resolver.resolve_for_policy(&query(), POLICY_A).await.unwrap();
    assert_eq!(&answer[..2], &[0x12, 0x34]);
    assert_eq!(&answer[2..4], &[0x81, 0x80]);
    broker_task.await.unwrap();
}

#[tokio::test]
async fn stale_policy_grant_is_adopted_released_and_channel_remains_usable() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let broker_task = tokio::spawn(async move {
        assert!(matches!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenDns { request_id: 1, .. }
        ));
        let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(peer.local_addr().unwrap()).unwrap();
        send_response(
            &responses,
            &UpstreamGrantResponse::DescriptorGranted {
                request_id: 1,
                grant_id: 51,
                kind: UpstreamDescriptorKind::DnsUdp,
                policy_digest: POLICY_B.into(),
            },
            Some(socket.as_raw_fd()),
        )
        .await;
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Adopted { grant_id: 51 }
        );
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Release { resource_id: 51 }
        );
        assert!(matches!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenDns { request_id: 2, .. }
        ));
        send_response(
            &responses,
            &UpstreamGrantResponse::Denied {
                request_id: 2,
                reason: UpstreamGrantDenial::NotConfigured,
            },
            None,
        )
        .await;
    });

    let error = open_error(&client).await;
    assert!(error.contains("policy mismatch"), "{error}");
    let error = open_error(&client).await;
    assert!(error.contains("NotConfigured"), "{error}");
    broker_task.await.unwrap();
}

#[tokio::test]
async fn malformed_or_revoked_grant_channel_fails_closed() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let broker_task = tokio::spawn(async move {
        assert!(matches!(
            receive_request(&requests).await,
            UpstreamGrantRequest::OpenDns { request_id: 1, .. }
        ));
        send_response(
            &responses,
            &UpstreamGrantResponse::DescriptorGranted {
                request_id: 1,
                grant_id: 61,
                kind: UpstreamDescriptorKind::DnsUdp,
                policy_digest: POLICY_A.into(),
            },
            None,
        )
        .await;
    });
    let error = open_error(&client).await;
    assert!(error.contains("carried 0 descriptors"), "{error}");
    broker_task.await.unwrap();
    let error = open_error(&client).await;
    assert!(error.contains("closed") || error.contains("stopped"), "{error}");

    let (broker, worker) = UnixStream::pair().unwrap();
    let revoked = UpstreamGrantClient::start(worker).unwrap();
    drop(broker);
    assert!(!open_error(&revoked).await.is_empty());
}

#[tokio::test]
async fn tcp_selection_and_connected_stream_are_brokered_and_released() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = listener.local_addr().unwrap();
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let broker_task = tokio::spawn(async move {
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::ResolveTcp {
                request_id: 1,
                protocol: UpstreamProtocol::Tls,
                host: "api.example".into(),
                port: 443,
            }
        );
        send_response(
            &responses,
            &UpstreamGrantResponse::TcpResolved {
                request_id: 1,
                selection_id: 71,
                protocol: UpstreamProtocol::Http,
                judged_ip: Some("127.0.0.1".parse().unwrap()),
                policy_digest: POLICY_A.into(),
            },
            None,
        )
        .await;
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::ConnectTcp {
                request_id: 2,
                selection_id: 71,
            }
        );
        let socket = tokio::net::TcpStream::connect(upstream_address)
            .await
            .unwrap()
            .into_std()
            .unwrap();
        let (mut upstream, _) = listener.accept().await.unwrap();
        send_response(
            &responses,
            &UpstreamGrantResponse::DescriptorGranted {
                request_id: 2,
                grant_id: 72,
                kind: UpstreamDescriptorKind::Tcp,
                policy_digest: POLICY_A.into(),
            },
            Some(socket.as_raw_fd()),
        )
        .await;
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Adopted { grant_id: 72 }
        );
        let mut request = [0_u8; 4];
        upstream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        upstream.write_all(b"pong").await.unwrap();
        assert_eq!(
            receive_request(&requests).await,
            UpstreamGrantRequest::Release { resource_id: 72 }
        );
    });

    let selection = client
        .resolve(Protocol::Tls, "api.example", 443, POLICY_A)
        .await
        .unwrap();
    assert_eq!(selection.protocol, Protocol::Http);
    assert_eq!(selection.judged_ip, Some("127.0.0.1".parse().unwrap()));
    let target = UpstreamTarget::Granted {
        host: "api.example".into(),
        port: 443,
        guest_protocol: Protocol::Tls,
        protocol: selection.protocol,
        judged_ip: selection.judged_ip,
        selection: Some(selection),
    };
    let (mut stream, pinned) = target.connect_with_grants(Some(&client), POLICY_A).await.unwrap();
    assert!(matches!(pinned, UpstreamTarget::Granted { selection: None, .. }));
    stream.write_all(b"ping").await.unwrap();
    let mut response = [0_u8; 4];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"pong");
    drop(stream);
    broker_task.await.unwrap();
}

#[tokio::test]
async fn unused_or_stale_tcp_selections_are_released_without_connecting() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker).unwrap();
    let (requests, responses) = channels(broker);
    let broker_task = tokio::spawn(async move {
        for (request_id, selection_id, digest) in [(1, 81, POLICY_A), (2, 82, POLICY_B)] {
            assert!(matches!(
                receive_request(&requests).await,
                UpstreamGrantRequest::ResolveTcp {
                    request_id: actual,
                    ..
                } if actual == request_id
            ));
            send_response(
                &responses,
                &UpstreamGrantResponse::TcpResolved {
                    request_id,
                    selection_id,
                    protocol: UpstreamProtocol::Tls,
                    judged_ip: Some("93.184.216.34".parse().unwrap()),
                    policy_digest: digest.into(),
                },
                None,
            )
            .await;
            assert_eq!(
                receive_request(&requests).await,
                UpstreamGrantRequest::Release {
                    resource_id: selection_id,
                }
            );
        }
    });

    let unused = client
        .resolve(Protocol::Tls, "unused.example", 443, POLICY_A)
        .await
        .unwrap();
    drop(unused);
    let error = client
        .resolve(Protocol::Tls, "stale.example", 443, POLICY_A)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("policy mismatch"), "{error}");
    broker_task.await.unwrap();
}

#[test]
fn inherited_channel_is_duplicated_cloexec_and_connected() {
    let (inherited, peer) = UnixStream::pair().unwrap();
    let mut adopted = adopt_inherited(inherited.as_fd()).unwrap();
    let mut peer = peer;
    assert!(fd::set_close_on_exec(inherited.as_fd()).unwrap());
    adopted.write_all(b"x").unwrap();
    let mut byte = [0_u8; 1];
    peer.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [b'x']);
}
