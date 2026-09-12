use super::*;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_core::security_engine::network::ledger::NetworkSecurity;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use std::os::fd::AsRawFd;
use std::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OWN: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);

/// A guest stream over a socket pair: the owner's end as a `VsockConnection`,
/// the guest's end for the test to watch.
fn guest_stream() -> (VsockConnection, tokio::net::UnixStream) {
    let (owner, guest) = std::os::unix::net::UnixStream::pair().unwrap();
    guest.set_nonblocking(true).unwrap();
    let raw = owner.as_raw_fd();
    let anchor: Box<dyn Send> = Box::new(owner);
    (
        VsockConnection::new(raw, capsem_proto::VSOCK_PORT_NETWORK, anchor),
        tokio::net::UnixStream::from_std(guest).unwrap(),
    )
}

fn authorized_publisher(action: &str) -> Arc<Publisher> {
    let text = format!(
        "[profiles.rules.private]\nname = \"private\"\naction = \"{action}\"\nmatch = 'network.mode == \"private\" && network.protocol == \"link\"'"
    );
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(&text).unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap();
    let engine = Arc::new(NetworkSecurity {
        db: Arc::new(capsem_logger::DbWriter::open_in_memory(128).unwrap()),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(std::collections::BTreeMap::new()))),
    });
    Arc::new(Publisher::default().with_security("vm-b".into(), "beta".into(), engine))
}

fn network() -> NetworkIdentity {
    NetworkIdentity::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "team".into()).unwrap()
}

/// The service's side of one link request: the token frame goes out, the
/// answer frame and its descriptor come back.
struct Service {
    sender: Sender,
    receiver: Receiver,
}

impl Service {
    fn asks(link: &Arc<PrivateLink>) -> Self {
        let (owner_side, service_side) = std::os::unix::net::UnixStream::pair().unwrap();
        owner_side.set_nonblocking(true).unwrap();
        let taking = Arc::clone(link);
        tokio::spawn(async move {
            let std = owner_side;
            let receiver = Receiver::new(std.try_clone().unwrap()).unwrap();
            let frame = receiver.recv().await.unwrap();
            let token = u64::from_be_bytes(frame.bytes[2..].try_into().unwrap());
            if let Err(error) = taking.take(token, std).await {
                tracing::debug!(%error, "link refused");
            }
        });
        Self {
            sender: Sender::new(service_side.try_clone().unwrap()).unwrap(),
            receiver: Receiver::new(service_side).unwrap(),
        }
    }

    async fn present(&self, token: u64) {
        self.sender.send(&encode_link_token(token), &[]).await.unwrap();
    }

    async fn stream(&self) -> Option<tokio::net::UnixStream> {
        let frame = tokio::time::timeout(Duration::from_secs(3), self.receiver.recv())
            .await
            .expect("the owner answers")
            .ok()?;
        assert_eq!(frame.bytes[1], FRAME_LINK);
        let fd = frame.fds.into_iter().next().expect("the answer carries the stream");
        let std = std::os::unix::net::UnixStream::from(fd);
        std.set_nonblocking(true).unwrap();
        Some(tokio::net::UnixStream::from_std(std).unwrap())
    }
}

#[tokio::test]
async fn a_reconnecting_guest_replaces_the_stream_the_owner_holds() {
    let link = PrivateLink::new(authorized_publisher("allow"), OWN);
    let (first, mut first_guest) = guest_stream();
    link.attach_guest(first);
    let (second, _second_guest) = guest_stream();
    link.attach_guest(second);
    let read = tokio::time::timeout(Duration::from_secs(1), first_guest.read(&mut [0u8; 1]))
        .await
        .expect("the replaced stream ends")
        .unwrap();
    assert_eq!(read, 0);
}

#[tokio::test]
async fn a_blocked_profile_refuses_the_link_and_a_token_is_expected_once() {
    let blocked = PrivateLink::new(authorized_publisher("block"), OWN);
    let refused = blocked.expect("00000000000000aa", network()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("block"), "{refused:#}");
    let unconfigured = PrivateLink::new(Arc::new(Publisher::default()), OWN);
    assert!(unconfigured.expect("00000000000000aa", network()).await.is_err());
    let allowed = PrivateLink::new(authorized_publisher("allow"), OWN);
    allowed.expect("00000000000000aa", network()).await.unwrap();
    assert!(
        allowed.expect("00000000000000aa", network()).await.is_err(),
        "the same token cannot be expected twice"
    );
    assert!(allowed.redeem(0xaa).is_some());
    assert!(allowed.redeem(0xaa).is_none(), "redeemed once");
}

#[tokio::test]
async fn the_service_gets_the_guest_stream_and_the_owner_lets_go_when_the_service_does() {
    let link = Arc::new(PrivateLink::new(authorized_publisher("allow"), OWN));
    link.expect("00000000000000bb", network()).await.unwrap();
    let (conn, mut guest) = guest_stream();
    link.attach_guest(conn);
    let service = Service::asks(&link);
    service.present(0xbb).await;
    let mut stream = service.stream().await.expect("the link is granted");
    stream.write_all(b"to the guest").await.unwrap();
    let mut heard = [0u8; 12];
    guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"to the guest");
    guest.write_all(b"from the guest").await.unwrap();
    let mut heard = [0u8; 14];
    stream.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"from the guest");
    // The service is done with the link: the owner ends the guest's stream
    // so the pump reconnects for the next attach.
    drop(service);
    drop(stream);
    let read = tokio::time::timeout(Duration::from_secs(2), guest.read(&mut [0u8; 1]))
        .await
        .expect("the guest stream ends")
        .unwrap();
    assert_eq!(read, 0);
    assert!(link.guest.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_link_asked_before_the_guest_connected_waits_for_it() {
    let link = Arc::new(PrivateLink::new(authorized_publisher("allow"), OWN));
    link.expect("00000000000000cc", network()).await.unwrap();
    let service = Service::asks(&link);
    service.present(0xcc).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (conn, mut guest) = guest_stream();
    link.attach_guest(conn);
    let mut stream = service.stream().await.expect("granted once the guest is there");
    stream.write_all(b"late").await.unwrap();
    let mut heard = [0u8; 4];
    guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"late");
}

#[tokio::test]
async fn an_unknown_token_gets_no_stream() {
    let link = Arc::new(PrivateLink::new(authorized_publisher("allow"), OWN));
    let (conn, _guest) = guest_stream();
    link.attach_guest(conn);
    let service = Service::asks(&link);
    service.present(0xdd).await;
    assert!(service.stream().await.is_none(), "the connection ends without a frame");
}
