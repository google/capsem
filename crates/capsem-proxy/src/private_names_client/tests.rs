use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;

use capsem_core::net::dns::private::PrivateNames;
use capsem_foundation::ipc_channel;
use capsem_proto::proxy_private_names::{ProxyPrivateNameRequest, ProxyPrivateNameResponse};

#[tokio::test]
async fn correlated_lookup_and_idle_disconnect_are_observed() {
    let (service, proxy) = UnixStream::pair().unwrap();
    let (client, mut closed) = super::PrivateNameClient::start(proxy).unwrap();
    let serving = tokio::spawn(async move {
        let (sender, receiver) =
            ipc_channel::channel_from_std::<ProxyPrivateNameResponse, ProxyPrivateNameRequest>(service).unwrap();
        let request = receiver.recv().await.unwrap();
        assert_eq!(
            request,
            ProxyPrivateNameRequest::AddressOf {
                request_id: 1,
                name: "box.dev.capsem.internal".to_string(),
            }
        );
        sender
            .send(ProxyPrivateNameResponse::Address {
                request_id: 1,
                address: Ipv4Addr::new(10, 0, 0, 2),
            })
            .await
            .unwrap();
    });
    assert_eq!(
        client.address_of("box.dev.capsem.internal").await,
        Some(Ipv4Addr::new(10, 0, 0, 2))
    );
    serving.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), closed.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *closed.borrow(),
        Some(capsem_proto::proxy_control::ProxyChannelCloseReason::Disconnected)
    );
}
