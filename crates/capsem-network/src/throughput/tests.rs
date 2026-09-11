//! The server proven against a smoltcp client over an in-memory pipe: two
//! stacks, two addresses on one link, the same frames the VSOCK carries.
use super::*;
use crate::{Stack, LINK_MTU};
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Cidr};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

const SERVER: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 1);
const CLIENT: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const PORT: u16 = THROUGHPUT_PORT;

/// The bench client's behaviour, on smoltcp: one direction byte, then move
/// bytes until the deadline, then close.
struct Client {
    handle: Option<SocketHandle>,
    direction: Direction,
    deadline: Instant,
    header_sent: bool,
    sent: usize,
    received: usize,
    echoes: usize,
    closed: bool,
}

impl Client {
    fn new(direction: Direction, run_for: Duration) -> Self {
        Self {
            handle: None,
            direction,
            deadline: Instant::now() + run_for,
            header_sent: false,
            sent: 0,
            received: 0,
            echoes: 0,
            closed: false,
        }
    }
}

impl App for Client {
    fn attach(&mut self, iface: &mut Interface, sockets: &mut SocketSet<'static>) {
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]),
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]),
        );
        socket.set_nagle_enabled(false);
        socket
            .connect(iface.context(), IpEndpoint::new(IpAddress::Ipv4(SERVER), PORT), 49152)
            .unwrap();
        self.handle = Some(sockets.add(socket));
    }

    fn step(&mut self, sockets: &mut SocketSet<'static>) -> Progress {
        let socket = sockets.get_mut::<tcp::Socket>(self.handle.unwrap());
        if self.closed {
            // The client closes first, so it is the side that lands in
            // TIME-WAIT; its FIN being acknowledged is the end of the trial.
            // Keep draining meanwhile: the server's last window and its FIN
            // only arrive if this side reads them. (The bench client drops
            // the socket instead and the kernel resets; both end the slot.)
            let mut drained = false;
            let mut buffer = [0u8; 8192];
            while socket.can_recv() {
                drained |= socket.recv_slice(&mut buffer).unwrap_or(0) > 0;
            }
            return if matches!(socket.state(), tcp::State::Closed | tcp::State::TimeWait) {
                Progress::Done
            } else if drained {
                Progress::Changed
            } else {
                Progress::Idle
            };
        }
        if socket.state() == tcp::State::Closed {
            // Reset or refused before we closed: finish so the counts speak.
            return Progress::Done;
        }
        if !socket.is_active() {
            return Progress::Idle;
        }
        let mut moved = false;
        if !self.header_sent && socket.can_send() {
            socket.send_slice(&[self.direction.wire()]).unwrap();
            self.header_sent = true;
            moved = true;
        }
        let expired = Instant::now() >= self.deadline;
        match self.direction {
            Direction::Upload | Direction::Bidirectional if !expired => {
                while socket.can_send() {
                    let sent = socket.send_slice(&[0u8; 8192]).unwrap();
                    self.sent += sent;
                    moved |= sent > 0;
                }
            }
            Direction::Latency if !expired => {
                // One echo in flight: send when nothing is pending.
                if self.sent == self.received && socket.can_send() {
                    self.sent += socket.send_slice(&[0x5a; ECHO_BYTES]).unwrap();
                    moved = true;
                }
            }
            _ => {}
        }
        let mut buffer = [0u8; 8192];
        while socket.can_recv() {
            let read = socket.recv_slice(&mut buffer).unwrap();
            if self.direction == Direction::Latency {
                assert!(buffer[..read].iter().all(|byte| *byte == 0x5a), "echo corrupted");
                if self.received + read == self.sent {
                    self.echoes += 1;
                }
            }
            self.received += read;
            moved |= read > 0;
        }
        if expired && !self.closed {
            socket.close();
            self.closed = true;
            moved = true;
        }
        if moved {
            Progress::Changed
        } else {
            Progress::Idle
        }
    }
}

async fn run_lane(direction: Direction) -> (Client, ThroughputServer) {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let mut server = ThroughputServer::new(PORT);
    let mut client = Client::new(direction, Duration::from_millis(300));
    let serve = Stack::new(server_io, Ipv4Cidr::new(SERVER, 24), LINK_MTU).run(&mut server);
    let measure = Stack::new(client_io, Ipv4Cidr::new(CLIENT, 24), LINK_MTU).run(&mut client);
    let (served, measured) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(serve, measure) })
        .await
        .expect("both stacks finish");
    assert!(measured.unwrap(), "the client runs to its deadline");
    assert!(!served.unwrap(), "the server stops when the client's stream ends");
    (client, server)
}

#[tokio::test]
async fn upload_is_drained() {
    let (client, server) = run_lane(Direction::Upload).await;
    assert!(
        client.sent > SOCKET_BUFFER_BYTES,
        "more than one window moved: {}",
        client.sent
    );
    assert_eq!(client.received, 0);
    assert_eq!(server.served(), 1);
}

#[tokio::test]
async fn download_is_filled() {
    let (client, _) = run_lane(Direction::Download).await;
    assert!(client.received > SOCKET_BUFFER_BYTES, "{}", client.received);
    assert_eq!(client.sent, 0, "nothing but the header goes up");
}

#[tokio::test]
async fn bidirectional_moves_both_ways() {
    let (client, _) = run_lane(Direction::Bidirectional).await;
    assert!(client.sent > SOCKET_BUFFER_BYTES);
    assert!(client.received > SOCKET_BUFFER_BYTES);
}

#[tokio::test]
async fn latency_echoes_every_payload() {
    let (client, _) = run_lane(Direction::Latency).await;
    assert!(client.echoes > 10, "{}", client.echoes);
    assert_eq!(client.sent, client.received);
}

#[tokio::test]
async fn unknown_direction_is_reset_and_the_slot_listens_again() {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let mut server = ThroughputServer::new(PORT);
    struct Bad {
        handle: Option<SocketHandle>,
        sent: bool,
    }
    impl App for Bad {
        fn attach(&mut self, iface: &mut Interface, sockets: &mut SocketSet<'static>) {
            let mut socket = tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0u8; 4096]),
                tcp::SocketBuffer::new(vec![0u8; 4096]),
            );
            socket
                .connect(iface.context(), IpEndpoint::new(IpAddress::Ipv4(SERVER), PORT), 49153)
                .unwrap();
            self.handle = Some(sockets.add(socket));
        }
        fn step(&mut self, sockets: &mut SocketSet<'static>) -> Progress {
            let socket = sockets.get_mut::<tcp::Socket>(self.handle.unwrap());
            if !self.sent && socket.can_send() {
                socket.send_slice(&[9]).unwrap();
                self.sent = true;
                return Progress::Changed;
            }
            if self.sent && socket.state() == tcp::State::Closed {
                return Progress::Done;
            }
            Progress::Idle
        }
    }
    let mut bad = Bad {
        handle: None,
        sent: false,
    };
    let serve = Stack::new(server_io, Ipv4Cidr::new(SERVER, 24), LINK_MTU).run(&mut server);
    let attempt = Stack::new(client_io, Ipv4Cidr::new(CLIENT, 24), LINK_MTU).run(&mut bad);
    let (_, done) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(serve, attempt) })
        .await
        .unwrap();
    assert!(done.unwrap(), "the bad header is answered with a reset");
    assert_eq!((server.served(), server.rejected()), (1, 1));
}

#[tokio::test]
async fn garbage_frames_are_dropped_and_the_stream_end_stops_the_stack() {
    let (server_io, mut client_io) = tokio::io::duplex(1 << 16);
    let mut server = ThroughputServer::new(PORT);
    let serve = tokio::spawn(async move {
        Stack::new(server_io, Ipv4Cidr::new(SERVER, 24), LINK_MTU)
            .run(&mut server)
            .await
    });
    crate::frames::write_frame(&mut client_io, &[0xff; 40]).await.unwrap();
    crate::frames::write_frame(&mut client_io, &[0x45; 3]).await.unwrap();
    drop(client_io);
    let finished = tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap();
    assert!(!finished.unwrap(), "stream end, not application completion");
}

#[test]
fn direction_bytes_match_the_bench_client() {
    for (byte, direction) in [
        (0, Direction::Upload),
        (1, Direction::Download),
        (2, Direction::Bidirectional),
        (3, Direction::Latency),
    ] {
        assert_eq!(Direction::from_wire(byte), Some(direction));
        assert_eq!(direction.wire(), byte);
    }
    assert_eq!(Direction::from_wire(4), None);
}

#[tokio::test]
async fn serve_throughput_is_the_endpoint_on_the_gateway_address() {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let mut client = Client::new(Direction::Upload, Duration::from_millis(200));
    let serve = crate::serve_throughput(server_io);
    let measure = Stack::new(client_io, Ipv4Cidr::new(CLIENT, 24), LINK_MTU).run(&mut client);
    let (served, measured) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(serve, measure) })
        .await
        .unwrap();
    assert!(measured.unwrap());
    assert!(!served.unwrap(), "ends with the stream, like a VSOCK does");
    assert!(client.sent > SOCKET_BUFFER_BYTES, "{}", client.sent);
}
