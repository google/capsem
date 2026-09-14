use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A pump's connection as the owner sees it: a `VsockConnection` over a
/// socket pair, and the pump's end.
fn pump_connection() -> (VsockConnection, tokio::net::UnixStream) {
    use std::os::fd::AsRawFd;
    let (owner, pump) = std::os::unix::net::UnixStream::pair().unwrap();
    pump.set_nonblocking(true).unwrap();
    let raw = owner.as_raw_fd();
    let anchor: Box<dyn Send> = Box::new(owner);
    (
        VsockConnection::new(raw, capsem_proto::VSOCK_PORT_NETWORK, anchor),
        tokio::net::UnixStream::from_std(pump).unwrap(),
    )
}

#[tokio::test]
async fn a_cable_header_is_read_exactly_and_the_frames_after_it_stay_for_the_switch() {
    let (conn, mut pump) = pump_connection();
    let mut segment = capsem_proto::privatelink::cable_header(3).to_vec();
    segment.extend_from_slice(b"the first frame, in the same segment");
    pump.write_all(&segment).await.unwrap();
    assert_eq!(read_cable_header(&conn).await, Ok(3));
    let fd = conn.try_clone_fd().unwrap();
    let rest = std::os::unix::net::UnixStream::from(fd);
    rest.set_nonblocking(true).unwrap();
    let mut rest = tokio::net::UnixStream::from_std(rest).unwrap();
    let mut frame = [0u8; 36];
    rest.read_exact(&mut frame).await.unwrap();
    assert_eq!(&frame, b"the first frame, in the same segment");
    pump.write_all(b"!").await.unwrap();
    let mut more = [0u8; 1];
    rest.read_exact(&mut more).await.unwrap();
    assert_eq!(&more, b"!", "reading the header shut nothing down");
}

#[tokio::test]
async fn a_pump_that_names_no_cable_is_refused() {
    let (conn, mut pump) = pump_connection();
    pump.write_all(&[0, 0, 0, 0]).await.unwrap();
    assert!(read_cable_header(&conn).await.is_err(), "cable 0 is no cable");
    let (conn, pump) = pump_connection();
    drop(pump);
    assert!(read_cable_header(&conn).await.is_err(), "a closed pump names nothing");
    let (conn, _silent) = pump_connection();
    let started = std::time::Instant::now();
    assert!(read_cable_header(&conn).await.is_err(), "a silent pump times out");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
