//! Owned TCP listener creation at the host Unix boundary.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::os::fd::AsFd;

/// Bind a literal loopback IP on a kernel-selected port without resolving names.
/// The returned listener is nonblocking and can be adopted by Tokio. Dropping
/// it, including after failed reactor registration, closes the owned socket.
pub fn bind_loopback(ip: IpAddr) -> io::Result<TcpListener> {
    if !ip.is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "listener must bind a literal loopback IP",
        ));
    }
    let listener = TcpListener::bind(SocketAddr::new(ip, 0))?;
    super::fd::set_nonblocking(listener.as_fd(), true)?;
    Ok(listener)
}

#[cfg(test)]
mod tests;
