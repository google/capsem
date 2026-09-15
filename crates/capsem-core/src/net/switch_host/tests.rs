use super::*;
use capsem_foundation::unix::process::{probe, ProcessId, ProcessState};
use capsem_proto::privatelink::mac_of;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);

struct Harness {
    host: Arc<SwitchHost>,
    reports: mpsc::Receiver<(u64, PortReport)>,
    /// The in-process switch; it ends when the host closes the grant channel.
    switch: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// A host over an in-process switch -- the real protocol -- that also owns
/// `child`, standing in for the confined process a real start would spawn.
async fn harness(limit: usize, child: Option<tokio::process::Child>) -> Harness {
    let (parent, switch_end) = StdUnixStream::pair().unwrap();
    parent.set_nonblocking(true).unwrap();
    switch_end.set_nonblocking(true).unwrap();
    let grants = capsem_foundation::unix::router_channel::Receiver::new(switch_end.try_clone().unwrap()).unwrap();
    let switch = tokio::spawn(capsem_router::switch::run(
        grants,
        tokio::net::UnixStream::from_std(switch_end).unwrap(),
        limit,
    ));
    let (reports_sender, reports) = mpsc::channel(8);
    let host = SwitchHost::attach(
        capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap(),
        tokio::net::UnixStream::from_std(parent).unwrap(),
        reports_sender,
        child,
    );
    timeout(Duration::from_secs(2), host.ready()).await.unwrap().unwrap();
    Harness { host, reports, switch }
}

/// A long-lived stand-in for the switch process, not killed on drop: only
/// the host may end it.
fn stand_in() -> (tokio::process::Child, ProcessId) {
    let child = tokio::process::Command::new("/bin/sleep")
        .arg("600")
        .kill_on_drop(false)
        .spawn()
        .unwrap();
    let pid = ProcessId::try_from(child.id().unwrap()).unwrap();
    (child, pid)
}

/// Gone means reaped: an exited but unwaited child still probes alive.
async fn reaped(pid: ProcessId) -> bool {
    timeout(Duration::from_secs(5), async {
        while probe(pid).unwrap() == ProcessState::Alive {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

fn framed_tcp(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&mac_of(destination));
    frame.extend_from_slice(&mac_of(source));
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    let mut packet = vec![0u8; 40];
    packet[0] = 0x45;
    packet[9] = 6;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    frame.extend_from_slice(&packet);
    frame.extend_from_slice(payload);
    let mut record = (frame.len() as u16).to_be_bytes().to_vec();
    record.extend_from_slice(&frame);
    record
}

#[tokio::test]
async fn plugs_are_acknowledged_frames_cross_and_closes_are_reported() {
    let Harness { host, mut reports, .. } = harness(64, None).await;
    let (a_switch, a_guest) = StdUnixStream::pair().unwrap();
    let (b_switch, b_guest) = StdUnixStream::pair().unwrap();
    let a = host.plug(1, A, a_switch.as_fd()).await.unwrap();
    let b = host.plug(1, B, b_switch.as_fd()).await.unwrap();
    assert_eq!(
        (capsem_router::port_generation(a), capsem_router::port_address(a)),
        (1, A)
    );
    assert_ne!(a, b);
    drop((a_switch, b_switch));
    a_guest.set_nonblocking(true).unwrap();
    b_guest.set_nonblocking(true).unwrap();
    let mut a_guest = tokio::net::UnixStream::from_std(a_guest).unwrap();
    let mut b_guest = tokio::net::UnixStream::from_std(b_guest).unwrap();
    let record = framed_tcp(A, B, b"over the host");
    a_guest.write_all(&record).await.unwrap();
    let mut received = vec![0u8; record.len()];
    timeout(Duration::from_secs(2), b_guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, record);
    drop(a_guest);
    let (closed, report) = timeout(Duration::from_secs(2), reports.recv()).await.unwrap().unwrap();
    assert_eq!((closed, report.frames_in), (a, 1));
    host.unplug(b).await.unwrap();
    let (closed, report) = timeout(Duration::from_secs(2), reports.recv()).await.unwrap().unwrap();
    assert_eq!((closed, report.reason), (b, capsem_router::CloseReason::Cancelled));
    host.retire().await;
}

#[tokio::test]
async fn a_refused_plug_is_an_error_and_leaves_the_host_open() {
    let Harness { host, .. } = harness(1, None).await;
    let (a_switch, _a_guest) = StdUnixStream::pair().unwrap();
    let (b_switch, _b_guest) = StdUnixStream::pair().unwrap();
    host.plug(1, A, a_switch.as_fd()).await.unwrap();
    let refused = host.plug(1, B, b_switch.as_fd()).await.unwrap_err();
    assert!(refused.to_string().contains("refused"), "{refused:#}");
    assert!(!host.is_closed());
    host.retire().await;
}

/// Finding 2 of the PR #200 review: retiring a network cancelled a token
/// nothing watched, so the switch process, its runtime and its control
/// sockets lived until the service exited.
#[tokio::test]
async fn retiring_kills_and_reaps_the_switch_process() {
    let (child, pid) = stand_in();
    let Harness { host, .. } = harness(64, Some(child)).await;
    host.retire().await;
    assert!(host.is_closed());
    assert!(reaped(pid).await, "the retired switch process is still alive");
}

#[tokio::test]
async fn retiring_joins_every_task_and_closes_the_grant_channel() {
    let (child, _pid) = stand_in();
    let Harness { host, switch, .. } = harness(64, Some(child)).await;
    let weak = Arc::downgrade(&host);
    host.retire().await;
    drop(host);
    assert!(weak.upgrade().is_none(), "a task still holds the retired host");
    let ended = timeout(Duration::from_secs(2), switch).await;
    assert!(ended.is_ok(), "the switch still has a grant channel after retirement");
}

#[tokio::test]
async fn retiring_twice_and_plugging_after_retirement_are_harmless() {
    let Harness { host, .. } = harness(64, None).await;
    host.retire().await;
    host.retire().await;
    let (socket, _guest) = StdUnixStream::pair().unwrap();
    let error = host.plug(1, A, socket.as_fd()).await.unwrap_err();
    assert!(error.to_string().contains("closed"), "{error:#}");
    assert!(host.unplug(capsem_router::port_id(1, A)).await.is_err());
}

#[tokio::test]
async fn dropping_the_last_handle_without_retiring_still_stops_the_process() {
    let (child, pid) = stand_in();
    let Harness { host, .. } = harness(64, Some(child)).await;
    drop(host);
    assert!(reaped(pid).await, "an abandoned host left its switch process alive");
}

#[tokio::test]
async fn a_switch_that_dies_closes_the_host_and_is_reaped() {
    let (child, pid) = stand_in();
    let Harness { host, switch, .. } = harness(64, Some(child)).await;
    switch.abort();
    timeout(Duration::from_secs(2), host.closed())
        .await
        .expect("the host notices its switch is gone");
    assert!(reaped(pid).await, "a failed switch process was left behind");
    let (socket, _guest) = StdUnixStream::pair().unwrap();
    assert!(host.plug(1, A, socket.as_fd()).await.is_err());
    host.retire().await;
}
