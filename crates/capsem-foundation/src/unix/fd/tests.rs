use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;

use nix::fcntl::{fcntl, FcntlArg, FdFlag, OFlag};

use super::{duplicate, retry_eintr, set_nonblocking, shutdown, wait_readable, SocketShutdown};
use nix::errno::Errno;

#[test]
fn inherited_descriptor_policy_keeps_only_stdio_and_explicit_grants() {
    for descriptor in 0..=2 {
        assert!(!super::should_close_inherited_descriptor(descriptor, &[]));
    }
    assert!(super::should_close_inherited_descriptor(7, &[]));
    assert!(!super::should_close_inherited_descriptor(7, &[7, 11]));
    assert!(super::should_close_inherited_descriptor(9, &[7, 11]));
}

#[test]
fn isolated_exec_child_closes_ambient_descriptor_and_keeps_declared_grant() {
    const AMBIENT: &str = "CAPSEM_TEST_AMBIENT_FD";
    const GRANT: &str = "CAPSEM_TEST_GRANTED_FD";
    if let (Ok(ambient), Ok(grant)) = (std::env::var(AMBIENT), std::env::var(GRANT)) {
        let ambient: i32 = ambient.parse().unwrap();
        let grant: i32 = grant.parse().unwrap();
        // SAFETY: this isolated subprocess runs exactly this one test and exits
        // immediately after taking ownership of its declared report grant.
        unsafe { super::close_inherited_descriptors_except(&[grant]).unwrap() };
        let ambient_closed = fcntl(ambient, FcntlArg::F_GETFD) == Err(Errno::EBADF);
        let grant_open = fcntl(grant, FcntlArg::F_GETFD).is_ok();
        // SAFETY: the descriptor was inherited solely for this child report.
        let mut report = unsafe { UnixStream::from_raw_fd(grant) };
        report.write_all(&[u8::from(ambient_closed && grant_open)]).unwrap();
        std::process::exit(i32::from(!(ambient_closed && grant_open)));
    }

    let (_ambient_parent, ambient_child) = UnixStream::pair().unwrap();
    let (mut report_parent, report_child) = UnixStream::pair().unwrap();
    for descriptor in [&ambient_child, &report_child] {
        fcntl(descriptor.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::empty())).unwrap();
    }
    let executable = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(executable)
        .args([
            "--exact",
            "unix::fd::tests::isolated_exec_child_closes_ambient_descriptor_and_keeps_declared_grant",
            "--test-threads=1",
        ])
        .env(AMBIENT, ambient_child.as_raw_fd().to_string())
        .env(GRANT, report_child.as_raw_fd().to_string())
        .spawn()
        .unwrap();
    drop(ambient_child);
    drop(report_child);

    let mut result = [0];
    report_parent.read_exact(&mut result).unwrap();
    assert_eq!(result, [1]);
    assert!(child.wait().unwrap().success());
}

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
fn resetting_a_connection_the_peer_already_reset_is_not_an_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    // Before the reset: XNU refuses setsockopt with EINVAL on a socket whose
    // connection is already gone, which failed this test whenever the reset
    // won the race.
    server
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    // Unread data at the client when it closes makes the kernel answer with
    // a reset instead of a FIN: the connection is gone before we look.
    server.write_all(b"unread").unwrap();
    drop(client);
    let observed = server.read(&mut [0]);
    assert!(
        matches!(
            observed.as_ref().map_err(std::io::Error::kind),
            Ok(0) | Err(std::io::ErrorKind::ConnectionReset)
        ),
        "{observed:?}"
    );
    assert!(super::reset_tcp(server.as_fd()).unwrap(), "already reset is reset");
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
fn readable_wait_distinguishes_idle_data_and_eof() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    assert!(!wait_readable(reader.as_fd(), std::time::Duration::ZERO).unwrap());

    writer.write_all(b"x").unwrap();
    assert!(wait_readable(reader.as_fd(), std::time::Duration::ZERO).unwrap());
    let mut byte = [0; 1];
    reader.read_exact(&mut byte).unwrap();

    drop(writer);
    assert!(wait_readable(reader.as_fd(), std::time::Duration::ZERO).unwrap());
    assert_eq!(reader.read(&mut byte).unwrap(), 0);
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

#[test]
fn stream_queues_count_what_each_end_of_a_unix_stream_holds() {
    let (mut writer, reader) = UnixStream::pair().unwrap();
    assert_eq!(super::stream_queues(reader.as_fd()).unread, Some(0));
    writer.write_all(&[7; 100]).unwrap();
    let held = super::stream_queues(reader.as_fd());
    assert_eq!(held.unread, Some(100));
    #[cfg(target_os = "linux")]
    assert!(
        super::stream_queues(writer.as_fd()).unsent.unwrap() >= 100,
        "a Unix stream's unread bytes are charged to its writer"
    );
    assert_eq!(
        (held.untransmitted, held.probes, held.backoff),
        (None, None, None),
        "window state belongs to TCP alone"
    );
}

/// The case a stalled published port has to be told apart by: a TCP peer
/// that stopped reading shuts its window, so what the writer queued is never
/// even transmitted.
#[cfg(target_os = "linux")]
#[test]
fn stream_queues_show_bytes_held_back_by_a_shut_tcp_window() {
    use nix::sys::socket::{setsockopt, sockopt};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    setsockopt(&listener, sockopt::RcvBuf, &(16 * 1024)).unwrap();
    let mut writer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (reader, _) = listener.accept().unwrap();
    writer.set_nonblocking(true).unwrap();
    let chunk = vec![7; 64 * 1024];
    let mut accepted = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match writer.write(&chunk) {
            Ok(count) => accepted += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if super::stream_queues(writer.as_fd()).untransmitted.unwrap() > 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    }
    let writer_queues = super::stream_queues(writer.as_fd());
    let untransmitted = writer_queues.untransmitted.unwrap();
    assert!(untransmitted > 0, "{writer_queues:?}");
    assert!(writer_queues.unsent.unwrap() >= untransmitted, "{writer_queues:?}");
    assert!(writer_queues.probes.is_some() && writer_queues.backoff.is_some());
    let reader_queues = super::stream_queues(reader.as_fd());
    assert!(reader_queues.unread.unwrap() > 0, "{reader_queues:?}");
    // Delivered-but-unacknowledged bytes count on both sides, so each queue,
    // not their sum, is bounded by what the writer handed the kernel.
    assert!(reader_queues.unread.unwrap() as usize <= accepted, "{reader_queues:?}");
    assert!(writer_queues.unsent.unwrap() as usize <= accepted, "{writer_queues:?}");
}

/// The watch reads queues off whatever descriptor it was handed, at any
/// moment of the copy's life: a listener or a stream whose peer is gone must
/// answer with counts or nothing, never an error or a panic.
#[test]
fn stream_queues_of_a_listener_or_an_orphaned_stream_are_harmless() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let queues = super::stream_queues(listener.as_fd());
    assert_eq!((queues.unsent, queues.untransmitted), (None, None), "{queues:?}");
    let (stream, peer) = UnixStream::pair().unwrap();
    drop(peer);
    assert_eq!(super::stream_queues(stream.as_fd()).unread, Some(0));
}
