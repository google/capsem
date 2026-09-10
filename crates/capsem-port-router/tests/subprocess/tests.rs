use capsem_port_router::{send_grant, Event, Grant, MAX_CONNECTIONS};
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::process::{Child, Command, Stdio};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};
use tokio::time::{timeout, Duration};

struct Router {
    child: Child,
    sender: Option<capsem_foundation::unix::router_channel::Sender>,
    events: Option<UnixStream>,
    address: std::net::SocketAddr,
}

impl Router {
    async fn start() -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (parent, child) = StdUnixStream::pair().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_capsem-port-router"))
            .args(["--parent-pid", &std::process::id().to_string()])
            .env_clear()
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child)))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        parent.set_nonblocking(true).unwrap();
        let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
        let events = UnixStream::from_std(parent).unwrap();
        let mut router = Self {
            child,
            sender: Some(sender),
            events: Some(events),
            address,
        };
        send_grant(
            router.sender.as_ref().unwrap(),
            Grant::Listen {
                socket: listener.as_fd(),
            },
        )
        .await
        .unwrap();
        assert_eq!(router.event().await, Event::Ready);
        router
    }

    async fn event(&mut self) -> Event {
        timeout(Duration::from_secs(5), Event::read(self.events.as_mut().unwrap()))
            .await
            .unwrap()
            .unwrap()
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
    }
}

#[tokio::test]
async fn sandboxed_router_accepts_data_descriptors_and_preserves_half_close() {
    let mut router = Router::start().await;
    let mut tcp = TcpStream::connect(router.address).await.unwrap();
    let Event::Open(id) = router.event().await else {
        panic!("missing connection request");
    };
    let (data, echo) = StdUnixStream::pair().unwrap();
    send_grant(
        router.sender.as_ref().unwrap(),
        Grant::Connected {
            id,
            socket: data.as_fd(),
        },
    )
    .await
    .unwrap();
    echo.set_nonblocking(true).unwrap();
    let mut echo = UnixStream::from_std(echo).unwrap();
    tcp.write_all(b"\x00\xffhello").await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(5), echo.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"\x00\xffhello");
    echo.write_all(b"reply after EOF").await.unwrap();
    echo.shutdown().await.unwrap();
    bytes.clear();
    timeout(Duration::from_secs(5), tcp.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"reply after EOF");
    assert_eq!(router.event().await, Event::Closed(id));
    router.sender.take();
    router.events.take();
    timeout(Duration::from_secs(3), async {
        while router.child.try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        TcpStream::connect(router.address).await.is_err(),
        "listener survived parent channel close"
    );
}

#[tokio::test]
async fn connection_limit_applies_backpressure_and_refusal_frees_a_slot() {
    let mut router = Router::start().await;
    let mut clients = Vec::new();
    let mut first = 0;
    for _ in 0..MAX_CONNECTIONS {
        clients.push(TcpStream::connect(router.address).await.unwrap());
        let Event::Open(id) = router.event().await else {
            panic!("missing open");
        };
        if first == 0 {
            first = id;
        }
    }
    clients.push(TcpStream::connect(router.address).await.unwrap());
    assert!(
        timeout(Duration::from_millis(100), Event::read(router.events.as_mut().unwrap()))
            .await
            .is_err()
    );
    send_grant(router.sender.as_ref().unwrap(), Grant::Refused { id: first })
        .await
        .unwrap();
    assert_eq!(router.event().await, Event::Closed(first));
    assert!(matches!(router.event().await, Event::Open(_)));
}
