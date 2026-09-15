use super::*;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use capsem_network::switch::DropReason;
use capsem_proto::privatelink::mac_of;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);
const C: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 4);
const STRANGER: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 9);

fn ethernet(destination: [u8; 6], source: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A TCP segment from `source` to `destination`: the switch never parses it.
fn tcp(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0u8; 40];
    packet[0] = 0x45;
    packet[9] = 6;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    packet.extend_from_slice(payload);
    ethernet(mac_of(destination), mac_of(source), 0x0800, &packet)
}

fn arp_request(source: Ipv4Addr, target: Ipv4Addr) -> Vec<u8> {
    let mut body = vec![0, 1, 8, 0, 6, 4, 0, 1];
    body.extend_from_slice(&mac_of(source));
    body.extend_from_slice(&source.octets());
    body.extend_from_slice(&[0; 6]);
    body.extend_from_slice(&target.octets());
    ethernet([0xff; 6], mac_of(source), 0x0806, &body)
}

/// `frame` sent from `station`'s MAC, whatever else it claims.
fn sent_by(mut frame: Vec<u8>, station: Ipv4Addr) -> Vec<u8> {
    frame[6..12].copy_from_slice(&mac_of(station));
    frame
}

fn framed(frame: &[u8]) -> Vec<u8> {
    let mut record = (frame.len() as u16).to_be_bytes().to_vec();
    record.extend_from_slice(frame);
    record
}

async fn read_one(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(header))];
    stream.read_exact(&mut frame).await.unwrap();
    frame
}

async fn read_framed(stream: &mut UnixStream) -> Vec<u8> {
    timeout(Duration::from_secs(2), read_one(stream))
        .await
        .expect("a frame arrives")
}

async fn nothing_arrives(stream: &mut UnixStream) {
    assert!(
        timeout(Duration::from_millis(100), stream.read(&mut [0u8; 1]))
            .await
            .is_err(),
        "a frame arrived that should have been dropped"
    );
}

/// Read and discard until the switch closes its end; fails if it never does.
async fn closed_by_switch(stream: &mut UnixStream) {
    let drained = timeout(Duration::from_secs(5), async {
        let mut sink = vec![0u8; 1 << 16];
        loop {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
    assert!(drained.is_ok(), "the switch still holds the port's descriptor");
}

struct Switch {
    sender: Sender,
    events: UnixStream,
    task: tokio::task::JoinHandle<io::Result<()>>,
    /// The sender's copy of each granted cable, kept until the switch answers
    /// the grant: Darwin flushes a socket whose only reference is an in-flight
    /// SCM_RIGHTS message, and the switch would read an empty cable.
    granted: HashMap<u64, StdUnixStream>,
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
        Self {
            sender,
            events,
            task,
            granted: HashMap::new(),
        }
    }

    async fn event(&mut self) -> Event {
        let event = timeout(Duration::from_secs(5), Event::read(&mut self.events))
            .await
            .expect("an event")
            .unwrap();
        if let Event::Accepted(port) | Event::Refused(port) = event {
            self.granted.remove(&port);
        }
        event
    }

    /// Plug a cable in; returns the port id and the guest's end of the cable.
    async fn plug(&mut self, generation: u32, address: Ipv4Addr) -> (u64, UnixStream) {
        let (switch_end, guest_end) = StdUnixStream::pair().unwrap();
        guest_end.set_nonblocking(true).unwrap();
        let port = port_id(generation, address);
        send_grant(
            &self.sender,
            Grant::Plug {
                port,
                socket: switch_end.as_fd(),
            },
        )
        .await
        .unwrap();
        self.granted.insert(port, switch_end);
        (port, UnixStream::from_std(guest_end).unwrap())
    }

    async fn plugged(&mut self, generation: u32, address: Ipv4Addr) -> (u64, UnixStream) {
        let (port, cable) = self.plug(generation, address).await;
        assert_eq!(self.event().await, Event::Accepted(port));
        (port, cable)
    }

    async fn unplug(&mut self, port: u64) {
        send_grant(&self.sender, Grant::Unplug { port }).await.unwrap();
    }

    async fn closed(&mut self) -> (u64, PortReport) {
        match self.event().await {
            Event::PortClosed(port, report) => (port, report),
            other => panic!("expected a closed port, got {other:?}"),
        }
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
fn a_port_id_carries_its_generation_and_address() {
    let id = port_id(7, A);
    assert_eq!(port_generation(id), 7);
    assert_eq!(port_address(id), A);
    assert_ne!(port_id(7, B), id);
    assert_ne!(port_id(8, A), id);
}

#[tokio::test]
async fn tcp_crosses_and_an_arp_broadcast_floods_to_every_other_port() {
    let mut switch = Switch::start(64).await;
    let (_, mut a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    let (_, mut c) = switch.plugged(1, C).await;

    let segment = tcp(A, B, b"hello beta");
    a.write_all(&framed(&segment)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, segment);
    nothing_arrives(&mut c).await;

    let request = arp_request(A, B);
    a.write_all(&framed(&request)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, request);
    assert_eq!(read_framed(&mut c).await, request);
    nothing_arrives(&mut a).await;
    switch.stop().await;
}

#[tokio::test]
async fn a_forged_source_and_an_unknown_destination_go_nowhere_and_are_counted() {
    let mut switch = Switch::start(64).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;

    a.write_all(&framed(&tcp(B, A, b"forged"))).await.unwrap();
    a.write_all(&framed(&ethernet([0xff; 6], mac_of(STRANGER), 0x0806, &[0; 28])))
        .await
        .unwrap();
    // A's own MAC speaking as B: a packet from B's address, and an ARP
    // broadcast claiming it, which would draw B's traffic to A.
    a.write_all(&framed(&sent_by(tcp(B, B, b"as beta"), A))).await.unwrap();
    a.write_all(&framed(&sent_by(arp_request(B, A), A))).await.unwrap();
    a.write_all(&framed(&tcp(A, STRANGER, b"stranger"))).await.unwrap();
    let delivered = tcp(A, B, b"real");
    a.write_all(&framed(&delivered)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, delivered);
    nothing_arrives(&mut b).await;
    b.write_all(&framed(&tcp(B, A, b"back"))).await.unwrap();
    assert_eq!(read_framed(&mut a).await, tcp(B, A, b"back"));

    drop(a);
    let (closed, report) = switch.closed().await;
    assert_eq!(closed, port_a);
    assert_eq!(report.reason, CloseReason::Complete);
    assert_eq!((report.frames_in, report.frames_out), (6, 1));
    assert_eq!(report.bytes_out, framed(&tcp(B, A, b"back")).len() as u64);
    assert_eq!(report.dropped[DropReason::SourceMac as usize], 2);
    assert_eq!(report.dropped[DropReason::SourceAddress as usize], 2);
    assert_eq!(report.dropped[DropReason::Unknown as usize], 1);
    switch.stop().await;
}

#[tokio::test]
async fn a_newer_generation_replaces_the_cable_plugged_for_its_address() {
    let mut switch = Switch::start(64).await;
    let (first, mut old_a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    let (second, mut new_a) = switch.plug(2, A).await;
    let mut seen = vec![switch.event().await, switch.event().await];
    seen.sort_by_key(|event| matches!(event, Event::Accepted(_)));
    let [Event::PortClosed(closed, report), Event::Accepted(accepted)] = seen.as_slice() else {
        panic!("unexpected events {seen:?}")
    };
    assert_eq!(
        (*closed, report.reason, *accepted),
        (first, CloseReason::Cancelled, second)
    );
    closed_by_switch(&mut old_a).await;
    let segment = tcp(A, B, b"from the new cable");
    new_a.write_all(&framed(&segment)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, segment);
    switch.stop().await;
}

#[tokio::test]
async fn a_stale_generation_is_refused_and_the_current_cable_keeps_working() {
    let mut switch = Switch::start(64).await;
    let (_, mut a) = switch.plugged(5, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    for stale in [5, 4, 1] {
        let (port, _cable) = switch.plug(stale, A).await;
        assert_eq!(switch.event().await, Event::Refused(port));
    }
    let segment = tcp(A, B, b"still here");
    a.write_all(&framed(&segment)).await.unwrap();
    assert_eq!(read_framed(&mut b).await, segment);
    switch.stop().await;
}

#[tokio::test]
async fn the_port_limit_refuses_the_extra_cable_and_unplug_frees_it() {
    let mut switch = Switch::start(1).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    let (port_b, _b) = switch.plug(1, B).await;
    assert_eq!(switch.event().await, Event::Refused(port_b));
    switch.unplug(port_a).await;
    let (closed, report) = switch.closed().await;
    assert_eq!((closed, report.reason), (port_a, CloseReason::Cancelled));
    closed_by_switch(&mut a).await;
    switch.plugged(2, B).await;
    switch.stop().await;
}

#[tokio::test]
async fn unplugging_an_unknown_or_already_unplugged_port_is_harmless() {
    let mut switch = Switch::start(64).await;
    let (port_a, _a) = switch.plugged(1, A).await;
    switch.unplug(port_id(9, STRANGER)).await;
    switch.unplug(port_a).await;
    switch.unplug(port_a).await;
    assert_eq!(switch.closed().await.0, port_a);
    switch.plugged(2, A).await;
    switch.stop().await;
}

#[tokio::test]
async fn a_member_that_never_reads_does_not_stall_the_one_sending_to_it() {
    let mut switch = Switch::start(64).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    let (_, _b_never_reads) = switch.plugged(1, B).await;
    let record = framed(&tcp(A, B, &vec![0xab; 60_000]));
    let writer = async {
        for _ in 0..1000 {
            a.write_all(&record).await.unwrap();
        }
    };
    timeout(Duration::from_secs(10), writer)
        .await
        .expect("the sender keeps going while its peer is stalled");
    drop(a);
    let (closed, report) = switch.closed().await;
    assert_eq!(closed, port_a);
    assert_eq!(report.frames_in, 1000);
    assert!(report.dropped[DropReason::QueueFull as usize] > 0, "{report:?}");
    switch.stop().await;
}

/// Finding 3 of the PR #200 review: a member that stops reading pins its
/// writer in a blocked write. Unplugging it must still close its descriptor
/// and join that writer before the port is reported closed and its slot
/// reused -- otherwise every stalled replug leaks a socket, a task and a
/// queue's worth of frames outside the port limit.
#[tokio::test]
async fn unplugging_a_stalled_member_closes_its_cable_before_reporting_and_frees_the_slot() {
    let mut switch = Switch::start(2).await;
    let (_, mut a) = switch.plugged(1, A).await;
    let mut stalled_ports = Vec::new();
    for generation in 1..=5 {
        let (port_b, mut b) = switch.plugged(generation, B).await;
        // Fill B's socket buffer and its queue: its writer is now blocked.
        let record = framed(&tcp(A, B, &vec![0xcd; 60_000]));
        for _ in 0..(QUEUE_FRAMES * 4) {
            a.write_all(&record).await.unwrap();
        }
        switch.unplug(port_b).await;
        let (closed, report) = switch.closed().await;
        assert_eq!((closed, report.reason), (port_b, CloseReason::Cancelled));
        // Reported closed means already closed: the drain must end promptly.
        closed_by_switch(&mut b).await;
        stalled_ports.push(b);
    }
    // Five stalled replugs later the limit of two still admits B.
    switch.plugged(6, B).await;
    switch.stop().await;
}

#[tokio::test]
async fn a_port_queue_takes_256_small_frames_and_refuses_the_next() {
    // 64 frames could not absorb one 256 KiB read of small frames: at 1500
    // bytes a frame that is 170 frames for a single port.
    let (queue, _frames) = Queue::new();
    let small = Bytes::from(vec![0u8; 1502]);
    for index in 0..256 {
        assert!(queue.offer(small.clone()), "frame {index} fits");
    }
    assert!(!queue.offer(small), "the 257th frame is refused");
}

#[tokio::test]
async fn a_port_queue_holds_no_more_bytes_than_64_full_frames() {
    let (queue, _frames) = Queue::new();
    let full = Bytes::from(vec![0u8; HEADER_BYTES + u16::MAX as usize]);
    for index in 0..64 {
        assert!(queue.offer(full.clone()), "full frame {index} fits");
    }
    assert!(!queue.offer(full), "a 65th full frame would outgrow the byte cap");
}

#[tokio::test]
async fn a_batch_in_flight_counts_against_the_queue_until_its_bytes_are_released() {
    let (queue, mut frames) = Queue::new();
    let full = Bytes::from(vec![0u8; HEADER_BYTES + u16::MAX as usize]);
    for _ in 0..64 {
        assert!(queue.offer(full.clone()));
    }
    let mut batch = Vec::new();
    assert_eq!(frames.recv_many(&mut batch, QUEUE_FRAMES).await, 64);
    assert!(
        !queue.offer(full.clone()),
        "the writer holds every byte until it has written them"
    );
    queue.release(batch.iter().map(Bytes::len).sum());
    assert!(queue.offer(full), "written bytes make room again");
}

#[tokio::test]
async fn a_writer_releases_what_it_wrote_so_a_long_transfer_never_fills_the_queue() {
    // Over three times the byte cap of full frames, one at a time: each is
    // read before the next is sent, so the queue never holds more than one.
    // A writer that kept the bytes of what it wrote would refuse every frame
    // after the 64th. (Sent all at once, a sender faster than the receiver
    // loses frames by design: the switch drops, it does not push back.)
    let mut switch = Switch::start(64).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    let frame = tcp(A, B, &vec![0x5a; u16::MAX as usize - 14 - 40]);
    for _ in 0..200 {
        a.write_all(&framed(&frame)).await.unwrap();
        assert_eq!(read_framed(&mut b).await.len(), frame.len());
    }
    drop(a);
    let (closed, report) = switch.closed().await;
    assert_eq!(closed, port_a);
    assert_eq!(report.dropped[DropReason::QueueFull as usize], 0, "{report:?}");
    switch.stop().await;
}

#[tokio::test]
async fn a_burst_of_small_frames_arrives_complete_and_in_order() {
    let mut switch = Switch::start(64).await;
    let (_, mut a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    let frames: Vec<Vec<u8>> = (0..QUEUE_FRAMES as u32 / 2)
        .map(|i| tcp(A, B, &i.to_be_bytes()))
        .collect();
    let burst: Vec<u8> = frames.iter().flat_map(|frame| framed(frame)).collect();
    a.write_all(&burst).await.unwrap();
    for frame in &frames {
        assert_eq!(&read_framed(&mut b).await, frame);
    }
    switch.stop().await;
}

#[tokio::test]
async fn a_broadcast_storm_is_capped_and_counted() {
    let mut switch = Switch::start(64).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    let (_, mut b) = switch.plugged(1, B).await;
    let request = framed(&arp_request(A, B));
    let storm = (BROADCASTS_PER_SECOND * 3) as usize;
    let received = tokio::spawn(async move {
        let mut count = 0usize;
        while timeout(Duration::from_millis(300), read_one(&mut b)).await.is_ok() {
            count += 1;
        }
        // Kept open: the report under test is A's, not B's closing.
        (count, b)
    });
    let burst: Vec<u8> = std::iter::repeat_n(request, storm).flatten().collect();
    a.write_all(&burst).await.unwrap();
    let (count, _b) = received.await.unwrap();
    assert!(count >= 1, "ARP must still cross");
    assert!(
        count <= BROADCASTS_PER_SECOND as usize * 2,
        "{count} broadcasts crossed"
    );
    drop(a);
    let (closed, report) = switch.closed().await;
    assert_eq!(closed, port_a);
    assert!(report.dropped[DropReason::Storm as usize] > 0, "{report:?}");
    switch.stop().await;
}

#[tokio::test]
async fn an_empty_record_ends_the_port_as_an_error() {
    let mut switch = Switch::start(64).await;
    let (port_a, mut a) = switch.plugged(1, A).await;
    a.write_all(&[0, 0]).await.unwrap();
    let (closed, report) = switch.closed().await;
    assert_eq!((closed, report.reason), (port_a, CloseReason::Io));
    closed_by_switch(&mut a).await;
    switch.stop().await;
}

/// `Accepted` is the owner's go-ahead: a member may send the moment it sees
/// it, so the port must be in the table before that event is written. A
/// full event socket holds the switch inside the write and makes the window
/// deterministic.
#[tokio::test]
async fn a_member_is_reachable_before_its_port_is_reported_accepted() {
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
    let mut switch = Switch {
        sender,
        events,
        task,
        granted: HashMap::new(),
    };

    let (_, mut a) = switch.plugged(1, A).await;
    filler.set_nonblocking(true).unwrap();
    while filler.write(&[0u8; 4096]).is_ok() {}
    let (_, mut b) = switch.plug(1, B).await;

    let segment = tcp(A, B, b"sent as soon as plugged");
    let arrived = timeout(Duration::from_secs(2), async {
        loop {
            a.write_all(&framed(&segment)).await.unwrap();
            if let Ok(frame) = timeout(Duration::from_millis(20), read_one(&mut b)).await {
                return frame;
            }
        }
    })
    .await
    .expect("a member plugged but not yet reported must already receive");
    assert_eq!(arrived, segment);
    switch.task.abort();
}
