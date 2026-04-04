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

#[path = "procfs.rs"]
mod procfs;

use std::io;
use std::os::unix::io::{BorrowedFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::process;
use std::task::{Context, Poll};

use nix::libc;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;

use vsock_io::{VSOCK_HOST_CID, vsock_connect};

/// vsock port for SNI proxy on the host.
const VSOCK_PORT_SNI_PROXY: u32 = 5002;

/// TCP port to listen on for HTTPS traffic (iptables REDIRECT target).
const LISTEN_PORT_HTTPS: u16 = 10443;

// Async wrapper for vsock RawFd
struct AsyncVsock {
    inner: AsyncFd<std::os::unix::net::UnixStream>,
    fd: RawFd,
}

impl AsyncVsock {
    fn new(fd: RawFd) -> io::Result<Self> {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        // We wrap it in a UnixStream to be able to use AsyncFd,
        // although it's actually an AF_VSOCK socket.
        let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        Ok(Self {
            inner: AsyncFd::new(std_stream)?,
            fd,
        })
    }
}

impl AsyncRead for AsyncVsock {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.inner.poll_read_ready(cx) {
                Poll::Ready(Ok(guard)) => guard,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            let unfilled = buf.initialize_unfilled();
            match nix::unistd::read(self.fd, unfilled) {
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    guard.clear_ready();
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }
    }
}

impl AsyncWrite for AsyncVsock {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = match self.inner.poll_write_ready(cx) {
                Poll::Ready(Ok(guard)) => guard,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            match nix::unistd::write(unsafe { BorrowedFd::borrow_raw(self.fd) }, buf) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(nix::errno::Errno::EAGAIN) => {
                    guard.clear_ready();
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let ret = unsafe { libc::shutdown(self.fd, libc::SHUT_WR) };
        if ret == 0 {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::last_os_error()))
        }
    }
}

// No custom Drop: inner AsyncFd<UnixStream> owns the fd via from_raw_fd
// and closes it automatically. Manual libc::close would double-close.

/// Retrieve the process name that initiated the TCP connection.
async fn get_process_name(client_port: u16) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let port_hex = format!("{:04X}", client_port);

        let mut inode = None;
        if let Ok(tcp_content) = std::fs::read_to_string("/proc/net/tcp") {
            for line in tcp_content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                // Index 1 is local_address (ip:port).
                // Index 9 is inode.
                if parts.len() >= 10 {
                    let local_addr = parts[1];
                    if local_addr.ends_with(&format!(":{}", port_hex)) {
                        inode = Some(parts[9].to_string());
                        break;
                    }
                }
            }
        }

        // In rare cases (e.g., IPv6), it might be in /proc/net/tcp6
        if inode.is_none() {
            if let Ok(tcp6_content) = std::fs::read_to_string("/proc/net/tcp6") {
                for line in tcp6_content.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 10 {
                        let local_addr = parts[1];
                        if local_addr.ends_with(&format!(":{}", port_hex)) {
                            inode = Some(parts[9].to_string());
                            break;
                        }
                    }
                }
            }
        }

        let inode = inode?;
        let target = format!("socket:[{}]", inode);

        // Search /proc/<pid>/fd/
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        let pid_str = entry.file_name();
                        let pid_str = pid_str.to_string_lossy();
                        if pid_str.chars().all(|c| c.is_ascii_digit()) {
                            let fd_dir = entry.path().join("fd");
                            if let Ok(fds) = std::fs::read_dir(&fd_dir) {
                                for fd_entry in fds.flatten() {
                                    if let Ok(link) = std::fs::read_link(fd_entry.path()) {
                                        if link.to_string_lossy() == target {
                                            let pid: u32 = pid_str.parse().unwrap_or(0);
                                            return Some(procfs::process_name_for_pid(pid));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    })
    .await
    .unwrap_or(None)
}

async fn handle_connection(mut tcp_stream: TcpStream) {
    let peer_addr = match tcp_stream.peer_addr() {
        Ok(addr) => addr,
        Err(_) => return,
    };

    let process_name = get_process_name(peer_addr.port())
        .await
        .unwrap_or_else(|| "unknown".to_string());

    let vsock_raw = match tokio::task::spawn_blocking(|| {
        vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_SNI_PROXY)
    })
    .await
    {
        Ok(Ok(fd)) => fd,
        _ => {
            eprintln!("[capsem-net-proxy] vsock connect failed");
            return;
        }
    };

    let mut vsock_stream = match AsyncVsock::new(vsock_raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[capsem-net-proxy] failed to create AsyncVsock: {e}");
            unsafe {
                libc::close(vsock_raw);
            }
            return;
        }
    };

    let meta = format!("\0CAPSEM_META:{}\n", process_name);
    if let Err(e) = vsock_stream.write_all(meta.as_bytes()).await {
        eprintln!("[capsem-net-proxy] failed to inject process meta: {e}");
        return;
    }

    if let Err(e) = tokio::io::copy_bidirectional(&mut tcp_stream, &mut vsock_stream).await {
        let is_normal = e.kind() == io::ErrorKind::ConnectionReset
            || e.kind() == io::ErrorKind::UnexpectedEof
            || e.kind() == io::ErrorKind::BrokenPipe;
        if !is_normal {
            eprintln!("[capsem-net-proxy] bridge error: {e}");
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    eprintln!("[capsem-net-proxy] starting (pid {})", process::id());

    let listener = TcpListener::bind(("127.0.0.1", LISTEN_PORT_HTTPS)).await?;
    eprintln!("[capsem-net-proxy] listening on 127.0.0.1:{LISTEN_PORT_HTTPS}");

    loop {
        tokio::select! {
            Ok((stream, _)) = listener.accept() => {
                let _ = stream.set_nodelay(true);
                tokio::spawn(async move {
                    handle_connection(stream).await;
                });
            }
            _ = signal::ctrl_c() => {
                eprintln!("[capsem-net-proxy] shutting down");
                break;
            }
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
    fn vsock_port_matches_host() {
        assert_eq!(VSOCK_PORT_SNI_PROXY, 5002);
    }

    #[test]
    fn listen_port_is_10443() {
        assert_eq!(LISTEN_PORT_HTTPS, 10443);
    }

    #[test]
    fn async_vsock_from_socketpair() {
        // Verify AsyncVsock wraps a raw fd from a unix socketpair
        let (a, _b) = UnixStream::pair().unwrap();
        let fd = a.into_raw_fd();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let vsock = AsyncVsock::new(fd);
            assert!(vsock.is_ok(), "AsyncVsock should wrap a socketpair fd");
            // Drop will close the fd
        });
    }

    #[tokio::test]
    async fn tcp_bind_accept_localhost() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();

        assert_eq!(peer.ip(), std::net::Ipv4Addr::LOCALHOST);
        assert!(client.peer_addr().is_ok());
        drop(server);
        drop(client);
    }

    #[tokio::test]
    async fn meta_line_injected_before_data() {
        // Simulate the meta line injection that handle_connection does
        let (a, b) = UnixStream::pair().unwrap();
        let fd = a.into_raw_fd();
        let mut vsock = AsyncVsock::new(fd).unwrap();

        let meta = "\0CAPSEM_META:test-agent\n".to_string();
        tokio::io::AsyncWriteExt::write_all(&mut vsock, meta.as_bytes())
            .await
            .unwrap();

        // Read from the other end
        let mut buf = vec![0u8; meta.len()];
        use std::io::Read;
        let mut reader = b;
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf[0], 0); // NUL prefix
        assert!(String::from_utf8_lossy(&buf).contains("CAPSEM_META:test-agent"));
    }

    #[tokio::test]
    async fn async_vsock_write_then_read() {
        let (a, b) = UnixStream::pair().unwrap();
        let fd_a = a.into_raw_fd();
        let fd_b = b.into_raw_fd();

        let mut va = AsyncVsock::new(fd_a).unwrap();
        let mut vb = AsyncVsock::new(fd_b).unwrap();

        // Write from a, read fixed-size from b
        tokio::io::AsyncWriteExt::write_all(&mut va, b"ping").await.unwrap();

        let mut buf = [0u8; 4];
        tokio::io::AsyncReadExt::read_exact(&mut vb, &mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[tokio::test]
    async fn async_vsock_large_transfer() {
        let (a, b) = UnixStream::pair().unwrap();
        let fd_a = a.into_raw_fd();
        let fd_b = b.into_raw_fd();

        let mut va = AsyncVsock::new(fd_a).unwrap();
        let mut vb = AsyncVsock::new(fd_b).unwrap();

        let data: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
        let data_clone = data.clone();
        
        let (write_res, read_res) = tokio::join!(
            async {
                let r = tokio::io::AsyncWriteExt::write_all(&mut va, &data_clone).await;
                tokio::io::AsyncWriteExt::shutdown(&mut va).await.unwrap();
                r
            },
            async {
                let mut received = Vec::new();
                let r = tokio::io::AsyncReadExt::read_to_end(&mut vb, &mut received).await;
                (r, received)
            }
        );

        write_res.unwrap();
        let (_, received) = read_res;
        assert_eq!(received.len(), 65536);
        assert_eq!(received, data);
    }

}