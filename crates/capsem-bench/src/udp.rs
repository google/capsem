//! UDP echo, shared by the native host and musl guest binary: `--serve`
//! answers every datagram with itself, the default mode sends numbered
//! datagrams and counts what comes back and how fast. Datagrams are
//! independent, so loss is a number rather than a failure; the private link
//! between VMs is measured with the same code on both sides.
use anyhow::{ensure, Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// The largest datagram that fits one IPv4 packet.
const MAX_SIZE: usize = 65_507;
const SEQUENCE_BYTES: usize = 4;
/// Socket buffers sized for the largest datagram: a stock macOS socket
/// refuses anything past 9 KiB, and the guest kernel's default is not much
/// more generous for a burst.
const SOCKET_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// A UDP socket whose buffers hold the largest datagram, bound to `address`.
pub(crate) fn bound(address: SocketAddr) -> Result<UdpSocket> {
    let domain = if address.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)).context("open udp socket")?;
    socket.set_send_buffer_size(SOCKET_BUFFER_BYTES)?;
    socket.set_recv_buffer_size(SOCKET_BUFFER_BYTES)?;
    socket.set_nonblocking(true)?;
    socket
        .bind(&address.into())
        .with_context(|| format!("bind udp socket on {address}"))?;
    UdpSocket::from_std(socket.into()).context("adopt udp socket")
}

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct Args {
    /// Echo server to measure against (client mode).
    #[arg(long, required_unless_present = "serve", conflicts_with = "serve")]
    pub address: Option<SocketAddr>,
    /// Run the echo server on this address instead of measuring.
    #[arg(long)]
    pub serve: Option<SocketAddr>,
    /// Datagrams to send.
    #[arg(long, default_value_t = 100)]
    pub count: u32,
    /// Bytes per datagram, sequence included.
    #[arg(long, default_value_t = 64)]
    pub size: usize,
    /// Pause between datagrams.
    #[arg(long, default_value_t = 10)]
    pub interval_ms: u64,
    /// How long to wait for the last echoes after the last send.
    #[arg(long, default_value_t = 1000)]
    pub wait_ms: u64,
}

/// A datagram: its sequence, big-endian, then padding to `size`.
pub(crate) fn datagram(sequence: u32, size: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; size.max(SEQUENCE_BYTES)];
    bytes[..SEQUENCE_BYTES].copy_from_slice(&sequence.to_be_bytes());
    bytes
}

pub(crate) fn sequence_of(bytes: &[u8]) -> Option<u32> {
    bytes
        .get(..SEQUENCE_BYTES)
        .map(|head| u32::from_be_bytes(head.try_into().unwrap()))
}

pub(crate) async fn run(args: Args) -> Result<serde_json::Value> {
    ensure!((1..=100_000).contains(&args.count), "count must be 1..100000");
    ensure!(
        (SEQUENCE_BYTES..=MAX_SIZE).contains(&args.size),
        "size must be {SEQUENCE_BYTES}..{MAX_SIZE}"
    );
    if let Some(address) = args.serve {
        let socket = bound(address)?;
        eprintln!("udp echo server listening on {}", socket.local_addr()?);
        return serve(socket).await;
    }
    let address = args.address.context("--address is required in client mode")?;
    measure(&args, address).await
}

/// Echo until the socket fails.
pub(crate) async fn serve(socket: UdpSocket) -> Result<serde_json::Value> {
    let mut buffer = vec![0u8; MAX_SIZE];
    loop {
        let (length, peer) = socket.recv_from(&mut buffer).await.context("receive datagram")?;
        // A peer that vanished is its own loss, not the server's end.
        let _ = socket.send_to(&buffer[..length], peer).await;
    }
}

async fn measure(args: &Args, address: SocketAddr) -> Result<serde_json::Value> {
    let bind: SocketAddr = if address.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse()?;
    let socket = bound(bind)?;
    socket
        .connect(address)
        .await
        .with_context(|| format!("connect udp client to {address}"))?;
    let mut sent_at: HashMap<u32, Instant> = HashMap::with_capacity(args.count as usize);
    let mut round_trips_ms = Vec::with_capacity(args.count as usize);
    let mut buffer = vec![0u8; MAX_SIZE];
    let started = Instant::now();
    let interval = Duration::from_millis(args.interval_ms);
    let receive =
        |socket: &UdpSocket, buffer: &mut [u8], sent_at: &mut HashMap<u32, Instant>, round_trips_ms: &mut Vec<f64>| {
            while let Ok(length) = socket.try_recv(buffer) {
                if let Some(started) = sequence_of(&buffer[..length]).and_then(|sequence| sent_at.remove(&sequence)) {
                    round_trips_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
        };
    for sequence in 0..args.count {
        let bytes = datagram(sequence, args.size);
        sent_at.insert(sequence, Instant::now());
        // A refused or unreachable send is a lost datagram, like any other.
        let _ = socket.send(&bytes).await;
        receive(&socket, &mut buffer, &mut sent_at, &mut round_trips_ms);
        if !interval.is_zero() {
            tokio::time::sleep(interval).await;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(args.wait_ms);
    while !sent_at.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, socket.recv(&mut buffer)).await {
            Ok(Ok(length)) => {
                if let Some(started) = sequence_of(&buffer[..length]).and_then(|sequence| sent_at.remove(&sequence)) {
                    round_trips_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => break,
        }
    }
    let received = round_trips_ms.len() as u32;
    let mut metrics = serde_json::json!({
        "sent": {"unit": "count", "samples": [args.count]},
        "received": {"unit": "count", "samples": [received]},
        "lost": {"unit": "count", "samples": [args.count - received]},
        "elapsed_seconds": {"unit": "seconds", "samples": [started.elapsed().as_secs_f64()]},
    });
    if !round_trips_ms.is_empty() {
        metrics["round_trip_ms"] = serde_json::json!({"unit": "milliseconds", "samples": round_trips_ms});
    }
    Ok(serde_json::json!({
        "udp": {"address": address.to_string(), "count": args.count, "size": args.size,
            "interval_ms": args.interval_ms, "version": env!("CARGO_PKG_VERSION")},
        "metrics": metrics
    }))
}

#[cfg(test)]
mod tests;
