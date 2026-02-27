// capsem-net-proxy: Guest-side TCP-to-vsock relay for air-gapped networking.
//
// Listens on TCP 127.0.0.1:10443 (HTTPS) and bridges each connection to
// the host SNI proxy via vsock port 5002. The host proxy inspects the TLS
// ClientHello for the SNI hostname, checks the domain policy, and bridges
// to the real server if allowed.
//
// This binary runs inside the guest VM, launched by capsem-init.
// iptables REDIRECT captures port 443 traffic and sends it here.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io;
use std::os::unix::io::RawFd;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use vsock_io::{VSOCK_HOST_CID, vsock_connect, write_all_fd};

/// vsock port for SNI proxy on the host.
const VSOCK_PORT_SNI_PROXY: u32 = 5002;

/// TCP port to listen on for HTTPS traffic (iptables REDIRECT target).
const LISTEN_PORT_HTTPS: u16 = 10443;

/// Bind a TCP listener on 127.0.0.1:port.
fn tcp_listen(port: u16) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // SO_REUSEADDR for fast restart.
    let optval: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &optval as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as libc::sa_family_t;
    addr.sin_port = port.to_be();
    addr.sin_addr = libc::in_addr {
        s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
    };

    let ret = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(err);
    }

    let ret = unsafe { libc::listen(fd, 128) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(err);
    }

    Ok(fd)
}

/// Accept a connection on the given listener fd.
fn tcp_accept(listen_fd: RawFd) -> io::Result<RawFd> {
    let fd = unsafe {
        libc::accept(listen_fd, std::ptr::null_mut(), std::ptr::null_mut())
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Bridge two fds bidirectionally using poll(2).
/// Returns when either side closes or errors.
fn bridge(fd_a: RawFd, fd_b: RawFd) {
    let fd_a_clone = fd_a;
    let fd_b_clone = fd_b;

    // Spawn thread for fd_b -> fd_a
    let _thread = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let mut poll_fds = [
                PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd_b_clone) }, PollFlags::POLLIN),
            ];

            match poll(&mut poll_fds, PollTimeout::from(5000u16)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            }

            if let Some(revents) = poll_fds[0].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match nix::unistd::read(fd_b_clone, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if write_all_fd(fd_a_clone, &buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(nix::errno::Errno::EAGAIN) => {}
                        Err(_) => break,
                    }
                }
                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                    break;
                }
            }
        }
    });

    let mut buf = [0u8; 8192];

    loop {
        let mut poll_fds = [
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd_a) }, PollFlags::POLLIN),
        ];

        match poll(&mut poll_fds, PollTimeout::from(5000u16)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => break,
        }

        // fd_a -> fd_b
        if let Some(revents) = poll_fds[0].revents() {
            if revents.contains(PollFlags::POLLIN) {
                match nix::unistd::read(fd_a, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if write_all_fd(fd_b, &buf[..n]).is_err() {
                            break;
                        }
                    }
                    Err(nix::errno::Errno::EAGAIN) => {}
                    Err(_) => break,
                }
            }
            if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                break;
            }
        }
    }

    // Wait for the other direction to finish
    // However, if one direction closes (e.g. fd_a stops sending), 
    // the other direction might still be sending. If fd_a closes completely, 
    // the read above breaks, and we exit `bridge`. We should make sure we don't 
    // leave the thread running indefinitely, but dropping the fds in `handle_connection` 
    // will send an error/HUP to the thread and terminate it.
}

/// Handle a single TCP connection: connect to host vsock and bridge.
fn handle_connection(tcp_fd: RawFd) {
    // Connect to host SNI proxy via vsock.
    let vsock_fd = match vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_SNI_PROXY) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-net-proxy] vsock connect failed: {e}");
            unsafe { libc::close(tcp_fd); }
            return;
        }
    };

    // Bridge TCP <-> vsock.
    bridge(tcp_fd, vsock_fd);

    // Clean up.
    unsafe {
        libc::close(tcp_fd);
        libc::close(vsock_fd);
    }
}

static RUNNING: AtomicBool = AtomicBool::new(true);

fn main() {
    eprintln!("[capsem-net-proxy] starting (pid {})", process::id());

    // Install SIGTERM handler for clean shutdown.
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
    }

    let listen_fd = match tcp_listen(LISTEN_PORT_HTTPS) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-net-proxy] failed to bind port {LISTEN_PORT_HTTPS}: {e}");
            process::exit(1);
        }
    };
    eprintln!("[capsem-net-proxy] listening on 127.0.0.1:{LISTEN_PORT_HTTPS}");

    while RUNNING.load(Ordering::Relaxed) {
        match tcp_accept(listen_fd) {
            Ok(tcp_fd) => {
                thread::spawn(move || {
                    handle_connection(tcp_fd);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("[capsem-net-proxy] accept error: {e}");
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    eprintln!("[capsem-net-proxy] shutting down");
    unsafe { libc::close(listen_fd); }
}

extern "C" fn handle_signal(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::os::unix::io::IntoRawFd;
    use std::thread;

    #[test]
    fn vsock_port_matches_host() {
        assert_eq!(VSOCK_PORT_SNI_PROXY, 5002);
    }

    #[test]
    fn listen_port_is_10443() {
        assert_eq!(LISTEN_PORT_HTTPS, 10443);
    }

    #[test]
    fn tcp_listen_and_accept_works() {
        // Try to listen on an ephemeral port.
        let listen_fd = tcp_listen(0).expect("tcp_listen failed");
        assert!(listen_fd >= 0);

        // Get the port it bound to using getsockname
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        let ret = unsafe {
            libc::getsockname(
                listen_fd,
                &mut addr as *mut _ as *mut libc::sockaddr,
                &mut len,
            )
        };
        assert_eq!(ret, 0);
        let port = u16::from_be(addr.sin_port);

        // Connect a client in another thread
        let handle = thread::spawn(move || {
            let client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client.into_raw_fd()
        });

        // Accept the connection
        let client_fd = tcp_accept(listen_fd).expect("tcp_accept failed");
        assert!(client_fd >= 0);

        let peer_fd = handle.join().unwrap();

        // Cleanup
        unsafe {
            libc::close(client_fd);
            libc::close(peer_fd);
            libc::close(listen_fd);
        }
    }

    #[test]
    fn test_bridge_bidirectional() {
        // We can simulate the two sides of the bridge with UnixStream pairs.
        let (client_a, server_a) = UnixStream::pair().unwrap();
        let (client_b, server_b) = UnixStream::pair().unwrap();

        let fd_a = server_a.into_raw_fd();
        let fd_b = server_b.into_raw_fd();

        // Start the bridge in a background thread
        let bridge_handle = thread::spawn(move || {
            bridge(fd_a, fd_b);
            unsafe {
                libc::close(fd_a);
                libc::close(fd_b);
            }
        });

        let mut client_a = client_a;
        let mut client_b = client_b;

        // Test A -> B
        client_a.write_all(b"hello from A").unwrap();
        let mut buf = [0u8; 12];
        client_b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello from A");

        // Test B -> A
        client_b.write_all(b"hello from B").unwrap();
        let mut buf = [0u8; 12];
        client_a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello from B");

        // Close one side, the bridge should terminate
        drop(client_a);
        
        // The bridge should join successfully
        bridge_handle.join().unwrap();
    }

    #[test]
    fn test_bridge_concurrency_no_deadlock() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::AsRawFd;

        let (mut host_a, guest_a) = UnixStream::pair().unwrap();
        let (mut host_b, guest_b) = UnixStream::pair().unwrap();

        let fd_a = guest_a.as_raw_fd();
        let fd_b = guest_b.as_raw_fd();

        let _bridge_thread = std::thread::spawn(move || {
            bridge(fd_a, fd_b);
        });

        let data_size = 1024 * 1024;
        let test_data = vec![0x42u8; data_size];

        let mut host_a_read = host_a.try_clone().unwrap();
        let mut host_b_read = host_b.try_clone().unwrap();

        let t_a_write = std::thread::spawn({
            let test_data = test_data.clone();
            move || {
                std::io::Write::write_all(&mut host_a, &test_data).unwrap();
            }
        });

        let t_b_write = std::thread::spawn({
            let test_data = test_data.clone();
            move || {
                std::io::Write::write_all(&mut host_b, &test_data).unwrap();
            }
        });

        let t_a_read = std::thread::spawn(move || {
            let mut buf = vec![0u8; data_size];
            std::io::Read::read_exact(&mut host_a_read, &mut buf).unwrap();
            buf
        });

        let t_b_read = std::thread::spawn(move || {
            let mut buf = vec![0u8; data_size];
            std::io::Read::read_exact(&mut host_b_read, &mut buf).unwrap();
            buf
        });

        t_a_write.join().unwrap();
        t_b_write.join().unwrap();

        let out_a = t_a_read.join().unwrap();
        let out_b = t_b_read.join().unwrap();

        assert_eq!(out_a, test_data);
        assert_eq!(out_b, test_data);
    }
}
