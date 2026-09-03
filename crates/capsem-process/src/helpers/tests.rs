use super::*;

#[test]
fn clone_fd_valid_file() {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;

    let (mut reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let connection = capsem_core::VsockConnection::new(writer.as_raw_fd(), 5000, Box::new(writer));
    let mut cloned = clone_fd(&connection, "test-duplicate").unwrap();
    cloned.write_all(b"test").unwrap();
    let mut received = [0; 4];
    reader.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"test");
}

#[test]
fn clone_fd_invalid_fd_fails() {
    // -1 is universally an invalid file descriptor in POSIX.
    // This avoids multithreaded race conditions where a closed FD
    // is instantly reused by another test.
    let connection = capsem_core::VsockConnection::new(-1, 5000, Box::new(()));
    assert!(clone_fd(&connection, "test-invalid-duplicate").is_none());
}
