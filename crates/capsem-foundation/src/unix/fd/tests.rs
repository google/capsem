use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;

use nix::fcntl::{fcntl, FcntlArg, FdFlag, OFlag};

use super::{duplicate, retry_eintr, set_nonblocking, shutdown, SocketShutdown};
use nix::errno::Errno;

#[test]
fn tcp_reset_waits_for_the_last_owned_descriptor_then_reaches_the_peer() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    let retained = duplicate(server.as_fd()).unwrap();
    let marked = super::tcp_reset_on_close(server.as_fd()).unwrap();
    drop(server);
    assert_eq!(
        client.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(retained);
    assert_eq!(
        client.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::ConnectionReset
    );
    assert!(marked);
}

#[test]
fn tcp_reset_does_not_change_unix_stream_close_semantics() {
    let (mut stream, mut peer) = UnixStream::pair().unwrap();
    assert!(!super::tcp_reset_on_close(stream.as_fd()).unwrap());
    stream.write_all(b"unchanged").unwrap();
    drop(stream);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"unchanged");
}

#[test]
fn tcp_reset_revokes_retained_copies_without_waiting_for_their_close() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    let mut retained = std::net::TcpStream::from(duplicate(server.as_fd()).unwrap());
    assert!(super::reset_tcp(server.as_fd()).unwrap());
    assert_eq!(
        client.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::ConnectionReset
    );
    assert!(retained.write(b"revoked").is_err());
    drop(retained);
    drop(server);
}

#[test]
fn clearing_armed_reset_restores_graceful_tcp_close() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    assert!(super::tcp_reset_on_close(server.as_fd()).unwrap());
    let cleared = super::tcp_clear_reset_on_close(server.as_fd()).unwrap();
    drop(server);
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
    assert!(cleared);
}

#[test]
fn stream_buffer_limits_replace_large_kernel_queues() {
    use nix::sys::socket::{getsockopt, setsockopt, sockopt};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_server, _) = listener.accept().unwrap();
    setsockopt(&client, sockopt::SndBuf, &(256 * 1024)).unwrap();
    setsockopt(&client, sockopt::RcvBuf, &(256 * 1024)).unwrap();
    super::set_stream_buffers(client.as_fd(), 32 * 1024).unwrap();
    // Linux reports doubled accounting space, while Darwin reports the request.
    assert!(getsockopt(&client, sockopt::SndBuf).unwrap() <= 64 * 1024);
    assert!(getsockopt(&client, sockopt::RcvBuf).unwrap() <= 64 * 1024);
}

#[test]
fn zero_buffer_limit_is_refused_without_changing_the_socket() {
    use nix::sys::socket::{getsockopt, sockopt};
    let (stream, _peer) = UnixStream::pair().unwrap();
    let before = (
        getsockopt(&stream, sockopt::SndBuf).unwrap(),
        getsockopt(&stream, sockopt::RcvBuf).unwrap(),
    );
    assert_eq!(
        super::set_stream_buffers(stream.as_fd(), 0).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        (
            getsockopt(&stream, sockopt::SndBuf).unwrap(),
            getsockopt(&stream, sockopt::RcvBuf).unwrap()
        ),
        before
    );
}

#[test]
fn duplicate_owns_an_independent_cloexec_descriptor() {
    let (mut writer, reader) = UnixStream::pair().unwrap();
    let duplicated = duplicate(reader.as_fd()).unwrap();
    let descriptor_flags = FdFlag::from_bits_truncate(fcntl(duplicated.as_raw_fd(), FcntlArg::F_GETFD).unwrap());
    assert!(descriptor_flags.contains(FdFlag::FD_CLOEXEC));

    drop(reader);
    writer.write_all(b"owned").unwrap();
    let mut duplicated = UnixStream::from(duplicated);
    let mut bytes = [0; 5];
    duplicated.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"owned");
}

#[test]
fn nonblocking_change_reports_previous_state_and_preserves_other_flags() {
    let (stream, _peer) = UnixStream::pair().unwrap();
    assert!(!set_nonblocking(stream.as_fd(), true).unwrap());
    assert!(set_nonblocking(stream.as_fd(), true).unwrap());

    let flags = OFlag::from_bits_truncate(fcntl(stream.as_raw_fd(), FcntlArg::F_GETFL).unwrap());
    assert!(flags.contains(OFlag::O_NONBLOCK));
    assert!(set_nonblocking(stream.as_fd(), false).unwrap());
    let restored = OFlag::from_bits_truncate(fcntl(stream.as_raw_fd(), FcntlArg::F_GETFL).unwrap());
    assert!(!restored.contains(OFlag::O_NONBLOCK));
    assert_eq!(flags - OFlag::O_NONBLOCK, restored);
}

#[test]
fn socket_shutdown_write_preserves_the_read_half() {
    let (mut local, mut peer) = UnixStream::pair().unwrap();
    local.write_all(b"before").unwrap();
    shutdown(local.as_fd(), SocketShutdown::Write).unwrap();

    let mut sent = [0; 6];
    peer.read_exact(&mut sent).unwrap();
    assert_eq!(&sent, b"before");
    let mut eof = [0; 1];
    assert_eq!(peer.read(&mut eof).unwrap(), 0);

    peer.write_all(b"reply").unwrap();
    let mut reply = [0; 5];
    local.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"reply");
}

#[test]
fn interrupted_descriptor_operation_is_retried_without_hiding_other_errno() {
    let mut attempts = 0;
    let value = retry_eintr(|| {
        attempts += 1;
        if attempts < 3 {
            Err(Errno::EINTR)
        } else {
            Ok(7)
        }
    })
    .unwrap();
    assert_eq!(value, 7);
    assert_eq!(attempts, 3);

    assert_eq!(retry_eintr::<()>(|| Err(Errno::EBADF)).unwrap_err(), Errno::EBADF);
}
