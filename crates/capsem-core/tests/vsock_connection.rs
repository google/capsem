//! Public connection ownership works independently of either hypervisor backend.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

use capsem_core::VsockConnection;

#[test]
fn vsock_connection_can_be_sent_to_thread() {
    let (socket, mut peer) = UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let conn = VsockConnection::new(socket.as_raw_fd(), 5001, Box::new(socket));
    std::thread::spawn(move || {
        assert_eq!(conn.port, 5001);
        let mut duplicate = conn.try_clone_file().unwrap();
        drop(conn);
        duplicate.write_all(b"hello").unwrap();
    })
    .join()
    .unwrap();

    let mut payload = [0; 5];
    peer.read_exact(&mut payload).unwrap();
    assert_eq!(&payload, b"hello");
    assert_eq!(peer.read(&mut payload).unwrap(), 0);
}

#[test]
fn vsock_connection_can_be_stored_in_vec() {
    let mut conns = Vec::new();
    let mut peers = Vec::new();
    for port in 5000..5010 {
        let (socket, peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        conns.push(VsockConnection::new(socket.as_raw_fd(), port, Box::new(socket)));
        peers.push(peer);
    }
    assert_eq!(conns.len(), 10);
    assert_eq!(conns[5].port, 5005);
    let mut duplicate = conns[5].try_clone_file().unwrap();
    drop(conns);
    duplicate.write_all(b"hello").unwrap();
    drop(duplicate);

    let mut payload = [0; 5];
    peers[5].read_exact(&mut payload).unwrap();
    assert_eq!(&payload, b"hello");
    for mut peer in peers {
        assert_eq!(peer.read(&mut payload).unwrap(), 0);
    }
}
