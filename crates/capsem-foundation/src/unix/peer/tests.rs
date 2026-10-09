use super::*;
use crate::unix::{fd, process};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn current() -> PeerIdentity {
    PeerIdentity {
        pid: std::process::id().try_into().unwrap(),
        uid: process::current_uid(),
    }
}

#[test]
fn connected_unix_pairs_have_kernel_identity_and_remain_owned_by_the_caller() {
    let (mut left, mut right) = UnixStream::pair().unwrap();
    assert_eq!(identity(left.as_fd()).unwrap(), current());
    assert_eq!(identity(right.as_fd()).unwrap(), current());
    require(left.as_fd(), current()).unwrap();
    left.write_all(b"still open").unwrap();
    let mut bytes = [0; 10];
    right.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"still open");
}

#[test]
fn a_same_uid_child_cannot_claim_the_parent_process_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peer.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unix::peer::tests::connecting_child", "--nocapture"])
        .env_clear()
        .env("CAPSEM_PEER_TEST_SOCKET", &path)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    assert!(fd::wait_readable(listener.as_fd(), Duration::from_secs(5)).unwrap());
    let (mut socket, _) = listener.accept().unwrap();
    let child_identity = PeerIdentity {
        pid: child.id().try_into().unwrap(),
        uid: process::current_uid(),
    };
    let observed = identity(socket.as_fd());
    let refused = require(socket.as_fd(), current());
    let allowed = require(socket.as_fd(), child_identity);
    socket.write_all(b"done").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(observed.unwrap(), child_identity);
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    allowed.unwrap();
}

#[test]
fn connecting_child() {
    let Some(path) = std::env::var_os("CAPSEM_PEER_TEST_SOCKET") else {
        return;
    };
    let mut socket = UnixStream::connect(path).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reply = [0; 4];
    socket.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"done");
}

#[test]
fn mismatched_uid_is_refused_even_when_the_pid_matches() {
    let (socket, _peer) = UnixStream::pair().unwrap();
    let mut wrong = current();
    wrong.uid ^= 1;
    assert_eq!(
        require(socket.as_fd(), wrong).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn non_unix_streams_and_non_sockets_cannot_supply_identity() {
    let file = tempfile::tempfile().unwrap();
    let error = identity(file.as_fd()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(nix::errno::Errno::ENOTSOCK as i32));
    let (datagram, _) = UnixDatagram::pair().unwrap();
    assert_eq!(
        identity(datagram.as_fd()).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let dir = tempfile::tempdir().unwrap();
    let unconnected = UnixListener::bind(dir.path().join("listener.sock")).unwrap();
    assert_eq!(
        identity(unconnected.as_fd()).unwrap_err().raw_os_error(),
        Some(nix::errno::Errno::ENOTCONN as i32)
    );
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    assert_eq!(
        identity(socket.as_fd()).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}
