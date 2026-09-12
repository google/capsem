//! ICMP echo without a `ping` binary: the guest image carries none, and the
//! private link's ICMP is a claim this harness has to prove on its own.
//! One raw socket, numbered requests with this process as the identifier,
//! replies matched behind whatever IP header the kernel leaves on them.
use anyhow::{ensure, Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

const ECHO_REQUEST: u8 = 8;
const ECHO_REPLY: u8 = 0;
const ICMP_HEADER_BYTES: usize = 8;

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct Args {
    /// The address to echo against, or a name the guest's resolver answers.
    #[arg(long)]
    pub address: String,
    #[arg(long, default_value_t = 10)]
    pub count: u16,
    /// Payload bytes after the ICMP header.
    #[arg(long, default_value_t = 56)]
    pub size: usize,
    #[arg(long, default_value_t = 100)]
    pub interval_ms: u64,
    /// How long one reply may take.
    #[arg(long, default_value_t = 1000)]
    pub timeout_ms: u64,
}

/// RFC 1071: the one's complement of the one's complement sum of the
/// sixteen-bit words, an odd trailing byte padded with zero.
pub(crate) fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = bytes
        .chunks(2)
        .map(|word| u32::from(u16::from_be_bytes([word[0], *word.get(1).unwrap_or(&0)])))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub(crate) fn echo_request(identifier: u16, sequence: u16, size: usize) -> Vec<u8> {
    let mut packet = vec![0u8; ICMP_HEADER_BYTES + size];
    packet[0] = ECHO_REQUEST;
    packet[4..6].copy_from_slice(&identifier.to_be_bytes());
    packet[6..8].copy_from_slice(&sequence.to_be_bytes());
    for (index, byte) in packet[ICMP_HEADER_BYTES..].iter_mut().enumerate() {
        *byte = index as u8;
    }
    let sum = checksum(&packet);
    packet[2..4].copy_from_slice(&sum.to_be_bytes());
    packet
}

/// The sequence of an echo reply for `identifier` in `bytes`, which may
/// start with the IPv4 header a raw socket delivers, or with the ICMP one.
pub(crate) fn echo_reply(bytes: &[u8], identifier: u16) -> Option<u16> {
    let icmp = if bytes.first().is_some_and(|byte| byte >> 4 == 4) {
        let header = usize::from(bytes[0] & 0x0f) * 4;
        bytes.get(header..)?
    } else {
        bytes
    };
    if icmp.len() < ICMP_HEADER_BYTES || icmp[0] != ECHO_REPLY || checksum(icmp) != 0 {
        return None;
    }
    (u16::from_be_bytes([icmp[4], icmp[5]]) == identifier).then(|| u16::from_be_bytes([icmp[6], icmp[7]]))
}

pub(crate) async fn run(args: Args) -> Result<serde_json::Value> {
    ensure!((1..=10_000).contains(&args.count), "count must be 1..10000");
    ensure!(args.size <= 65_000, "size must fit one packet");
    let target = match args.address.parse::<Ipv4Addr>() {
        Ok(address) => address,
        Err(_) => match crate::udp::resolve(&format!("{}:0", args.address)).await?.ip() {
            std::net::IpAddr::V4(address) => address,
            std::net::IpAddr::V6(_) => anyhow::bail!("{} names no IPv4 address", args.address),
        },
    };
    tokio::task::spawn_blocking(move || measure(&args, target))
        .await
        .context("ping task failed")?
}

fn measure(args: &Args, target: Ipv4Addr) -> Result<serde_json::Value> {
    let socket = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4)).context("open a raw ICMP socket")?;
    socket.set_read_timeout(Some(Duration::from_millis(args.timeout_ms)))?;
    let target = SocketAddrV4::new(target, 0);
    let identifier = (std::process::id() & 0xffff) as u16;
    let mut round_trips_ms = Vec::with_capacity(usize::from(args.count));
    let mut buffer = vec![std::mem::MaybeUninit::<u8>::uninit(); 65_536];
    let started = Instant::now();
    for sequence in 0..args.count {
        let request = echo_request(identifier, sequence, args.size);
        let sent = Instant::now();
        socket
            .send_to(&request, &target.into())
            .with_context(|| format!("send echo request to {}", target.ip()))?;
        let deadline = sent + Duration::from_millis(args.timeout_ms);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            socket.set_read_timeout(Some(remaining))?;
            let Ok((length, _)) = socket.recv_from(&mut buffer) else {
                break;
            };
            // SAFETY: recv_from initialised the first `length` bytes.
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), length) };
            if echo_reply(bytes, identifier) == Some(sequence) {
                round_trips_ms.push(sent.elapsed().as_secs_f64() * 1000.0);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(args.interval_ms));
    }
    let received = round_trips_ms.len() as u16;
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
        "ping": {"address": args.address.clone(), "target": target.ip().to_string(), "count": args.count, "size": args.size,
            "version": env!("CARGO_PKG_VERSION")},
        "metrics": metrics
    }))
}

#[cfg(test)]
mod tests;
