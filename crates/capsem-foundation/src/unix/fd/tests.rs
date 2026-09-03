use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;

use nix::fcntl::{fcntl, FcntlArg, FdFlag, OFlag};

use super::{duplicate, retry_eintr, set_nonblocking, shutdown, SocketShutdown};
use nix::errno::Errno;

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
