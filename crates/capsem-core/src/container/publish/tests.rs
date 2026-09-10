use super::*;
use std::os::fd::AsRawFd;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

#[tokio::test]
async fn concurrent_guest_setups_grant_ids_in_handoff_order() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let receiver = capsem_foundation::unix::router_channel::Receiver::new(child.try_clone().unwrap()).unwrap();
    let mut events = UnixStream::from_std(child).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let _first = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { id: first, .. } = requests.recv().await.unwrap() else {
        panic!("missing first setup");
    };
    let _second = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { id: second, .. } = requests.recv().await.unwrap() else {
        panic!("missing second setup");
    };
    let mut peers = Vec::new();
    for (expected_grant, request_id) in [(1, second), (2, first)] {
        let (connection, peer) = StdUnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut peer = UnixStream::from_std(peer).unwrap();
        let mut header = [0; 9];
        header[..8].copy_from_slice(&request_id.to_be_bytes());
        header[8] = 1;
        peer.write_all(&header).await.unwrap();
        owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
        let frame = receiver.recv().await.unwrap();
        let Grant::Connected { id, .. } = Grant::decode(frame).unwrap() else {
            panic!("missing pair");
        };
        assert_eq!(id, expected_grant, "setup completion order must not look like replay");
        Event::Accepted(id).write(&mut events).await.unwrap();
        peers.push(peer);
    }
    drop(receiver);
    drop(events);
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(owner.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_pair_ack_shuts_down_retained_descriptors() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let receiver = capsem_foundation::unix::router_channel::Receiver::new(child).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { id, .. } = requests.recv().await.unwrap() else {
        panic!("missing guest setup");
    };
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    let mut header = [0; 9];
    header[..8].copy_from_slice(&id.to_be_bytes());
    header[8] = 1;
    peer.write_all(&header).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    let granted = Grant::decode(receiver.recv().await.unwrap()).unwrap();
    assert!(matches!(granted, Grant::Connected { id: granted_id, .. } if granted_id == id));
    assert!(tokio::time::timeout(Duration::from_secs(4), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    // The hostile child still owns its copies; owner shutdown must wake peers.
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    assert!(owner.pending.lock().unwrap().is_empty());
    drop(granted);
}

#[tokio::test]
async fn compromised_router_cannot_request_destination_connections() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut child = UnixStream::from_std(child).unwrap();
    Event::Accepted(1).write(&mut child).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(
        requests.recv().await.is_none(),
        "child caused destination setup without host accept"
    );
    assert!(owner.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_child_record_cannot_allocate_or_dial() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let broker = tokio::spawn(broker::serve(
        owner,
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut child = UnixStream::from_std(child).unwrap();
    child.write_all(&[255; 10]).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(requests.recv().await.is_none());
}

#[tokio::test]
async fn child_control_eof_cancels_guest_setup_and_closes_accepted_tcp() {
    let owner = Arc::new(Publisher::default());
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let (control, mut requests) = mpsc::channel(4);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
    ));
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    assert!(matches!(
        requests.recv().await.unwrap(),
        ServiceToProcess::ConnectPort { port: 6379, .. }
    ));
    drop(child);
    assert!(tokio::time::timeout(Duration::from_secs(1), broker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(owner.pending.lock().unwrap().is_empty());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), client.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}
