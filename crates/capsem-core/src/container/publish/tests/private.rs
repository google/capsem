//! The private class of the broker: a stream another owner delivered, fed by
//! channel instead of accepted from a host listener, granted to the router
//! as a private pair.
use super::*;
use crate::security_engine::network::{NetworkIdentity, NetworkVm};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn network() -> NetworkIdentity {
    NetworkIdentity::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "team".into()).unwrap()
}

fn member() -> NetworkVm {
    NetworkVm {
        id: "vm-a".into(),
        name: "vm-a".into(),
        generation: NonZeroU64::new(7).unwrap(),
    }
}

/// A delivered stream and the far end another owner's guest would hold.
fn delivered(owner: &Publisher, port: u16) -> (Incoming, UnixStream) {
    let (stream, far) = StdUnixStream::pair().unwrap();
    far.set_nonblocking(true).unwrap();
    let audit = owner
        .private_audit(network(), member(), (Ipv4Addr::new(10, 128, 0, 2), 40001).into(), port)
        .unwrap();
    let incoming = Incoming {
        source: Source::Stream(stream.into()),
        audit,
        port,
        keepalive: None,
    };
    (incoming, UnixStream::from_std(far).unwrap())
}

struct Fixture {
    owner: Arc<Publisher>,
    feed: mpsc::Sender<Incoming>,
    requests: mpsc::Receiver<ServiceToProcess>,
    grants: capsem_foundation::unix::router_channel::Receiver,
    events: UnixStream,
    stop: CancellationToken,
    broker: tokio::task::JoinHandle<Result<()>>,
}

fn fixture(budgets: capsem_config::router::RouterConfig) -> Fixture {
    let owner = Arc::new(security::authorized_publisher(budgets));
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    let grants = capsem_foundation::unix::router_channel::Receiver::new(child.try_clone().unwrap()).unwrap();
    let router = Arc::new(companion::Router::new(0, sender, CancellationToken::new()));
    let monitor = router.clone();
    let parent = UnixStream::from_std(parent).unwrap();
    tokio::spawn(async move {
        let _ = monitor.read_events(parent).await;
        monitor.closed.cancel();
    });
    let (control, requests) = mpsc::channel(4);
    let (feed, incoming) = mpsc::channel(4);
    let stop = CancellationToken::new();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        incoming,
        control,
        router,
        stop.clone(),
        capsem_router::Class::Private,
    ));
    Fixture {
        owner,
        feed,
        requests,
        grants,
        events: UnixStream::from_std(child).unwrap(),
        stop,
        broker,
    }
}

async fn guest_answers(owner: &Arc<Publisher>, flow: capsem_proto::router::FlowKey) -> UnixStream {
    let (connection, peer) = StdUnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = UnixStream::from_std(peer).unwrap();
    peer.write_all(&flow.data_header(true)).await.unwrap();
    owner.accept(VsockConnection::new(connection.as_raw_fd(), 0, Box::new(connection)));
    peer
}

async fn relays(mut from: UnixStream, to: std::os::fd::OwnedFd, what: &[u8]) {
    from.write_all(what).await.unwrap();
    let std = StdUnixStream::from(to);
    std.set_nonblocking(true).unwrap();
    let mut to = UnixStream::from_std(std).unwrap();
    let mut seen = vec![0; what.len()];
    tokio::time::timeout(Duration::from_secs(2), to.read_exact(&mut seen))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seen, what);
}

#[tokio::test]
async fn a_delivered_stream_is_granted_to_the_router_as_a_private_pair() {
    let mut fixture = fixture(capsem_config::router::RouterConfig::default());
    let (incoming, far) = delivered(&fixture.owner, 80);
    fixture.feed.send(incoming).await.unwrap();
    let Some(ServiceToProcess::ConnectPort { flow, port }) = fixture.requests.recv().await else {
        panic!("the destination guest was not asked for the connection");
    };
    assert_eq!(port, 80, "the guest is dialled on the port the source asked for");
    let guest = guest_answers(&fixture.owner, flow).await;
    let frame = tokio::time::timeout(Duration::from_secs(2), fixture.grants.recv())
        .await
        .unwrap()
        .unwrap();
    let Grant::Connected {
        id,
        class,
        source,
        destination,
    } = Grant::decode(frame).unwrap()
    else {
        panic!("the router did not receive a pair");
    };
    assert_eq!(
        class,
        capsem_router::Class::Private,
        "a private flow must not spend the expose quota"
    );
    // The pair the router holds is the delivered stream and the guest's
    // data connection: bytes written at either far end arrive on them.
    relays(far, source, b"from the source member").await;
    relays(guest, destination, b"from the destination guest").await;
    Event::Accepted(id).write(&mut fixture.events).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.stop.cancel();
    fixture.broker.await.unwrap().unwrap();
    fixture.owner.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_private_setup_that_times_out_resets_the_delivered_stream() {
    let mut fixture = fixture(capsem_config::router::RouterConfig::default());
    let (incoming, mut far) = delivered(&fixture.owner, 80);
    fixture.feed.send(incoming).await.unwrap();
    let Some(ServiceToProcess::ConnectPort { flow, .. }) = fixture.requests.recv().await else {
        panic!("the destination guest was not asked for the connection");
    };
    // The guest never answers: the setup deadline ends the delivered stream
    // and tells the guest to forget the flow, without a router grant.
    let mut probe = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(20), far.read(&mut probe))
        .await
        .expect("the delivered stream outlived the setup deadline")
        .unwrap();
    assert_eq!(closed, 0, "the far end must see the stream end, not a byte");
    let Some(ServiceToProcess::AbortPorts { flows }) = fixture.requests.recv().await else {
        panic!("the guest was not told to abandon the flow");
    };
    assert_eq!(flows, vec![flow]);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.grants.recv())
            .await
            .is_err(),
        "a timed-out setup must never reach the router"
    );
    fixture.stop.cancel();
    fixture.broker.await.unwrap().unwrap();
    fixture.owner.shutdown().await;
}

#[tokio::test]
async fn the_private_quota_refuses_the_stream_past_the_last_seat() {
    let mut budgets = capsem_config::router::RouterConfig::default();
    budgets.private.connections = 1;
    let mut fixture = fixture(budgets);
    let (first, _first_far) = delivered(&fixture.owner, 80);
    let (second, mut second_far) = delivered(&fixture.owner, 80);
    fixture.feed.send(first).await.unwrap();
    assert!(matches!(
        fixture.requests.recv().await,
        Some(ServiceToProcess::ConnectPort { port: 80, .. })
    ));
    fixture.feed.send(second).await.unwrap();
    let mut probe = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(2), second_far.read(&mut probe))
        .await
        .expect("the refused stream was kept open")
        .unwrap();
    assert_eq!(closed, 0, "past the quota the stream ends without a byte");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.requests.recv())
            .await
            .is_err(),
        "a refused stream must not dial the guest"
    );
    fixture.stop.cancel();
    fixture.broker.await.unwrap().unwrap();
    fixture.owner.shutdown().await;
}
