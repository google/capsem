use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

fn async_stream(stream: UnixStream) -> tokio::net::UnixStream {
    stream.set_nonblocking(true).unwrap();
    tokio::net::UnixStream::from_std(stream).unwrap()
}

#[tokio::test]
async fn acknowledged_socket_remains_usable_after_sender_drops_original() {
    let (parent, child) = UnixStream::pair().unwrap();
    let sender = Sender::new(parent).unwrap();
    let receiver = Receiver::new(child).unwrap();
    let mut peers = Vec::new();
    for _ in 0..32 {
        let (data, peer) = UnixStream::pair().unwrap();
        sender.send(&[7; FRAME_SIZE], &[data.as_raw_fd()]).await.unwrap();
        peers.push((data, async_stream(peer)));
    }
    for (original, mut peer) in peers {
        let mut frame = receiver.recv().await.unwrap();
        let mut adopted = async_stream(UnixStream::from(frame.fds.pop().unwrap()));
        // Adoption is the receiver's acknowledgement boundary. Darwin's UNIX
        // socket GC can flush a socket held only by queued SCM_RIGHTS messages.
        drop(original);
        peer.write_all(b"ping").await.unwrap();
        let mut buffer = [0; 4];
        adopted.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"ping");
        adopted.write_all(b"pong").await.unwrap();
        peer.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"pong");
    }
}

#[test]
fn malformed_handoffs_restore_descriptor_count_in_isolated_process() {
    const PROBE: &str = "CAPSEM_ROUTER_FD_COUNT_PROBE";
    if std::env::var_os(PROBE).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix::router_channel::tests::malformed_handoffs_restore_descriptor_count_in_isolated_process",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    #[cfg(target_os = "linux")]
    const DIRECTORY: &str = "/proc/self/fd";
    #[cfg(not(target_os = "linux"))]
    const DIRECTORY: &str = "/dev/fd";
    let count = || std::fs::read_dir(DIRECTORY).unwrap().map(Result::unwrap).count();
    // Tokio installs a process-wide signal pipe on its first IO runtime.
    // Initialize that infrastructure before measuring channel ownership.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::task::yield_now().await });
    let baseline = count();
    // Each test owns and drops its runtime; an isolated process avoids counts
    // changing underneath us when the ordinary test runner uses other threads.
    for check in [
        oversized_ancillary_record_closes_every_delivered_descriptor,
        receiver_closes_excess_descriptors_before_rejecting,
        cancellation_of_partial_record_closes_channel_and_received_fd,
        eof_after_partial_record_closes_received_fd,
        closed_peer_fails_send_without_consuming_source_descriptors,
        fixed_record_transfers_independently_owned_cloexec_fds,
    ] {
        check();
        assert_eq!(count(), baseline, "handoff leaked an owned descriptor");
    }
}

#[tokio::test]
async fn oversized_ancillary_record_closes_every_delivered_descriptor() {
    let (parent, child) = UnixStream::pair().unwrap();
    let receiver = Receiver::new(child).unwrap();
    let (data, peer) = UnixStream::pair().unwrap();
    let descriptors = [data.as_raw_fd(); 20];
    // SAFETY: initialized, aligned storage holds the deliberately oversized
    // SCM_RIGHTS record; every source descriptor remains alive through sendmsg.
    unsafe {
        let mut control = [0usize; 32];
        let mut bytes = [0u8; FRAME_SIZE];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = zeroed();
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = libc::CMSG_SPACE(size_of_val(&descriptors) as u32) as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(size_of_val(&descriptors) as u32) as _;
        std::ptr::copy_nonoverlapping(descriptors.as_ptr(), libc::CMSG_DATA(header).cast(), descriptors.len());
        assert_eq!(
            libc::sendmsg(parent.as_raw_fd(), &message, libc::MSG_NOSIGNAL),
            FRAME_SIZE as isize
        );
    }
    drop(data);
    assert_eq!(receiver.recv().await.err().unwrap().kind(), ErrorKind::InvalidData);
    assert_eq!(
        timeout(Duration::from_secs(1), async_stream(peer).read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn closed_peer_fails_send_without_consuming_source_descriptors() {
    let (parent, child) = UnixStream::pair().unwrap();
    let sender = Sender::new(parent).unwrap();
    drop(child);
    let (data, peer) = UnixStream::pair().unwrap();
    assert_eq!(
        timeout(
            Duration::from_secs(1),
            sender.send(&[0; FRAME_SIZE], &[data.as_raw_fd()])
        )
        .await
        .unwrap()
        .unwrap_err()
        .kind(),
        ErrorKind::BrokenPipe
    );
    assert!(data.peer_addr().is_ok());
    drop(data);
    assert_eq!(async_stream(peer).read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn receiver_closes_excess_descriptors_before_rejecting() {
    let (parent, child) = UnixStream::pair().unwrap();
    let receiver = Receiver::new(child).unwrap();
    let (data, peer) = UnixStream::pair().unwrap();
    send_record(parent.as_raw_fd(), &[0; 10], &[data.as_raw_fd(); 3]).unwrap();
    drop(data);
    assert!(receiver.recv().await.is_err());
    assert_eq!(
        timeout(Duration::from_secs(1), async_stream(peer).read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn cancellation_of_partial_record_closes_channel_and_received_fd() {
    let (parent, child) = UnixStream::pair().unwrap();
    let receiver = Receiver::new(child).unwrap();
    let (data, peer) = UnixStream::pair().unwrap();
    send_record(parent.as_raw_fd(), &[1], &[data.as_raw_fd()]).unwrap();
    drop(data);
    assert!(timeout(Duration::from_millis(20), receiver.recv()).await.is_err());
    assert_eq!(
        timeout(Duration::from_secs(1), async_stream(peer).read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    // Peer EOF proves shutdown. Darwin may accept a write after SHUT_RD until
    // the receiver's descriptor is closed, so write failure is not portable.
    parent.set_nonblocking(true).unwrap();
    let mut parent = parent;
    assert_eq!(std::io::Read::read(&mut parent, &mut [0]).unwrap(), 0);
}

#[tokio::test]
async fn eof_after_partial_record_closes_received_fd() {
    let (parent, child) = UnixStream::pair().unwrap();
    let receiver = Receiver::new(child).unwrap();
    let (data, peer) = UnixStream::pair().unwrap();
    send_record(parent.as_raw_fd(), &[1], &[data.as_raw_fd()]).unwrap();
    drop(data);
    drop(parent);
    assert!(matches!(receiver.recv().await, Err(error) if error.kind() == io::ErrorKind::UnexpectedEof));
    assert_eq!(
        timeout(Duration::from_secs(1), async_stream(peer).read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn fixed_record_transfers_independently_owned_cloexec_fds() {
    let (parent, child) = UnixStream::pair().unwrap();
    let sender = Sender::new(parent).unwrap();
    let receiver = Receiver::new(child).unwrap();
    let (data, peer) = UnixStream::pair().unwrap();
    sender
        .send(&[7; 10], &[data.as_raw_fd(), data.as_raw_fd()])
        .await
        .unwrap();
    let frame = receiver.recv().await.unwrap();
    assert_eq!(frame.bytes, [7; 10]);
    assert_eq!(frame.fds.len(), 2);
    assert_ne!(frame.fds[0].as_raw_fd(), frame.fds[1].as_raw_fd());
    for fd in &frame.fds {
        // SAFETY: descriptors are live and F_GETFD does not mutate them.
        assert_ne!(
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }
    drop(data);
    drop(frame);
    assert_eq!(async_stream(peer).read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn sender_rejects_variable_length_records() {
    let (parent, _child) = UnixStream::pair().unwrap();
    let sender = Sender::new(parent).unwrap();
    assert_eq!(
        sender.send(&[0; 11], &[]).await.unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn sender_rejects_more_than_two_descriptors() {
    let (parent, _child) = UnixStream::pair().unwrap();
    let sender = Sender::new(parent).unwrap();
    let (data, _peer) = UnixStream::pair().unwrap();
    assert_eq!(
        sender.send(&[0; 10], &[data.as_raw_fd(); 3]).await.unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}
