use std::os::unix::net::UnixStream;

use capsem_foundation::ipc_channel;
use capsem_proto::proxy_private_names::{ProxyPrivateNameRequest, ProxyPrivateNameResponse};

#[tokio::test]
async fn session_bound_lookup_returns_not_found_without_visible_members() {
    let state = crate::tests::make_test_state();
    let (service, client) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(super::serve(state, "vm-a".to_string(), service));
    let (sender, receiver) =
        ipc_channel::channel_from_std::<ProxyPrivateNameRequest, ProxyPrivateNameResponse>(client).unwrap();
    sender
        .send(ProxyPrivateNameRequest::AddressOf {
            request_id: 8,
            name: "missing.dev.capsem.internal".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await.unwrap(),
        ProxyPrivateNameResponse::NotFound { request_id: 8 }
    );
    drop(sender);
    drop(receiver);
    serving.await.unwrap().unwrap();
}
