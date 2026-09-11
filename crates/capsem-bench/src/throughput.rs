//! Bulk TCP throughput, shared by the native host and musl guest binary.
//!
//! One binary plays both ends so a lane is measured with the same code on
//! either side of Capsem: `--serve` runs the sink/source, the default mode
//! runs the client. The wire protocol is one byte, the direction, written
//! by the client; after it the stream is raw bytes in the agreed
//! direction(s) until the client's deadline closes it. The host smoltcp
//! endpoint in `capsem-network` speaks the same byte so the tun0 lane and
//! the FD-pair lane differ only in transport.
use anyhow::{bail, ensure, Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Which way payload flows, as seen from the client.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Client sends, server discards.
    Upload,
    /// Server sends, client discards.
    Download,
    /// Both at once, one stream each way per connection.
    Bidirectional,
    /// Small echoes, one in flight per stream: round-trip latency.
    Latency,
}

impl Direction {
    /// The first byte on every connection.
    pub(crate) const fn wire(self) -> u8 {
        match self {
            Self::Upload => 0,
            Self::Download => 1,
            Self::Bidirectional => 2,
            Self::Latency => 3,
        }
    }

    pub(crate) const fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Upload),
            1 => Some(Self::Download),
            2 => Some(Self::Bidirectional),
            3 => Some(Self::Latency),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Bidirectional => "bidirectional",
            Self::Latency => "latency",
        }
    }

    const fn sends(self) -> bool {
        matches!(self, Self::Upload | Self::Bidirectional)
    }

    const fn receives(self) -> bool {
        matches!(self, Self::Download | Self::Bidirectional)
    }
}

/// The server's grace after the client's deadline, and the client's own
/// bound on connect plus close: a peer that never drains is a failed run,
/// not a hung one.
const SETUP_AND_TEARDOWN_GRACE: Duration = Duration::from_secs(10);
const MAX_CHUNK_BYTES: usize = 1 << 20;
/// One echo: small enough to sit in a single segment on any MTU, so the
/// round trip measures the path and not segmentation.
const ECHO_BYTES: usize = 64;

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct Args {
    /// Server to measure against (client mode).
    #[arg(long, required_unless_present = "serve", conflicts_with = "serve")]
    pub address: Option<SocketAddr>,
    /// Run the sink/source server on this address instead of measuring.
    #[arg(long)]
    pub serve: Option<SocketAddr>,
    #[arg(long, value_enum, default_value_t = Direction::Upload)]
    pub direction: Direction,
    #[arg(long, default_value_t = 1)]
    pub streams: usize,
    #[arg(long, default_value_t = 5)]
    pub seconds: u64,
    #[arg(long, default_value_t = 64 * 1024)]
    pub chunk_bytes: usize,
}

pub(crate) async fn run(args: Args) -> Result<serde_json::Value> {
    ensure!((1..=64).contains(&args.streams), "streams must be 1..64");
    ensure!((1..=60).contains(&args.seconds), "seconds must be 1..60");
    ensure!(
        (1024..=MAX_CHUNK_BYTES).contains(&args.chunk_bytes),
        "chunk_bytes must be 1024..1048576"
    );
    if let Some(address) = args.serve {
        let listener = TcpListener::bind(address)
            .await
            .with_context(|| format!("bind throughput server on {address}"))?;
        eprintln!("throughput server listening on {}", listener.local_addr()?);
        return serve(listener, args.chunk_bytes).await;
    }
    let address = args.address.context("--address is required in client mode")?;
    let deadline = Duration::from_secs(args.seconds) + SETUP_AND_TEARDOWN_GRACE;
    tokio::time::timeout(deadline, measure(&args, address))
        .await
        .context("throughput benchmark deadline exceeded")?
}

/// Serve until the listener fails. Every connection is independent; a
/// malformed first byte closes that connection and nothing else.
pub(crate) async fn serve(listener: TcpListener, chunk_bytes: usize) -> Result<serde_json::Value> {
    let chunk: Arc<[u8]> = vec![0u8; chunk_bytes].into();
    loop {
        let (mut stream, peer) = listener.accept().await.context("accept throughput client")?;
        let chunk = Arc::clone(&chunk);
        tokio::spawn(async move {
            if let Err(error) = serve_one(&mut stream, &chunk).await {
                eprintln!("throughput connection from {peer} ended: {error:#}");
            }
        });
    }
}

async fn serve_one(stream: &mut TcpStream, chunk: &[u8]) -> Result<()> {
    stream.set_nodelay(true)?;
    let direction = Direction::from_wire(stream.read_u8().await.context("read direction byte")?)
        .context("unknown direction byte")?;
    if direction == Direction::Latency {
        let mut echo = [0u8; ECHO_BYTES];
        while stream.read_exact(&mut echo).await.is_ok() {
            stream.write_all(&echo).await?;
        }
        return Ok(());
    }
    let (mut reader, mut writer) = stream.split();
    // The half that does not apply never completes, so the select ends on
    // the client's signal alone: EOF or a reset on the drain, a failed write
    // on the fill. A client that drops with unread bytes resets; that is
    // the normal end of a trial, not a failure to report.
    let drain = async {
        if !direction.sends() {
            std::future::pending::<()>().await;
        }
        let mut sink = vec![0u8; chunk.len()];
        while let Ok(count) = reader.read(&mut sink).await {
            if count == 0 {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    let fill = async {
        if !direction.receives() {
            std::future::pending::<()>().await;
        }
        loop {
            if writer.write_all(chunk).await.is_err() {
                return Ok::<_, anyhow::Error>(());
            }
        }
    };
    tokio::select! {
        result = drain => result,
        result = fill => result,
    }
}

async fn measure(args: &Args, address: SocketAddr) -> Result<serde_json::Value> {
    let barrier = Arc::new(tokio::sync::Barrier::new(args.streams));
    let chunk: Arc<[u8]> = vec![0u8; args.chunk_bytes].into();
    let duration = Duration::from_secs(args.seconds);
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..args.streams {
        let barrier = Arc::clone(&barrier);
        let chunk = Arc::clone(&chunk);
        let direction = args.direction;
        jobs.spawn(async move {
            let mut stream = TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            stream.write_u8(direction.wire()).await?;
            barrier.wait().await;
            let stop = Instant::now() + duration;
            if direction == Direction::Latency {
                return echo_round_trips(&mut stream, stop).await;
            }
            let (mut reader, mut writer) = stream.split();
            // Neither side half-closes: at the deadline the whole socket is
            // dropped, as iperf3 ends a trial. A proxied path may turn a
            // half-close into a reset after its own deadline, which is not
            // the transfer being measured.
            let send = async {
                let mut sent = 0u64;
                if direction.sends() {
                    while Instant::now() < stop {
                        writer.write_all(&chunk).await?;
                        sent += chunk.len() as u64;
                    }
                }
                Ok::<_, anyhow::Error>(sent)
            };
            let receive = async {
                let mut received = 0u64;
                if direction.receives() {
                    let mut sink = vec![0u8; chunk.len()];
                    while Instant::now() < stop {
                        let n = reader.read(&mut sink).await?;
                        if n == 0 {
                            bail!("server closed the download early");
                        }
                        received += n as u64;
                    }
                }
                Ok::<_, anyhow::Error>(received)
            };
            let (sent, received) = tokio::try_join!(send, receive)?;
            Ok::<_, anyhow::Error>(Moved {
                sent,
                received,
                round_trips_ms: Vec::new(),
            })
        });
    }
    let started = Instant::now();
    let mut total = Moved::default();
    while let Some(result) = jobs.join_next().await {
        let moved = result??;
        total.sent += moved.sent;
        total.received += moved.received;
        total.round_trips_ms.extend(moved.round_trips_ms);
    }
    let elapsed = started.elapsed();
    ensure!(
        total.sent + total.received > 0 || !total.round_trips_ms.is_empty(),
        "no bytes moved"
    );
    let megabits = |bytes: u64| bytes as f64 * 8.0 / 1e6 / elapsed.as_secs_f64().max(1e-9);
    Ok(serde_json::json!({
        "throughput": {"address": address.to_string(), "direction": args.direction.as_str(),
            "streams": args.streams, "seconds": args.seconds, "chunk_bytes": args.chunk_bytes,
            "version": env!("CARGO_PKG_VERSION")},
        "metrics": {
            "bytes_sent": {"unit": "bytes", "samples": [total.sent]},
            "bytes_received": {"unit": "bytes", "samples": [total.received]},
            "send_megabits_per_sec": {"unit": "megabits_per_second", "samples": [megabits(total.sent)]},
            "receive_megabits_per_sec": {"unit": "megabits_per_second", "samples": [megabits(total.received)]},
            "round_trip_ms": {"unit": "milliseconds", "samples": total.round_trips_ms},
            "elapsed_seconds": {"unit": "seconds", "samples": [elapsed.as_secs_f64()]},
            "streams": {"unit": "count", "samples": [args.streams]}
        }
    }))
}

/// What one stream moved: bytes each way for the bulk directions, one
/// timing per echo for latency.
#[derive(Default)]
struct Moved {
    sent: u64,
    received: u64,
    round_trips_ms: Vec<f64>,
}

async fn echo_round_trips(stream: &mut TcpStream, stop: Instant) -> Result<Moved> {
    let payload = [0x5au8; ECHO_BYTES];
    let mut echo = [0u8; ECHO_BYTES];
    let mut round_trips_ms = Vec::new();
    while Instant::now() < stop {
        let started = Instant::now();
        stream.write_all(&payload).await?;
        stream
            .read_exact(&mut echo)
            .await
            .context("server closed the echo early")?;
        ensure!(echo == payload, "echo payload corrupted");
        round_trips_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    stream.shutdown().await?;
    Ok(Moved {
        sent: (round_trips_ms.len() * ECHO_BYTES) as u64,
        received: (round_trips_ms.len() * ECHO_BYTES) as u64,
        round_trips_ms,
    })
}

#[cfg(test)]
mod tests;
