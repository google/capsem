//! Duration-driven DNS load without a thread per simulated client.

use crate::protocol::run_one_dns_query;
use crate::scenarios::{build_dns_query, RequestSample};
use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// UDP DNS endpoint to query.
    #[arg(long, default_value = "127.0.0.1:53")]
    pub(crate) target: SocketAddr,
    /// DNS name to resolve.
    #[arg(long, default_value = "load-test.capsem-bogus")]
    pub(crate) qname: String,
    /// Numeric DNS query type.
    #[arg(long, default_value_t = 1)]
    pub(crate) qtype: u16,
    /// Number of concurrent query loops.
    #[arg(long)]
    pub(crate) concurrency: usize,
    /// Minimum time to drive queries. Every loop makes at least one query.
    #[arg(long)]
    pub(crate) duration_s: f64,
    /// Per-query timeout.
    #[arg(long, default_value_t = 5_000)]
    pub(crate) timeout_ms: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    concurrency: usize,
    duration_s: f64,
    elapsed_s: f64,
    total_requests: usize,
    errors: usize,
    error_ratio: f64,
    rps: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p999_ms: f64,
    request_bytes: usize,
    response_bytes: usize,
    decision_distribution: BTreeMap<String, usize>,
    error_distribution: BTreeMap<String, usize>,
}

pub(crate) async fn run(args: Args) -> Result<Report> {
    validate(&args)?;
    let duration = Duration::from_secs_f64(args.duration_s);
    let timeout = Duration::from_millis(args.timeout_ms);
    let request_size = build_dns_query(&args.qname, args.qtype, 1)?.len();
    let started = Instant::now();
    let deadline = started + duration;
    let mut tasks = tokio::task::JoinSet::new();
    for client in 0..args.concurrency {
        let qname = args.qname.clone();
        tasks.spawn(drive_client(client, args.target, qname, args.qtype, timeout, deadline));
    }

    let mut samples = Vec::new();
    while let Some(result) = tasks.join_next().await {
        samples.extend(result.context("DNS load task panicked")??);
    }
    Ok(summarize(
        samples,
        args.concurrency,
        duration,
        started.elapsed(),
        request_size,
    ))
}

fn validate(args: &Args) -> Result<()> {
    if args.concurrency == 0 {
        bail!("--concurrency must be greater than zero");
    }
    if !args.duration_s.is_finite() || args.duration_s <= 0.0 {
        bail!("--duration-s must be a finite value greater than zero");
    }
    if args.timeout_ms == 0 {
        bail!("--timeout-ms must be greater than zero");
    }
    Ok(())
}

async fn drive_client(
    client: usize,
    target: SocketAddr,
    qname: String,
    qtype: u16,
    timeout: Duration,
    deadline: Instant,
) -> Result<Vec<RequestSample>> {
    let bind: SocketAddr = if target.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse()?;
    let socket = UdpSocket::bind(bind).await.context("bind logical DNS load client")?;
    let mut samples = Vec::new();
    let mut sequence = 0_u16;
    while samples.is_empty() || Instant::now() < deadline {
        let query_id = (client as u16).wrapping_mul(4099).wrapping_add(sequence);
        samples.push(run_one_dns_query(&socket, target, &qname, qtype, query_id, timeout).await);
        sequence = sequence.wrapping_add(1);
    }
    Ok(samples)
}

fn summarize(
    samples: Vec<RequestSample>,
    concurrency: usize,
    duration: Duration,
    elapsed: Duration,
    request_size: usize,
) -> Report {
    let total_requests = samples.len();
    let errors = samples
        .iter()
        .filter(|sample| sample.error.is_some() || sample.status == 2)
        .count();
    let successful = total_requests - errors;
    let mut latencies = samples
        .iter()
        .filter(|sample| sample.error.is_none() && sample.status != 2)
        .map(|sample| sample.latency_ms)
        .collect::<Vec<_>>();
    let mut decision_distribution = BTreeMap::new();
    let mut error_distribution = BTreeMap::new();
    for sample in &samples {
        if let Some(error) = &sample.error {
            *error_distribution.entry(error.clone()).or_insert(0) += 1;
            *decision_distribution.entry("transport_error".to_string()).or_insert(0) += 1;
        } else if sample.status == 2 {
            *error_distribution.entry("SERVFAIL".to_string()).or_insert(0) += 1;
            *decision_distribution.entry("error".to_string()).or_insert(0) += 1;
        } else {
            let decision = match sample.status {
                0 => "allowed".to_string(),
                3 => "denied".to_string(),
                status => format!("rcode_{status}"),
            };
            *decision_distribution.entry(decision).or_insert(0) += 1;
        }
    }
    let elapsed_s = elapsed.as_secs_f64();
    Report {
        concurrency,
        duration_s: duration.as_secs_f64(),
        elapsed_s,
        total_requests,
        errors,
        error_ratio: ratio(errors, total_requests),
        rps: if elapsed_s > 0.0 {
            successful as f64 / elapsed_s
        } else {
            0.0
        },
        p50_ms: percentile(&mut latencies, 0.50),
        p95_ms: percentile(&mut latencies, 0.95),
        p99_ms: percentile(&mut latencies, 0.99),
        p999_ms: percentile(&mut latencies, 0.999),
        request_bytes: request_size * total_requests,
        response_bytes: samples.iter().map(|sample| sample.size).sum(),
        decision_distribution,
        error_distribution,
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let position = quantile * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        values[lower]
    } else {
        let weight = position - lower as f64;
        values[lower] * (1.0 - weight) + values[upper] * weight
    }
}

#[cfg(test)]
mod tests;
