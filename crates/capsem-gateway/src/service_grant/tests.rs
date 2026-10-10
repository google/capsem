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
    }
}
