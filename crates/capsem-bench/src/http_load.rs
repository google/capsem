//! Duration-driven HTTP load without a thread per simulated client.

use anyhow::{bail, Context, Result};
use clap::Parser;
use reqwest::Client;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// HTTP(S) endpoint to request.
    #[arg(long)]
    pub(crate) target: String,
    /// Number of concurrent request loops.
    #[arg(long)]
    pub(crate) concurrency: usize,
    /// Minimum time to drive requests. Every loop makes at least one request.
    #[arg(long)]
    pub(crate) duration_s: f64,
    /// Per-request timeout.
    #[arg(long, default_value_t = 30_000)]
    pub(crate) timeout_ms: u64,
}

#[derive(Debug)]
struct Sample {
    latency_ms: f64,
    bytes: usize,
    error: Option<String>,
    cold: bool,
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
    cold_p50_ms: f64,
    cold_p95_ms: f64,
    warm_p50_ms: f64,
    warm_p95_ms: f64,
    response_body_bytes: usize,
    error_distribution: BTreeMap<String, usize>,
}

pub(crate) async fn run(args: Args) -> Result<Report> {
    validate(&args)?;
    let duration = Duration::from_secs_f64(args.duration_s);
    let timeout = Duration::from_millis(args.timeout_ms);
    let target = reqwest::Url::parse(&args.target).context("parse --target")?;
    if !matches!(target.scheme(), "http" | "https") {
        bail!("--target must start with http:// or https://");
    }

    let clients = (0..args.concurrency)
        .map(|_| build_client(timeout))
        .collect::<Result<Vec<_>>>()?;
    let started = Instant::now();
    let deadline = started + duration;
    let mut tasks = tokio::task::JoinSet::new();
    for client in clients {
        tasks.spawn(drive_client(client, target.clone(), deadline));
    }

    let mut samples = Vec::new();
    while let Some(result) = tasks.join_next().await {
        samples.extend(result.context("HTTP load task panicked")?);
    }
    Ok(summarize(samples, args.concurrency, duration, started.elapsed()))
}

fn build_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(1)
        .timeout(timeout)
        .build()
        .context("build logical HTTP load client")
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

async fn drive_client(client: Client, target: reqwest::Url, deadline: Instant) -> Vec<Sample> {
    let mut samples = Vec::new();
    let mut cold = true;
    while cold || Instant::now() < deadline {
        let started = Instant::now();
        let outcome = request(&client, target.clone()).await;
        let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let (bytes, error) = match outcome {
            Ok(bytes) => (bytes, None),
            Err(error) => (0, Some(error)),
        };
        samples.push(Sample {
            latency_ms,
            bytes,
            error,
            cold,
        });
        cold = false;
    }
    samples
}

async fn request(client: &Client, target: reqwest::Url) -> std::result::Result<usize, String> {
    let response = client.get(target).send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|error| error.to_string())?.len();
    if status.is_success() {
        Ok(bytes)
    } else {
        Err(format!("HTTP {}", status.as_u16()))
    }
}

fn summarize(samples: Vec<Sample>, concurrency: usize, duration: Duration, elapsed: Duration) -> Report {
    let total_requests = samples.len();
    let errors = samples.iter().filter(|sample| sample.error.is_some()).count();
    let successful = total_requests - errors;
    let mut latencies = successful_latencies(&samples, |_| true);
    let mut cold_latencies = successful_latencies(&samples, |sample| sample.cold);
    let mut warm_latencies = successful_latencies(&samples, |sample| !sample.cold);
    let mut error_distribution = BTreeMap::new();
    for error in samples.iter().filter_map(|sample| sample.error.as_ref()) {
        *error_distribution.entry(error.clone()).or_insert(0) += 1;
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
        cold_p50_ms: percentile(&mut cold_latencies, 0.50),
        cold_p95_ms: percentile(&mut cold_latencies, 0.95),
        warm_p50_ms: percentile(&mut warm_latencies, 0.50),
        warm_p95_ms: percentile(&mut warm_latencies, 0.95),
        response_body_bytes: samples.iter().map(|sample| sample.bytes).sum(),
        error_distribution,
    }
}

fn successful_latencies(samples: &[Sample], include: impl Fn(&Sample) -> bool) -> Vec<f64> {
    samples
        .iter()
        .filter(|sample| sample.error.is_none() && include(sample))
        .map(|sample| sample.latency_ms)
        .collect()
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
