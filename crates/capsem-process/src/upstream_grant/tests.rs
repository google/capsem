use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _};

use capsem_core::net::dns::{DnsResolver, DnsUpstreamGrants};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_request, encode_upstream_grant_response, UpstreamDescriptorKind, UpstreamGrantDenial,
    UpstreamGrantRequest, UpstreamGrantResponse,
};

use super::*;

const POLICY_A: &str = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const POLICY_B: &str = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn open_error(client: &UpstreamGrantClient) -> String {
    match client.open(0).await {
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

async fn send_response(
    sender: &WireSender,
    response: &UpstreamGrantResponse,
    descriptor: Option<&std::net::UdpSocket>,
) {
    let bytes = encode_upstream_grant_response(response).unwrap();
    let fds = descriptor.map_or_else(Vec::new, |socket| vec![socket.as_raw_fd()]);
    sender.send(&bytes, &fds).await.unwrap();
}

fn query() -> Vec<u8> {
    let mut query = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    query.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0]);
    query.extend_from_slice(&[0, 1, 0, 1]);
    query
}

#[tokio::test]
async fn dns_query_uses_connected_coordinator_descriptor_and_releases_it() {
    let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = Arc::new(UpstreamGrantClient::start(worker, POLICY_A.into()).unwrap());
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
            Some(&socket),
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
    let (answer, _) = resolver.resolve(&query()).await.unwrap();
    assert_eq!(&answer[..2], &[0x12, 0x34]);
    assert_eq!(&answer[2..4], &[0x81, 0x80]);
    broker_task.await.unwrap();
}

#[tokio::test]
async fn stale_policy_grant_is_adopted_released_and_channel_remains_usable() {
    let (broker, worker) = UnixStream::pair().unwrap();
    let client = UpstreamGrantClient::start(worker, POLICY_A.into()).unwrap();
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
            Some(&socket),
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
    let client = UpstreamGrantClient::start(worker, POLICY_A.into()).unwrap();
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
    let revoked = UpstreamGrantClient::start(worker, POLICY_A.into()).unwrap();
    drop(broker);
    assert!(!open_error(&revoked).await.is_empty());
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
