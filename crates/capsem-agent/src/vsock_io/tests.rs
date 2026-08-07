use super::*;
use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::thread;

#[test]
fn vsock_connect_fails_gracefully_on_host() {
    let result = vsock_connect(VSOCK_HOST_CID, 9999);
    assert!(
        result.is_err(),
        "vsock connect should fail on macOS/host machines gracefully"
    );
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

    unsafe {
        nix::libc::close(client_fd);
    }
    let mut small_buf = [0u8; 1];
    let eof_res = read_exact_fd(server_fd, &mut small_buf);
    assert!(eof_res.is_err());
    assert_eq!(
        eof_res.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );

    unsafe {
        nix::libc::close(server_fd);
    }
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
    unsafe {
        nix::libc::close(fd);
    }
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
        unsafe {
            nix::libc::close(server_fd);
        }
        buf
    });

    write_all_fd(client_fd, &data).expect("large write");
    unsafe {
        nix::libc::close(client_fd);
    }

    let result = reader.join().unwrap();
    assert_eq!(result, data);
}

#[test]
fn write_all_fd_timeout_on_stalled_peer() {
    let (client, _server) = UnixStream::pair().unwrap();
    let fd = client.into_raw_fd();

    // Set a 200ms send timeout so the test doesn't wait 30s.
    let tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 200_000,
    };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }

    // Write 1MB with no reader -- must timeout, not hang.
    let result = write_all_fd(fd, &vec![0u8; 1024 * 1024]);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);

    unsafe {
        nix::libc::close(fd);
    }
}

#[test]
fn read_exact_fd_zero_length_buf() {
    let (client, _server) = UnixStream::pair().unwrap();
    let fd = client.into_raw_fd();
    let mut buf = [];
    read_exact_fd(fd, &mut buf).expect("zero-length read should succeed");
    unsafe {
        nix::libc::close(fd);
    }
}

#[test]
fn write_all_fd_to_closed_peer() {
    let (client, server) = UnixStream::pair().unwrap();
    let client_fd = client.into_raw_fd();
    drop(server); // close read end
    let result = write_all_fd(client_fd, b"should fail");
    assert!(result.is_err());
    unsafe {
        nix::libc::close(client_fd);
    }
}

#[test]
fn backoff_uses_proto_defaults() {
    // vsock_connect_retry uses capsem_proto::poll::RetryOpts defaults.
    // Verify they match expectations: 50ms initial, 500ms max.
    let opts = capsem_proto::poll::RetryOpts::default();
    assert_eq!(opts.initial_delay, Duration::from_millis(50));
    assert_eq!(opts.max_delay, Duration::from_millis(500));
}

#[test]
fn parse_vsock_port_offset_from_kernel_cmdline() {
    let cmdline = "console=ttyS0 root=/dev/vda capsem.vsock_port_offset=15480 quiet";
    assert_eq!(parse_vsock_port_offset(cmdline), Some(15480));
}

#[test]
fn parse_vsock_port_offset_ignores_missing_or_invalid_value() {
    assert_eq!(parse_vsock_port_offset("console=ttyS0 root=/dev/vda"), None);
    assert_eq!(
        parse_vsock_port_offset("capsem.vsock_port_offset=not-a-number"),
        None
    );
}

#[test]
fn physical_vsock_port_adds_kvm_offset() {
    assert_eq!(physical_vsock_port(5001, 15480).unwrap(), 20481);
}

#[test]
fn physical_vsock_port_rejects_overflow() {
    let err = physical_vsock_port(u32::MAX, 1).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn constants_match_spec() {
    assert_eq!(VSOCK_HOST_CID, 2);
    assert_eq!(AF_VSOCK, 40);
}
