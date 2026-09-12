use super::*;
use std::os::fd::AsRawFd;
use tokio::io::AsyncReadExt;

/// A guest stream over a socket pair: the owner's end as a `VsockConnection`,
/// the guest's end for the test to watch.
fn guest_stream() -> (VsockConnection, tokio::net::UnixStream) {
    let (owner, guest) = std::os::unix::net::UnixStream::pair().unwrap();
    guest.set_nonblocking(true).unwrap();
    let raw = owner.as_raw_fd();
    let owned = std::os::fd::OwnedFd::from(owner);
    assert_eq!(owned.as_raw_fd(), raw);
    let anchor: Box<dyn Send> = Box::new(owned);
    (
        VsockConnection::new(raw, capsem_proto::VSOCK_PORT_NETWORK, anchor),
        tokio::net::UnixStream::from_std(guest).unwrap(),
    )
}

#[tokio::test]
async fn a_reconnecting_guest_replaces_the_stream_the_owner_holds() {
    let link = PrivateLink::new();
    let (first, mut first_guest) = guest_stream();
    link.attach_guest(first);
    let (second, _second_guest) = guest_stream();
    link.attach_guest(second);
    // The first stream is gone from the owner: its guest end reads EOF.
    let read = tokio::time::timeout(std::time::Duration::from_secs(1), first_guest.read(&mut [0u8; 1]))
        .await
        .expect("the replaced stream ends")
        .unwrap();
    assert_eq!(read, 0);
    assert!(link.guest.lock().unwrap().is_some());
}

#[tokio::test]
async fn the_held_stream_stays_open_until_replaced() {
    let link = PrivateLink::new();
    let (conn, mut guest) = guest_stream();
    link.attach_guest(conn);
    let pending = tokio::time::timeout(std::time::Duration::from_millis(100), guest.read(&mut [0u8; 1])).await;
    assert!(pending.is_err(), "nothing ends the held stream");
}
