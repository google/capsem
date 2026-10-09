use std::io;
use std::net::IpAddr;
use std::os::fd::{AsFd, AsRawFd};
use std::time::Duration;

use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::bind_loopback;

#[test]
fn wildcard_and_nonloopback_addresses_are_refused_before_binding() {
    for address in ["0.0.0.0", "::", "192.0.2.1", "2001:db8::1"] {
        assert_eq!(
            bind_loopback(address.parse().unwrap()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[tokio::test]
async fn ipv4_and_ipv6_listeners_are_local_nonblocking_cloexec_and_owned() {
    for ip in ["127.0.0.1".parse::<IpAddr>().unwrap(), "::1".parse().unwrap()] {
        let listener = bind_loopback(ip).unwrap();
        let address = listener.local_addr().unwrap();
        assert_eq!(address.ip(), ip);
        assert_ne!(address.port(), 0);
        assert!(super::super::fd::set_nonblocking(listener.as_fd(), true).unwrap());
        let flags = fcntl(listener.as_raw_fd(), FcntlArg::F_GETFD).unwrap();
        assert!(FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC));
        assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
            let (mut server, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            client.write_all(b"callback").await.unwrap();
            let mut payload = [0u8; 8];
            server.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"callback");
        })
        .await
        .unwrap();
        drop(listener);
        assert!(
            tokio::net::TcpStream::connect(address).await.is_err(),
            "a dropped listener remained reachable"
        );
    }
}
