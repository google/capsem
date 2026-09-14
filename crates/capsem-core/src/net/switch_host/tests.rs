use super::*;
use capsem_proto::privatelink::mac_of;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);

/// A switch host over an in-process switch: the real protocol, no child.
fn in_process(limit: usize) -> (Arc<SwitchHost>, mpsc::Receiver<(u64, capsem_router::CloseReport)>) {
    let (parent, child) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let grants = capsem_foundation::unix::router_channel::Receiver::new(child.try_clone().unwrap()).unwrap();
    tokio::spawn(capsem_router::switch::run(
        grants,
        tokio::net::UnixStream::from_std(child).unwrap(),
        limit,
    ));
    let (closed, reports) = mpsc::channel(8);
    let host = SwitchHost::attach(
        0,
        capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap(),
        tokio::net::UnixStream::from_std(parent).unwrap(),
        closed,
    );
    (host, reports)
}

fn framed_udp(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
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
    let mut record = (frame.len() as u16).to_be_bytes().to_vec();
    record.extend_from_slice(&frame);
    record
}

#[tokio::test]
async fn links_are_acknowledged_frames_cross_and_closes_are_reported() {
    let (host, mut reports) = in_process(64).await_ready().await;
    let (a_switch, a_guest) = StdUnixStream::pair().unwrap();
    let (b_switch, b_guest) = StdUnixStream::pair().unwrap();
    let a = host.link(A, a_switch.as_fd()).await.unwrap();
    let b = host.link(B, b_switch.as_fd()).await.unwrap();
    assert_ne!(a, b);
    assert_eq!(capsem_router::link_address(a), A);
    drop((a_switch, b_switch));
    a_guest.set_nonblocking(true).unwrap();
    b_guest.set_nonblocking(true).unwrap();
    let mut a_guest = tokio::net::UnixStream::from_std(a_guest).unwrap();
    let mut b_guest = tokio::net::UnixStream::from_std(b_guest).unwrap();
    let record = framed_udp(A, B, b"over the host");
    a_guest.write_all(&record).await.unwrap();
    let mut received = vec![0u8; record.len()];
    timeout(Duration::from_secs(2), b_guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, record);
    drop(a_guest);
    let (closed, report) = timeout(Duration::from_secs(2), reports.recv()).await.unwrap().unwrap();
    assert_eq!(closed, a);
    assert_eq!(report.from_source, 1);
    host.unlink(b).await.unwrap();
    let (closed, report) = timeout(Duration::from_secs(2), reports.recv()).await.unwrap().unwrap();
    assert_eq!((closed, report.reason), (b, capsem_router::CloseReason::Cancelled));
}

#[tokio::test]
async fn a_refused_link_is_an_error_and_a_dead_switch_ends_the_host() {
    let (host, _reports) = in_process(1).await_ready().await;
    let (a_switch, _a_guest) = StdUnixStream::pair().unwrap();
    let (b_switch, _b_guest) = StdUnixStream::pair().unwrap();
    host.link(A, a_switch.as_fd()).await.unwrap();
    let refused = host.link(B, b_switch.as_fd()).await.unwrap_err();
    assert!(refused.to_string().contains("refused"), "{refused:#}");
    assert!(!host.closed.is_cancelled());
    drop(host);
}

trait Ready {
    async fn await_ready(self) -> Self;
}

impl Ready for (Arc<SwitchHost>, mpsc::Receiver<(u64, capsem_router::CloseReport)>) {
    async fn await_ready(self) -> Self {
        timeout(Duration::from_secs(2), self.0.ready()).await.unwrap().unwrap();
        self
    }
}
