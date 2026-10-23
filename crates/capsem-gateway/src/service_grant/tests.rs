use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::{decode_gateway_grant_request, encode_gateway_grant_response, GatewayGrantResponse};
use std::os::fd::AsRawFd;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn grants_service_and_registry_named_owner_connections() {
    let (client, server) = UnixStream::pair().unwrap();
    let grants = GatewayGrantClient::start(client).unwrap();
    let (release, released) = tokio::sync::oneshot::channel();
    let broker = tokio::spawn(async move {
        let requests =
            DescriptorReceiver::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(server.try_clone().unwrap())
                .unwrap();
        let responses = DescriptorSender::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(server).unwrap();
        let mut peers = Vec::new();
        for expected in [GatewayGrantKind::Service, GatewayGrantKind::OwnerHandoff] {
            let request = requests.recv().await.unwrap();
            let request = decode_gateway_grant_request(&request.bytes).unwrap();
            if expected == GatewayGrantKind::OwnerHandoff {
                assert!(matches!(
                    &request,
                    GatewayGrantRequest::OpenOwnerHandoff { vm_id, .. } if vm_id == "box"
                ));
            }
            let (mut peer, granted) = UnixStream::pair().unwrap();
            let response = encode_gateway_grant_response(GatewayGrantResponse::Granted {
                request_id: request.request_id(),
                kind: expected,
            });
            responses.send(&response, &[granted.as_raw_fd()]).await.unwrap();
            let receipt = requests.recv().await.unwrap();
            assert!(receipt.fds.is_empty());
            assert_eq!(
                decode_gateway_grant_request(&receipt.bytes).unwrap(),
                GatewayGrantRequest::Adopted {
                    request_id: request.request_id()
                }
            );
            drop(granted);
            std::io::Write::write_all(&mut peer, &[kind_code(expected)]).unwrap();
            peers.push(peer);
        }
        let _ = released.await;
    });

    let mut service = grants.open_service().await.unwrap();
    let mut byte = [0];
    service.read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, [1]);
    let mut owner = grants.open_owner("box".into()).await.unwrap();
    owner.read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, [2]);
    release.send(()).unwrap();
    broker.await.unwrap();
}

fn kind_code(kind: GatewayGrantKind) -> u8 {
    match kind {
        GatewayGrantKind::Service => 1,
        GatewayGrantKind::OwnerHandoff => 2,
        GatewayGrantKind::Metrics => 3,
    }
}

#[tokio::test]
async fn optional_metrics_denial_does_not_disable_service_grants() {
    let (client, server) = UnixStream::pair().unwrap();
    let grants = GatewayGrantClient::start(client).unwrap();
    let broker = tokio::spawn(async move {
        let requests = WireReceiver::new(server.try_clone().unwrap()).unwrap();
        let responses = WireSender::new(server).unwrap();
        let request = requests.recv().await.unwrap();
        assert_eq!(
            decode_gateway_grant_request(&request.bytes).unwrap(),
            GatewayGrantRequest::OpenMetrics { request_id: 1 }
        );
        responses
            .send(
                &encode_gateway_grant_response(GatewayGrantResponse::Denied {
                    request_id: 1,
                    reason: capsem_proto::gateway_grant::GatewayGrantDenial::Unavailable,
                }),
                &[],
            )
            .await
            .unwrap();
        let request = requests.recv().await.unwrap();
        assert_eq!(
            decode_gateway_grant_request(&request.bytes).unwrap(),
            GatewayGrantRequest::OpenService { request_id: 2 }
        );
        let (_peer, descriptor) = UnixStream::pair().unwrap();
        responses
            .send(
                &encode_gateway_grant_response(GatewayGrantResponse::Granted {
                    request_id: 2,
                    kind: GatewayGrantKind::Service,
                }),
                &[descriptor.as_raw_fd()],
            )
            .await
            .unwrap();
        let receipt = requests.recv().await.unwrap();
        assert_eq!(
            decode_gateway_grant_request(&receipt.bytes).unwrap(),
            GatewayGrantRequest::Adopted { request_id: 2 }
        );
    });
    assert_eq!(
        grants.open_metrics().await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(grants.open_service().await.is_ok());
    broker.await.unwrap();
}

#[tokio::test]
async fn metrics_descriptor_is_adopted_from_the_authenticated_service_peer() {
    let (client, server) = UnixStream::pair().unwrap();
    let grants = GatewayGrantClient::start(client).unwrap();
    let broker = tokio::spawn(async move {
        let requests = WireReceiver::new(server.try_clone().unwrap()).unwrap();
        let responses = WireSender::new(server).unwrap();
        let request = requests.recv().await.unwrap();
        assert_eq!(
            decode_gateway_grant_request(&request.bytes).unwrap(),
            GatewayGrantRequest::OpenMetrics { request_id: 1 }
        );
        let (peer, descriptor) = UnixStream::pair().unwrap();
        responses
            .send(
                &encode_gateway_grant_response(GatewayGrantResponse::Granted {
                    request_id: 1,
                    kind: GatewayGrantKind::Metrics,
                }),
                &[descriptor.as_raw_fd()],
            )
            .await
            .unwrap();
        let receipt = requests.recv().await.unwrap();
        assert_eq!(
            decode_gateway_grant_request(&receipt.bytes).unwrap(),
            GatewayGrantRequest::Adopted { request_id: 1 }
        );
        peer
    });
    let descriptor = grants.open_metrics().await.unwrap();
    assert_eq!(
        capsem_foundation::unix::peer::identity(descriptor.as_fd()).unwrap().uid,
        capsem_foundation::unix::process::current_uid()
    );
    drop(broker.await.unwrap());
}

#[tokio::test]
async fn metric_fd_from_a_foreign_same_user_process_is_not_adopted() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("foreign.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "service_grant::tests::foreign_metric_peer_child",
            "--nocapture",
        ])
        .env("CAPSEM_FOREIGN_METRIC_SOCKET", &path)
        .spawn()
        .unwrap();
    let (descriptor, _) = listener.accept().await.unwrap();
    let descriptor = descriptor.into_std().unwrap();
    let (client, server) = UnixStream::pair().unwrap();
    let grants = GatewayGrantClient::start(client).unwrap();
    let broker = tokio::spawn(async move {
        let requests = WireReceiver::new(server.try_clone().unwrap()).unwrap();
        let responses = WireSender::new(server).unwrap();
        requests.recv().await.unwrap();
        responses
            .send(
                &encode_gateway_grant_response(GatewayGrantResponse::Granted {
                    request_id: 1,
                    kind: GatewayGrantKind::Metrics,
                }),
                &[descriptor.as_raw_fd()],
            )
            .await
            .unwrap();
        // A peer failure must close control, never acknowledge adoption.
        assert!(requests.recv().await.is_err());
    });
    assert_eq!(
        grants.open_metrics().await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(grants.open_service().await.is_err());
    broker.await.unwrap();
    assert!(child.wait_with_output().await.unwrap().status.success());
}

#[test]
fn foreign_metric_peer_child() {
    let Some(path) = std::env::var_os("CAPSEM_FOREIGN_METRIC_SOCKET") else {
        return;
    };
    let mut connection = UnixStream::connect(path).unwrap();
    let mut byte = [0];
    assert_eq!(std::io::Read::read(&mut connection, &mut byte).unwrap(), 0);
}
