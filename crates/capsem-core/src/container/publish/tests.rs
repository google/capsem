use super::*;
use std::os::fd::AsRawFd;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

#[tokio::test]
async fn a_previous_boot_header_cannot_consume_a_reused_request_id() {
    let owner = Arc::new(Publisher::default());
    let (pending, receiver) = owner.request().unwrap();
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    let mut header = [0; 17];
    header[..8].copy_from_slice(&pending.id.to_be_bytes());
    header[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    header[16] = 1;
    peer.write_all(&header).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        owner.pending.lock().unwrap().contains_key(&pending.id),
        "a stale boot consumed a current request with the same numeric ID"
    );
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    let flow = capsem_proto::router::FlowKey {
        generation: owner.generation,
        id: pending.id,
    };
    peer.write_all(&flow.data_header(true)).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    let accepted = tokio::time::timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(owner.pending.lock().unwrap().is_empty());
    drop(accepted);
    drop(pending);
    owner.shutdown().await;
}

async fn serve_fixture(
    owner: Arc<Publisher>,
    guest_port: u16,
    listener: TcpListener,
    control: mpsc::Sender<ServiceToProcess>,
    sender: capsem_foundation::unix::router_channel::Sender,
    events: UnixStream,
    cancellation: CancellationToken,
) -> Result<()> {
    let router = Arc::new(companion::Router::new(0, sender, CancellationToken::new()));
    let monitor = router.clone();
    let mut readers = tokio::task::JoinSet::new();
    readers.spawn(async move {
        let result = monitor.read_events(events).await;
        monitor.closed.cancel();
        result
    });
    let result = broker::serve(owner, guest_port, listener, control, router.clone(), cancellation).await;
    router.closed.cancel();
    let events = readers.join_next().await.unwrap().unwrap();
    result.and(events)
}

#[tokio::test]
async fn shutdown_joins_incomplete_guest_headers_and_refuses_new_arrivals() {
    let owner = Arc::new(Publisher::default());
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    owner.shutdown().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), peer.read(&mut [0]))
            .await
            .expect("shutdown left a detached header reader")
            .unwrap(),
        0
    );
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), peer.read(&mut [0]))
            .await
            .expect("closed publisher accepted another guest socket")
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn guest_setup_budget_is_shared_across_publications() {
    let owner = Arc::new(Publisher::default());
    let (parent, _child) = StdUnixStream::pair().unwrap();
    let router = Arc::new(companion::Router::new(
        0,
        capsem_foundation::unix::router_channel::Sender::new(parent).unwrap(),
        CancellationToken::new(),
    ));
    let (control, mut requests) = mpsc::channel(32);
    let cancellation = CancellationToken::new();
    let mut brokers = tokio::task::JoinSet::new();
    let mut clients = Vec::new();
    for guest_port in [6379, 6380] {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        brokers.spawn(broker::serve(
            owner.clone(),
            guest_port,
            listener,
            control.clone(),
            router.clone(),
            cancellation.clone(),
        ));
        for _ in 0..5 {
            clients.push(tokio::net::TcpStream::connect(address).await.unwrap());
        }
    }
    for _ in 0..8 {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), requests.recv())
                .await
                .unwrap()
                .unwrap(),
            ServiceToProcess::ConnectPort { .. }
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), requests.recv())
            .await
            .is_err(),
        "another mapping bypassed the VM-wide setup budget"
    );
    cancellation.cancel();
    while let Some(result) = brokers.join_next().await {
        result.unwrap().unwrap();
    }
    assert!(owner.pending.lock().unwrap().is_empty());
    assert_eq!(owner.ingress.available_permits(), capsem_router::CONNECTIONS_PER_CLASS);
    assert_eq!(owner.setups.available_permits(), 8);
    owner.shutdown().await;
}

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
    let broker = tokio::spawn(serve_fixture(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
        CancellationToken::new(),
    ));
    let _first = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { flow: first, .. } = requests.recv().await.unwrap() else {
        panic!("missing first setup");
    };
    let _second = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { flow: second, .. } = requests.recv().await.unwrap() else {
        panic!("missing second setup");
    };
    let mut peers = Vec::new();
    for (expected_grant, flow) in [(1, second), (2, first)] {
        let (connection, peer) = StdUnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut peer = UnixStream::from_std(peer).unwrap();
        let header = flow.data_header(true);
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
    let broker = tokio::spawn(serve_fixture(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
        CancellationToken::new(),
    ));
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let ServiceToProcess::ConnectPort { flow, .. } = requests.recv().await.unwrap() else {
        panic!("missing guest setup");
    };
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    let header = flow.data_header(true);
    peer.write_all(&header).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    let granted = Grant::decode(receiver.recv().await.unwrap()).unwrap();
    assert!(matches!(granted, Grant::Connected { id: 1, .. }));
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
    let broker = tokio::spawn(serve_fixture(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
        CancellationToken::new(),
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
    let broker = tokio::spawn(serve_fixture(
        owner,
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
        CancellationToken::new(),
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
    let broker = tokio::spawn(serve_fixture(
        owner.clone(),
        6379,
        listener,
        control,
        sender,
        UnixStream::from_std(parent).unwrap(),
        CancellationToken::new(),
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
