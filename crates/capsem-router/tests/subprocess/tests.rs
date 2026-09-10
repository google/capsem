use capsem_foundation::unix::router_channel::Sender;
use capsem_router::{send_grant, Class, Event, Grant, CONNECTIONS_PER_CLASS};
use std::net::Ipv4Addr;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};

struct Router {
    child: Child,
    sender: Option<Sender>,
    events: Option<UnixStream>,
    retained: std::collections::HashMap<u64, (OwnedFd, OwnedFd)>,
}
impl Router {
    async fn start() -> Self {
        let (parent, child) = StdUnixStream::pair().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_capsem-router"))
            .args(["--parent-pid", &std::process::id().to_string()])
            .env_clear()
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child)))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        parent.set_nonblocking(true).unwrap();
        let sender = Sender::new(parent.try_clone().unwrap()).unwrap();
        let mut router = Self {
            child,
            sender: Some(sender),
            events: Some(UnixStream::from_std(parent).unwrap()),
            retained: std::collections::HashMap::new(),
        };
        router.grant(Grant::Hello).await;
        assert_eq!(router.event().await, Event::Ready);
        router
    }
    async fn grant(&self, grant: Grant<std::os::fd::BorrowedFd<'_>>) {
        timeout(Duration::from_secs(2), send_grant(self.sender.as_ref().unwrap(), grant))
            .await
            .unwrap()
            .unwrap();
    }
    async fn event(&mut self) -> Event {
        let event = timeout(Duration::from_secs(5), Event::read(self.events.as_mut().unwrap()))
            .await
            .unwrap()
            .unwrap();
        if let Event::Accepted(id) | Event::Refused(id) = event {
            self.retained.remove(&id);
        }
        event
    }
    async fn pair(&mut self, id: u64) -> (TcpStream, UnixStream) {
        self.class_pair(id, Class::Expose).await
    }
    async fn class_pair(&mut self, id: u64, class: Class) -> (TcpStream, UnixStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (source, _) = listener.accept().await.unwrap();
        let (destination, peer) = StdUnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        self.grant(Grant::Connected {
            id,
            class,
            source: source.as_fd(),
            destination: destination.as_fd(),
        })
        .await;
        self.retained
            .insert(id, (source.into_std().unwrap().into(), destination.into()));
        (client, UnixStream::from_std(peer).unwrap())
    }
    async fn close(&mut self) {
        self.sender.take();
        self.events.take();
        timeout(Duration::from_secs(3), self.child.wait())
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn sandboxed_pair_preserves_half_close_without_a_listener_grant() {
    let mut router = Router::start().await;
    let (mut tcp, mut peer) = router.pair(1).await;
    assert_eq!(router.event().await, Event::Accepted(1));
    tcp.write_all(b"\x00\xffhello").await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(5), peer.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"\x00\xffhello");
    peer.write_all(b"reply after EOF").await.unwrap();
    peer.shutdown().await.unwrap();
    bytes.clear();
    timeout(Duration::from_secs(5), tcp.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"reply after EOF");
    assert_eq!(router.event().await, Event::Closed(1));
    router.close().await;
}

#[tokio::test]
async fn connection_limit_refuses_excess_pair_and_abort_frees_slot() {
    let mut router = Router::start().await;
    let mut peers = Vec::new();
    for id in 1..=CONNECTIONS_PER_CLASS as u64 {
        peers.push(router.pair(id).await);
        assert_eq!(router.event().await, Event::Accepted(id));
    }
    let excess = CONNECTIONS_PER_CLASS as u64 + 1;
    let (mut refused, _peer) = router.pair(excess).await;
    assert_eq!(router.event().await, Event::Refused(excess));
    assert_eq!(
        timeout(Duration::from_secs(2), refused.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    router.grant(Grant::Abort { id: 1 }).await;
    assert_eq!(router.event().await, Event::Closed(1));
    let replacement = router.pair(excess + 1).await;
    assert_eq!(router.event().await, Event::Accepted(excess + 1));
    peers.push(replacement);
    router.close().await;
    for (mut client, mut peer) in peers {
        assert_eq!(
            timeout(Duration::from_secs(2), client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(
            timeout(Duration::from_secs(2), peer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn duplicate_id_aborts_router_and_all_granted_endpoints() {
    let mut router = Router::start().await;
    let (mut client, _peer) = router.pair(1).await;
    assert_eq!(router.event().await, Event::Accepted(1));
    let _duplicate = router.pair(1).await;
    let status = timeout(Duration::from_secs(3), router.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
    assert_eq!(
        timeout(Duration::from_secs(2), client.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn unconnected_listener_descriptor_is_refused() {
    let mut router = Router::start().await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (data, _peer) = StdUnixStream::pair().unwrap();
    router
        .grant(Grant::Connected {
            id: 1,
            class: Class::Expose,
            source: listener.as_fd(),
            destination: data.as_fd(),
        })
        .await;
    assert_eq!(router.event().await, Event::Refused(1));
    router.close().await;
}

#[tokio::test]
async fn private_saturation_preserves_expose_reservation_without_borrowing() {
    let mut router = Router::start().await;
    let mut peers = Vec::new();
    for id in 1..=CONNECTIONS_PER_CLASS as u64 {
        peers.push(router.class_pair(id, Class::Private).await);
        assert_eq!(router.event().await, Event::Accepted(id));
    }
    let excess = CONNECTIONS_PER_CLASS as u64 + 1;
    let _refused = router.class_pair(excess, Class::Private).await;
    assert_eq!(
        router.event().await,
        Event::Refused(excess),
        "private borrowed expose capacity"
    );
    let (mut tcp, mut peer) = router.class_pair(excess + 1, Class::Expose).await;
    assert_eq!(router.event().await, Event::Accepted(excess + 1));
    tcp.write_all(b"ingress still works").await.unwrap();
    let mut bytes = [0; 19];
    timeout(Duration::from_secs(2), peer.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"ingress still works");
    router.close().await;
}
