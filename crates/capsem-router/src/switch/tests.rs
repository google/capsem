use super::*;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use capsem_proto::privatelink::mac_of;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);

fn udp(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&mac_of(destination));
    frame.extend_from_slice(&mac_of(source));
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    let mut packet = vec![0u8; 20];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    frame.extend_from_slice(&packet);
    frame.extend_from_slice(payload);
    frame
}

fn arp_request(source: Ipv4Addr, target: Ipv4Addr) -> Vec<u8> {
    let mut frame = vec![0xff; 6];
    frame.extend_from_slice(&mac_of(source));
    frame.extend_from_slice(&0x0806u16.to_be_bytes());
    frame.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
    frame.extend_from_slice(&mac_of(source));
    frame.extend_from_slice(&source.octets());
    frame.extend_from_slice(&[0; 6]);
    frame.extend_from_slice(&target.octets());
    frame
}

fn framed(frame: &[u8]) -> Vec<u8> {
    let mut record = (frame.len() as u16).to_be_bytes().to_vec();
    record.extend_from_slice(frame);
    record
}

async fn read_framed(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0u8; 2];
    timeout(Duration::from_secs(2), stream.read_exact(&mut header))
        .await
        .expect("a frame arrives")
        .unwrap();
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(header))];
    stream.read_exact(&mut frame).await.unwrap();
    frame
}

async fn read_one(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(header))];
    stream.read_exact(&mut frame).await.unwrap();
    frame
}

async fn nothing_arrives(stream: &mut UnixStream) {
    assert!(
        timeout(Duration::from_millis(100), stream.read(&mut [0u8; 1]))
            .await
            .is_err(),
        "a frame arrived that should have been dropped"
    );
}

struct Switch {
    sender: Sender,
    events: UnixStream,
    task: tokio::task::JoinHandle<io::Result<()>>,
}

impl Switch {
    async fn start(limit: usize) -> Self {
        let (parent, child) = StdUnixStream::pair().unwrap();
        let sender = Sender::new(parent.try_clone().unwrap()).unwrap();
        let receiver = Receiver::new(child.try_clone().unwrap()).unwrap();
        parent.set_nonblocking(true).unwrap();
        child.set_nonblocking(true).unwrap();
        let mut events = UnixStream::from_std(parent).unwrap();
        let task = tokio::spawn(run(receiver, UnixStream::from_std(child).unwrap(), limit));
        assert_eq!(Event::read(&mut events).await.unwrap(), Event::Ready);
        Self { sender, events, task }
    }

    async fn event(&mut self) -> Event {
        timeout(Duration::from_secs(5), Event::read(&mut self.events))
            .await
            .expect("an event")
            .unwrap()
    }

    /// Link a member; returns its id and the guest's end of the stream.
    async fn link(&mut self, seq: u32, address: Ipv4Addr) -> (u64, UnixStream) {
        let (switch_end, guest_end) = StdUnixStream::pair().unwrap();
        guest_end.set_nonblocking(true).unwrap();
        let id = link_id(seq, address);
        send_grant(
            &self.sender,
            Grant::Link {
                id,
                socket: switch_end.as_fd(),
            },
        )
        .await
        .unwrap();
        (id, UnixStream::from_std(guest_end).unwrap())
    }

    async fn stop(self) {
        drop(self.sender);
        drop(self.events);
        assert!(timeout(Duration::from_secs(2), self.task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}

#[test]
fn a_link_id_carries_its_sequence_and_address() {
    let id = link_id(7, A);
    assert_eq!(id >> 32, 7);
    assert_eq!(link_address(id), A);
    assert_ne!(link_id(7, B), id);
    assert_ne!(link_id(8, A), id);
}

#[tokio::test]
async fn frames_cross_between_linked_members_and_arp_is_answered_by_the_switch() {
    let mut switch = Switch::start(64).await;
    let (id_a, mut a) = switch.link(1, A).await;
    assert_eq!(switch.event().await, Event::Accepted(id_a));
    let (id_b, mut b) = switch.link(2, B).await;
    assert_eq!(switch.event().await, Event::Accepted(id_b));

    let datagram = udp(A, B, b"hello beta");
    a.write_all(&framed(&datagram)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, datagram);

    a.write_all(&framed(&arp_request(A, B))).await.unwrap();
    let reply = read_framed(&mut a).await;
    assert_eq!(&reply[..6], &mac_of(A));
    assert_eq!(&reply[6..12], &mac_of(B));
    assert_eq!(reply[21], 2, "an ARP reply");
    nothing_arrives(&mut b).await;

    // A forged source and an unknown destination go nowhere.
    a.write_all(&framed(&udp(B, A, b"forged"))).await.unwrap();
    a.write_all(&framed(&udp(A, Ipv4Addr::new(10, 128, 0, 9), b"stranger")))
        .await
        .unwrap();
    nothing_arrives(&mut b).await;
    nothing_arrives(&mut a).await;

    drop(a);
    assert_eq!(
        switch.event().await,
        Event::Closed(
            id_a,
            CloseReport {
                reason: CloseReason::Complete,
                from_source: 1,
                to_source: 1
            }
        )
    );
    // B is alone now: its frames to A are unknown, and it still lives.
    b.write_all(&framed(&udp(B, A, b"late"))).await.unwrap();
    nothing_arrives(&mut b).await;
    switch.stop().await;
}

#[tokio::test]
async fn a_relinked_address_replaces_the_stream_it_had() {
    let mut switch = Switch::start(64).await;
    let (first, mut old_a) = switch.link(1, A).await;
    assert_eq!(switch.event().await, Event::Accepted(first));
    let (id_b, mut b) = switch.link(2, B).await;
    assert_eq!(switch.event().await, Event::Accepted(id_b));
    let (second, mut new_a) = switch.link(3, A).await;
    // The old stream ends and the new one is accepted; the two reports race.
    let mut seen = vec![switch.event().await, switch.event().await];
    seen.sort_by_key(|event| matches!(event, Event::Accepted(_)));
    let [Event::Closed(closed, report), Event::Accepted(accepted)] = seen.as_slice() else {
        panic!("unexpected events {seen:?}")
    };
    assert_eq!(
        (*closed, report.reason, *accepted),
        (first, CloseReason::Cancelled, second)
    );
    assert_eq!(
        timeout(Duration::from_secs(1), old_a.read(&mut [0u8; 1]))
            .await
            .unwrap()
            .unwrap(),
        0,
        "the replaced stream is closed"
    );
    let datagram = udp(A, B, b"from the new stream");
    new_a.write_all(&framed(&datagram)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, datagram);
    switch.stop().await;
}

#[tokio::test]
async fn the_link_limit_refuses_the_extra_member_and_abort_unlinks() {
    let mut switch = Switch::start(1).await;
    let (id_a, mut a) = switch.link(1, A).await;
    assert_eq!(switch.event().await, Event::Accepted(id_a));
    let (id_b, _b) = switch.link(2, B).await;
    assert_eq!(switch.event().await, Event::Refused(id_b));
    send_grant(&switch.sender, Grant::Abort { id: id_a }).await.unwrap();
    let Event::Closed(closed, report) = switch.event().await else {
        panic!("abort closes the link")
    };
    assert_eq!((closed, report.reason), (id_a, CloseReason::Cancelled));
    assert_eq!(
        timeout(Duration::from_secs(1), a.read(&mut [0u8; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    switch.stop().await;
}

#[tokio::test]
async fn a_member_that_never_reads_does_not_stall_the_one_sending_to_it() {
    let mut switch = Switch::start(64).await;
    let (id_a, mut a) = switch.link(1, A).await;
    assert_eq!(switch.event().await, Event::Accepted(id_a));
    let (id_b, _b_never_reads) = switch.link(2, B).await;
    assert_eq!(switch.event().await, Event::Accepted(id_b));
    let record = framed(&udp(A, B, &vec![0xab; 60_000]));
    let writer = async {
        for _ in 0..1000 {
            a.write_all(&record).await.unwrap();
        }
    };
    timeout(Duration::from_secs(10), writer)
        .await
        .expect("the sender keeps going while its peer is stalled");
    drop(a);
    let Event::Closed(closed, report) = switch.event().await else {
        panic!("the sender's close is reported")
    };
    assert_eq!(closed, id_a);
    assert!((QUEUE_FRAMES as u64..=1000).contains(&report.from_source), "{report:?}");
    switch.stop().await;
}

/// `Accepted` is the owner's go-ahead: a member may send to the link the
/// moment it sees it. The switch wrote it before entering the link in its
/// table, so a frame that fast was addressed to nobody and dropped -- the
/// subprocess test lost one under Linux load. Holding the switch inside that
/// write (a full event socket) makes the window deterministic.
#[tokio::test]
async fn a_member_is_reachable_before_its_link_is_reported_accepted() {
    use std::io::Write;

    let (parent, child) = StdUnixStream::pair().unwrap();
    let sender = Sender::new(parent.try_clone().unwrap()).unwrap();
    let receiver = Receiver::new(child.try_clone().unwrap()).unwrap();
    let mut filler = child.try_clone().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let mut events = UnixStream::from_std(parent).unwrap();
    let task = tokio::spawn(run(receiver, UnixStream::from_std(child).unwrap(), 64));
    assert_eq!(Event::read(&mut events).await.unwrap(), Event::Ready);
    let mut switch = Switch { sender, events, task };

    let (id_a, mut a) = switch.link(1, A).await;
    assert_eq!(switch.event().await, Event::Accepted(id_a));
    // Nothing reads events now, so the next event write cannot complete.
    filler.set_nonblocking(true).unwrap();
    while filler.write(&[0u8; 4096]).is_ok() {}
    let (_, mut b) = switch.link(2, B).await;

    // The grant is processed at some point after it is sent; until then a
    // frame to B is rightly dropped. So keep sending: once B is linked one
    // arrives -- unless linking waits on the event write that cannot finish.
    let datagram = udp(A, B, b"sent as soon as linked");
    let arrived = timeout(Duration::from_secs(2), async {
        loop {
            a.write_all(&framed(&datagram)).await.unwrap();
            if let Ok(frame) = timeout(Duration::from_millis(20), read_one(&mut b)).await {
                return frame;
            }
        }
    })
    .await
    .expect("a member linked but not yet reported must already receive");
    assert_eq!(arrived, datagram);
    switch.task.abort();
}
