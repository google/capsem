//! The host end of a guest `tun0`.
//!
//! The guest's `capsem-tun` reads raw IP packets from its tun device and
//! writes them, length-prefixed, down one VSOCK connection; this crate turns
//! that byte stream into a smoltcp interface and terminates TCP on it. The
//! kernel never sees these packets: every connection a guest makes over
//! `tun0` is a smoltcp socket in a process the host controls, which is the
//! point of the design -- policy and audit sit on the socket, not on a
//! netfilter rule.
//!
//! [`Stack`] is the generic half: the framed device, the interface and the
//! poll loop. [`throughput::ThroughputServer`] is the first application on
//! it, the far end of `capsem-bench-rs throughput`, so the tun0 lane can be
//! measured against the FD-pair lane with the same client.

pub mod device;
pub mod frames;
pub mod relay;
pub mod stack;
pub mod throughput;

pub use stack::{App, Progress, Stack};

/// Serve `capsem-bench-rs throughput` clients on one guest's packet stream
/// until the stream ends. This is the whole host side of the tun0 lane.
pub async fn serve_throughput<IO>(io: IO) -> std::io::Result<bool>
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let address = smoltcp::wire::Ipv4Cidr::new(GATEWAY_ADDRESS, POOL_PREFIX_LEN);
    let mut server = throughput::ThroughputServer::new(throughput::THROUGHPUT_PORT);
    Stack::new(io, address, LINK_MTU).run(&mut server).await
}

/// The host's address on every guest's private link, until the registry of
/// S03-002 hands each VM its own peer address from the same pool.
pub const GATEWAY_ADDRESS: std::net::Ipv4Addr = std::net::Ipv4Addr::new(10, 128, 0, 1);
/// The private pool, `10.128.0.0/9`: disjoint from the guest's own
/// `10.0.0.0/24` and from the container link.
pub const POOL_PREFIX_LEN: u8 = 9;
/// Frames carry a `u16` length, so this is the largest IP packet the link
/// can move. A tun device accepts it as its MTU; a large MTU is what keeps
/// the per-packet cost of a user-space stack out of the bulk numbers.
pub const LINK_MTU: usize = u16::MAX as usize;
