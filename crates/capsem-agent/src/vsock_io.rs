// Shared vsock I/O helpers for guest-side binaries.
//
// Provides low-level vsock connect and fd read/write primitives.
// Used by both capsem-pty-agent and capsem-net-proxy.
//
// Included via #[path] in each binary, so not all functions are used in each.
#![allow(dead_code)]

use std::io;
use std::os::unix::io::RawFd;
use std::thread;
use std::time::Duration;

use nix::libc;

/// Host CID (always 2 for the hypervisor).
pub const VSOCK_HOST_CID: u32 = 2;
/// AF_VSOCK address family.
pub const AF_VSOCK: i32 = 40;

#[repr(C)]
pub struct SockaddrVm {
    pub svm_family: libc::sa_family_t,
    pub svm_reserved1: u16,
    pub svm_port: u32,
    pub svm_cid: u32,
    pub svm_flags: u8,
    pub svm_zero: [u8; 3],
}

/// I/O timeout for vsock read/write operations. If a single syscall blocks
/// longer than this, it returns EAGAIN instead of hanging forever.
/// 30s is generous -- vsock to hypervisor should drain in milliseconds.
const IO_TIMEOUT_SECS: i64 = 30;

/// Connect to a vsock port on the given CID.
///
/// Sets SO_SNDTIMEO and SO_RCVTIMEO so that blocking read/write calls
/// return EAGAIN after IO_TIMEOUT_SECS instead of hanging indefinitely
/// if the host stops draining the buffer.
pub fn vsock_connect(cid: u32, port: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let addr = SockaddrVm {
        svm_family: AF_VSOCK as libc::sa_family_t,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: cid,
        svm_flags: 0,
        svm_zero: [0; 3],
    };

    let ret = unsafe {
        libc::connect(
            fd,
            &addr as *const SockaddrVm as *const libc::sockaddr,
            std::mem::size_of::<SockaddrVm>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(err);
    }

    // Set I/O timeouts so blocking read/write return EAGAIN on stall
    // rather than hanging forever inside the kernel.
    set_io_timeouts(fd);

    Ok(fd)
}

/// Apply send and receive timeouts to a socket fd.
fn set_io_timeouts(fd: RawFd) {
    let tv = libc::timeval {
        tv_sec: IO_TIMEOUT_SECS,
        tv_usec: 0,
    };
    unsafe {
        libc::setsockopt(
            fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
        libc::setsockopt(
            fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
}

/// Connect to a vsock port with exponential backoff retry.
pub fn vsock_connect_retry(cid: u32, port: u32, label: &str) -> RawFd {
    let mut delay_ms = 100;
    loop {
        match vsock_connect(cid, port) {
            Ok(fd) => {
                eprintln!("[capsem-agent] {label} connected (port {port})");
                return fd;
            }
            Err(e) => {
                eprintln!("[capsem-agent] {label} connect failed: {e}, retrying in {delay_ms}ms");
                thread::sleep(Duration::from_millis(delay_ms));
                delay_ms = (delay_ms * 2).min(2000);
            }
        }
    }
}

/// Write all bytes to an fd, retrying on partial writes.
///
/// Defense in depth against hangs:
/// - `Ok(0)`: treated as WriteZero (no progress) to prevent infinite loop
/// - `EAGAIN`: treated as fatal timeout (SO_SNDTIMEO fired), not retryable,
///   to prevent turning a kernel hang into a userspace spin-loop
/// - `EINTR`: retried (signal interrupted the syscall, normal)
/// - All other errors: propagated immediately
pub fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < data.len() {
        match nix::unistd::write(
            unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd) },
            &data[written..],
        ) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "write returned 0 bytes (no progress)",
                ));
            }
            Ok(n) => written += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => {
                // SO_SNDTIMEO fired -- host is not draining the buffer.
                // Treat as fatal, not retryable, to prevent userspace spin.
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "write timed out (host not reading)",
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes from an fd, retrying on partial reads.
///
/// Defense in depth against hangs:
/// - `Ok(0)`: EOF before buffer filled, returns UnexpectedEof
/// - `EAGAIN`: treated as fatal timeout (SO_RCVTIMEO fired), not retryable
/// - `EINTR`: retried (signal interrupted the syscall, normal)
/// - All other errors: propagated immediately
pub fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        match nix::unistd::read(fd, &mut buf[pos..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF",
                ))
            }
            Ok(n) => pos += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => {
                // SO_RCVTIMEO fired -- host is not sending data.
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "read timed out (host not writing)",
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn vsock_connect_fails_gracefully_on_host() {
        let result = vsock_connect(VSOCK_HOST_CID, 9999);
        assert!(result.is_err(), "vsock connect should fail on macOS/host machines gracefully");
    }

    #[test]
    fn read_write_exact_fd() {
        let (client, server) = UnixStream::pair().unwrap();
        let client_fd = client.into_raw_fd();
        let server_fd = server.into_raw_fd();

        let data = b"hello vsock_io world";
        write_all_fd(client_fd, data).expect("write_all_fd");

        let mut buf = vec![0u8; data.len()];
        read_exact_fd(server_fd, &mut buf).expect("read_exact_fd");
        assert_eq!(&buf, data);

        unsafe { nix::libc::close(client_fd); }
        let mut small_buf = [0u8; 1];
        let eof_res = read_exact_fd(server_fd, &mut small_buf);
        assert!(eof_res.is_err());
        assert_eq!(eof_res.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);

        unsafe { nix::libc::close(server_fd); }
    }

    #[test]
    fn sockaddr_vm_abi_guard() {
        // SockaddrVm must match the kernel's sockaddr_vm layout exactly.
        assert_eq!(std::mem::size_of::<SockaddrVm>(), 16);
        assert_eq!(std::mem::align_of::<SockaddrVm>(), 4);

        // Verify field offsets via a zeroed instance.
        let addr = SockaddrVm {
            svm_family: 0,
            svm_reserved1: 0,
            svm_port: 0,
            svm_cid: 0,
            svm_flags: 0,
            svm_zero: [0; 3],
        };
        let base = &addr as *const _ as usize;
        assert_eq!(&addr.svm_family as *const _ as usize - base, 0);
        assert_eq!(&addr.svm_port as *const _ as usize - base, 4);
        assert_eq!(&addr.svm_cid as *const _ as usize - base, 8);
        assert_eq!(&addr.svm_flags as *const _ as usize - base, 12);
    }

    #[test]
    fn write_all_fd_empty_data() {
        let (client, _server) = UnixStream::pair().unwrap();
        let fd = client.into_raw_fd();
        write_all_fd(fd, b"").expect("empty write should succeed");
        unsafe { nix::libc::close(fd); }
    }

    #[test]
    fn write_all_fd_large_data() {
        let (client, server) = UnixStream::pair().unwrap();
        let client_fd = client.into_raw_fd();
        let server_fd = server.into_raw_fd();

        // 256KB exceeds the kernel socket buffer (~128KB on macOS).
        // A reader thread must drain concurrently or write blocks.
        let data = vec![0xABu8; 256 * 1024];
        let expected_len = data.len();

        let reader = thread::spawn(move || {
            let mut buf = vec![0u8; expected_len];
            read_exact_fd(server_fd, &mut buf).unwrap();
            unsafe { nix::libc::close(server_fd); }
            buf
        });

        write_all_fd(client_fd, &data).expect("large write");
        unsafe { nix::libc::close(client_fd); }

        let result = reader.join().unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn write_all_fd_timeout_on_stalled_peer() {
        let (client, _server) = UnixStream::pair().unwrap();
        let fd = client.into_raw_fd();

        // Set a 200ms send timeout so the test doesn't wait 30s.
        let tv = libc::timeval { tv_sec: 0, tv_usec: 200_000 };
        unsafe {
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO,
                &tv as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }

        // Write 1MB with no reader -- must timeout, not hang.
        let result = write_all_fd(fd, &vec![0u8; 1024 * 1024]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);

        unsafe { nix::libc::close(fd); }
    }

    #[test]
    fn read_exact_fd_zero_length_buf() {
        let (client, _server) = UnixStream::pair().unwrap();
        let fd = client.into_raw_fd();
        let mut buf = [];
        read_exact_fd(fd, &mut buf).expect("zero-length read should succeed");
        unsafe { nix::libc::close(fd); }
    }

    #[test]
    fn write_all_fd_to_closed_peer() {
        let (client, server) = UnixStream::pair().unwrap();
        let client_fd = client.into_raw_fd();
        drop(server); // close read end
        let result = write_all_fd(client_fd, b"should fail");
        assert!(result.is_err());
        unsafe { nix::libc::close(client_fd); }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        // Verify the retry logic by checking the progression:
        // 100 -> 200 -> 400 -> 800 -> 1600 -> 2000 (capped)
        let mut delay_ms: u64 = 100;
        let expected = [200, 400, 800, 1600, 2000, 2000];
        for &exp in &expected {
            delay_ms = (delay_ms * 2).min(2000);
            assert_eq!(delay_ms, exp);
        }
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(VSOCK_HOST_CID, 2);
        assert_eq!(AF_VSOCK, 40);
    }
}
