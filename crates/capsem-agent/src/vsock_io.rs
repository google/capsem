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

/// Connect to a vsock port on the given CID.
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

    Ok(fd)
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
pub fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < data.len() {
        match nix::unistd::write(
            unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd) },
            &data[written..],
        ) {
            Ok(n) => written += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes from an fd, retrying on partial reads.
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
    fn test_vsock_connect_fails_gracefully_on_host() {
        // AF_VSOCK is likely not supported or no device exists on the test host
        let result = vsock_connect(VSOCK_HOST_CID, 9999);
        assert!(result.is_err(), "vsock connect should fail on macOS/host machines gracefully");
    }

    #[test]
    fn test_read_write_exact_fd() {
        let (client, server) = UnixStream::pair().unwrap();
        let client_fd = client.into_raw_fd();
        let server_fd = server.into_raw_fd();

        let data = b"hello vsock_io world";
        
        // Write all
        write_all_fd(client_fd, data).expect("failed to write_all_fd");
        
        // Read exact
        let mut buf = vec![0u8; data.len()];
        read_exact_fd(server_fd, &mut buf).expect("failed to read_exact_fd");
        assert_eq!(&buf, data);

        // EOF read exact should fail with UnexpectedEof
        unsafe { nix::libc::close(client_fd); }
        let mut small_buf = [0u8; 1];
        let eof_res = read_exact_fd(server_fd, &mut small_buf);
        assert!(eof_res.is_err());
        assert_eq!(eof_res.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);

        unsafe { nix::libc::close(server_fd); }
    }
}
