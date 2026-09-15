//! Redis transport collector shared by the native host and musl guest binary.
use anyhow::{ensure, Context, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct Args {
    #[arg(long)]
    pub address: std::net::SocketAddr,
    #[arg(long, default_value_t = 10000)]
    pub requests: usize,
    #[arg(long, default_value_t = 32)]
    pub concurrency: usize,
    #[arg(long, default_value_t = 16)]
    pub pipeline: usize,
    #[arg(long, default_value_t = 30000)]
    pub timeout_ms: u64,
}

pub(crate) async fn run(args: Args) -> Result<serde_json::Value> {
    ensure!((1..=128).contains(&args.concurrency), "concurrency must be 1..128");
    ensure!((1..=128).contains(&args.pipeline), "pipeline must be 1..128");
    ensure!(
        (args.concurrency..=1_000_000).contains(&args.requests),
        "requests must be concurrency..1000000"
    );
    ensure!((1..=300_000).contains(&args.timeout_ms), "timeout must be 1..300000 ms");
    tokio::time::timeout(Duration::from_millis(args.timeout_ms), collect(&args))
        .await
        .context("Redis benchmark deadline exceeded")?
}

async fn collect(args: &Args) -> Result<serde_json::Value> {
    let barrier = Arc::new(tokio::sync::Barrier::new(args.concurrency));
    let mut jobs = tokio::task::JoinSet::new();
    let started = Instant::now();
    for worker in 0..args.concurrency {
        let address = args.address;
        let pipeline = args.pipeline;
        let count = args.requests / args.concurrency + usize::from(worker < args.requests % args.concurrency);
        let barrier = barrier.clone();
        jobs.spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let request = b"*1\r\n$4\r\nPING\r\n".repeat(pipeline);
            let mut response = vec![0; 7 * pipeline];
            let mut remaining = count;
            let mut samples = Vec::with_capacity(count.div_ceil(pipeline));
            barrier.wait().await;
            while remaining > 0 {
                let batch = remaining.min(pipeline);
                let started = Instant::now();
                stream.write_all(&request[..14 * batch]).await?;
                stream.read_exact(&mut response[..7 * batch]).await?;
                ensure!(
                    response[..7 * batch].chunks_exact(7).all(|reply| reply == b"+PONG\r\n"),
                    "Redis returned an invalid PONG"
                );
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
                remaining -= batch;
            }
            Ok::<_, anyhow::Error>(samples)
        });
    }
    let mut samples = Vec::new();
    while let Some(result) = jobs.join_next().await {
        samples.extend(result??);
    }
    let elapsed = started.elapsed();
    Ok(serde_json::json!({
        "redis": {"address": args.address.to_string(), "requests": args.requests,
            "concurrency": args.concurrency, "pipeline": args.pipeline,
            "operation": "PING", "payload_bytes": 0, "version": env!("CARGO_PKG_VERSION")},
        "metrics": {
            "batch_latency_ms": {"unit": "milliseconds", "samples": samples},
            "requests_per_sec": {"unit": "requests_per_second", "samples": [crate::rates::per_second(args.requests, elapsed)]},
            "elapsed_seconds": {"unit": "seconds", "samples": [elapsed.as_secs_f64()]},
            "completed_requests": {"unit": "count", "samples": [args.requests]}
        }
    }))
}

#[cfg(test)]
mod tests;
