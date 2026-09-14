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

#[tokio::test]
async fn configured_class_limits_are_independent_in_the_confined_child() {
    let mut router = Router::with_limits(2, 1).await;
    let mut peers = Vec::new();
    for (id, class, allowed) in [
        (1, Class::Private, true),
        (2, Class::Private, false),
        (3, Class::Expose, true),
        (4, Class::Expose, true),
        (5, Class::Expose, false),
    ] {
        peers.push(router.class_pair(id, class).await);
        assert_eq!(
            router.event().await,
            if allowed {
                Event::Accepted(id)
            } else {
                Event::Refused(id)
            }
        );
    }
    router.close().await;
    drop(peers);
}

#[tokio::test]
async fn process_kill_resets_tcp_without_running_stream_destructors() {
    let mut router = Router::start().await;
    let (mut client, _peer) = router.pair(1).await;
    assert_eq!(router.event().await, Event::Accepted(1));
    router.child.start_kill().unwrap();
    timeout(Duration::from_secs(2), router.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), client.read(&mut [0]))
            .await
            .unwrap()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::ConnectionReset
    );
    router.close().await;
}

#[tokio::test]
async fn abort_resets_tcp_after_the_parent_releases_its_shutdown_handle() {
    let mut router = Router::start().await;
    let (mut client, _peer) = router.pair(1).await;
    let retained = router.retained.remove(&1).unwrap();
    assert_eq!(router.event().await, Event::Accepted(1));
    router.grant(Grant::Abort { id: 1 }).await;
    let Event::Closed(1, report) = router.event().await else {
        panic!("missing close report")
    };
    assert_eq!(report.reason, capsem_router::CloseReason::Cancelled);
    assert!(
        timeout(Duration::from_millis(30), client.read(&mut [0])).await.is_err(),
        "child sent FIN before the last descriptor holder could reset TCP"
    );
    drop(retained);
    assert_eq!(
        timeout(Duration::from_secs(1), client.read(&mut [0]))
            .await
            .unwrap()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::ConnectionReset
    );
    router.close().await;
}

/// Read with a syscall: under the gate's sandbox, spawning setuid `ps` is
/// refused whatever the profile allows.
fn resident_bytes(pid: u32) -> usize {
    capsem_foundation::proctable::resident_bytes(pid).unwrap() as usize
}

#[tokio::test]
async fn stalled_destination_backpressures_tcp_with_bounded_router_rss() {
    let mut router = Router::start().await;
    let pid = router.child.id().unwrap();
    let before = resident_bytes(pid);
    let (mut client, _stalled_peer) = router.pair(1).await;
    assert_eq!(router.event().await, Event::Accepted(1));
    capsem_foundation::unix::fd::set_stream_buffers(
        client.as_fd(),
        capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
    )
    .unwrap();
    let writer = tokio::spawn(async move {
        let chunk = Box::new([0xab; 16 * 1024]);
        for _ in 0..1024 {
            client.write_all(chunk.as_slice()).await?;
        }
        Ok::<_, std::io::Error>(())
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !writer.is_finished(),
        "stalled peer did not close the upstream TCP window"
    );
    let after = resident_bytes(pid);
    eprintln!("router stalled-flow RSS before={before} after={after}");
    assert!(
        after.saturating_sub(before) < 8 * 1024 * 1024,
        "one stalled flow grew router RSS from {before} to {after}"
    );
    router.grant(Grant::Abort { id: 1 }).await;
    let Event::Closed(1, report) = router.event().await else {
        panic!("missing abort close report")
    };
    assert_eq!(report.reason, capsem_router::CloseReason::Cancelled);
    assert!(timeout(Duration::from_secs(2), writer).await.unwrap().unwrap().is_err());
    router.close().await;
}
impl Router {
    async fn start() -> Self {
        Self::with_limits(CONNECTIONS_PER_CLASS as u16, CONNECTIONS_PER_CLASS as u16).await
    }
    async fn with_limits(expose: u16, private: u16) -> Self {
        Self::spawn(&[
            "--expose-limit",
            &expose.to_string(),
            "--private-limit",
            &private.to_string(),
        ])
        .await
    }
    async fn spawn(args: &[&str]) -> Self {
        let (parent, child) = StdUnixStream::pair().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_capsem-router"))
            .args(["--parent-pid", &std::process::id().to_string()])
            .args(args)
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

fn frame(payload: &[u8]) -> Vec<u8> {
    [&(payload.len() as u32).to_be_bytes()[..], payload].concat()
}

/// Everything up to the end-of-stream frame on the guest's framed leg.
async fn read_frames(peer: &mut UnixStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let length = peer.read_u32().await.unwrap() as usize;
        if length == 0 {
            return bytes;
        }
        let start = bytes.len();
        bytes.resize(start + length, 0);
        peer.read_exact(&mut bytes[start..]).await.unwrap();
    }
}

/// The client's half-close crosses to the guest leg as an end-of-stream
/// frame, and the guest's reply after it comes back with a FIN.
#[tokio::test]
async fn sandboxed_pair_preserves_half_close_without_a_listener_grant() {
    let mut router = Router::start().await;
    let (mut tcp, mut peer) = router.pair(1).await;
    assert_eq!(router.event().await, Event::Accepted(1));
    tcp.write_all(b"\x00\xffhello").await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut bytes = timeout(Duration::from_secs(5), read_frames(&mut peer)).await.unwrap();
    assert_eq!(bytes, b"\x00\xffhello");
    peer.write_all(&[frame(b"reply after EOF"), frame(b"")].concat())
        .await
        .unwrap();
    bytes.clear();
    timeout(Duration::from_secs(5), tcp.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"reply after EOF");
    assert_eq!(
        router.event().await,
        Event::Closed(
            1,
            capsem_router::CloseReport {
                reason: capsem_router::CloseReason::Complete,
                from_source: 7,
                to_source: 15,
            }
        )
    );
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
    let (client, peer) = &mut peers[0];
    client.write_all(b"request").await.unwrap();
    let mut request = [0; 11];
    peer.read_exact(&mut request).await.unwrap();
    assert_eq!(request.to_vec(), frame(b"request"));
    peer.write_all(&frame(b"reply")).await.unwrap();
    client.read_exact(&mut [0; 5]).await.unwrap();
    router.grant(Grant::Abort { id: 1 }).await;
    assert_eq!(
        router.event().await,
        Event::Closed(
            1,
            capsem_router::CloseReport {
                reason: capsem_router::CloseReason::Cancelled,
                from_source: 7,
                to_source: 5,
            }
        )
    );
    let replacement = router.pair(excess + 1).await;
    assert_eq!(router.event().await, Event::Accepted(excess + 1));
    peers.push(replacement);
    router.close().await;
    for (mut client, mut peer) in peers {
        assert_eq!(
            timeout(Duration::from_secs(2), client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::ConnectionReset
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
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::ConnectionReset
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
    let mut bytes = [0; 23];
    timeout(Duration::from_secs(2), peer.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes.to_vec(), frame(b"ingress still works"));
    router.close().await;
}

/// One framed ethernet record on a cable: the switch reads only the MACs.
fn cable_record(destination: [u8; 6], source: Ipv4Addr, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&capsem_proto::privatelink::mac_of(source));
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    let mut record = (frame.len() as u16).to_be_bytes().to_vec();
    record.extend_from_slice(&frame);
    record
}

#[tokio::test]
async fn the_confined_network_switch_carries_tcp_and_floods_arp_between_plugged_cables() {
    use capsem_proto::privatelink::mac_of;
    let mut router = Router::spawn(&["--network"]).await;
    let alpha = Ipv4Addr::new(10, 128, 0, 2);
    let beta = Ipv4Addr::new(10, 128, 0, 3);
    let mut guests = Vec::new();
    for address in [alpha, beta] {
        let (switch_end, guest_end) = StdUnixStream::pair().unwrap();
        guest_end.set_nonblocking(true).unwrap();
        let port = capsem_router::port_id(1, address);
        router
            .grant(Grant::Plug {
                port,
                socket: switch_end.as_fd(),
            })
            .await;
        router
            .retained
            .insert(port, (switch_end.into(), StdUnixStream::pair().unwrap().0.into()));
        assert_eq!(router.event().await, Event::Accepted(port));
        guests.push(UnixStream::from_std(guest_end).unwrap());
    }
    let mut segment = vec![0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6];
    segment.resize(40, 0);
    segment.extend_from_slice(b"confined hop");
    for record in [
        cable_record(mac_of(beta), alpha, 0x0800, &segment),
        cable_record([0xff; 6], alpha, 0x0806, &[0; 28]),
    ] {
        guests[0].write_all(&record).await.unwrap();
        let mut received = vec![0u8; record.len()];
        timeout(Duration::from_secs(5), guests[1].read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, record);
    }
    router.close().await;
}
