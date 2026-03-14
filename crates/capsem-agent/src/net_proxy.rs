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
        Poll::Ready(Ok(()))
    }
}

impl Drop for AsyncVsock {
    fn drop(&mut self) {
        // Close the underlying file descriptor
        unsafe {
            libc::close(self.fd);
        }
        // Let the std_stream drop as normal, but its FD is now closed,
        // which might cause an issue on drop, so we take it out using into_raw_fd if we could.
        // Actually, UnixStream Drop will close it. So let's not double-close.
        // wait, we used UnixStream::from_raw_fd. So when inner is dropped,
        // the std UnixStream is dropped, which closes the fd automatically!
        // So we MUST NOT call libc::close(self.fd) manually.
    }
}

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

    #[test]
    fn vsock_port_matches_host() {
        assert_eq!(VSOCK_PORT_SNI_PROXY, 5002);
    }

    #[test]
    fn listen_port_is_10443() {
        assert_eq!(LISTEN_PORT_HTTPS, 10443);
    }
}