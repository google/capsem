use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::{decode_gateway_grant_response, encode_gateway_grant_request, GatewayGrantRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn grants_only_the_service_and_registry_derived_owner_seats() {
    let state = crate::tests::make_test_state();
    std::fs::create_dir_all(state.service_socket.parent().unwrap()).unwrap();
    let listener = tokio::net::UnixListener::bind(&state.service_socket).unwrap();
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, Arc::clone(&state)));
    let requests =
        DescriptorSender::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client.try_clone().unwrap()).unwrap();
    let responses = DescriptorReceiver::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client).unwrap();

    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenService { request_id: 1 }).unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 1,
            kind: GatewayGrantKind::Service
        }
    ));
    let (mut service_peer, _) = listener.accept().await.unwrap();
    let mut granted = tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(
        response.fds.into_iter().next().unwrap(),
    ))
    .unwrap();
    service_peer.write_all(b"s").await.unwrap();
    let mut byte = [0];
    granted.read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, [b's']);

    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenOwnerHandoff {
        request_id: 2,
        vm_id: "not-running".into(),
    })
    .unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(response.fds.is_empty());
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Denied {
            request_id: 2,
            reason: GatewayGrantDenial::NotFound
        }
    ));
    broker.abort();
}
