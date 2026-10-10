use std::os::unix::net::UnixStream;

use capsem_foundation::ipc_channel;
use capsem_proto::proxy_credentials::{
    ProxyCredentialRequest, ProxyCredentialResponse, ProxyHeader, ProxyModelProvider,
};

#[tokio::test]
async fn substitution_round_trip_preserves_unbrokered_headers_and_query() {
    let (service, client) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(super::serve(service));
    let (sender, receiver) =
        ipc_channel::channel_from_std::<ProxyCredentialRequest, ProxyCredentialResponse>(client).unwrap();
    sender
        .send(ProxyCredentialRequest::Substitute {
            request_id: 7,
            domain: "example.com".to_string(),
            ai_provider: Some(ProxyModelProvider::Unknown),
            headers: vec![ProxyHeader::new("x-test", b"ordinary".to_vec())],
            query: Some("page=1".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await.unwrap(),
        ProxyCredentialResponse::Substituted {
            request_id: 7,
            headers: vec![ProxyHeader::new("x-test", b"ordinary".to_vec())],
            query: Some("page=1".to_string()),
            credential_ref: None,
        }
    );
    drop(sender);
    drop(receiver);
    serving.await.unwrap().unwrap();
}
