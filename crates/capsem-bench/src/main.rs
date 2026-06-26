use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures::future::try_join_all;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = "0.4.0-rust";
const SECRET_SHAPED_MARKER: &str = "capsem_test_";

#[derive(Parser, Debug)]
#[command(about = "Capsem benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run deterministic protocol scenarios against capsem-mock-server.
    Protocol(ProtocolArgs),
    /// Compare host-direct and guest-through-Capsem artifacts.
    Delta(DeltaArgs),
}

#[derive(Parser, Debug)]
struct ProtocolArgs {
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    requests: usize,
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    scenarios: Option<String>,
    #[arg(long, default_value = "host_direct")]
    lane: String,
    #[arg(long, default_value = "/tmp/capsem-benchmark.json")]
    json_out: PathBuf,
}

#[derive(Parser, Debug)]
struct DeltaArgs {
    #[arg(long)]
    host: PathBuf,
    #[arg(long)]
    guest: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-delta.json")]
    json_out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    name: &'static str,
    path: &'static str,
    expected_status: u16,
    expected_bytes: Option<usize>,
    body_kind: &'static str,
    required_text: Option<&'static str>,
    secret_shaped_fixture: bool,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "tiny_http",
        path: "/tiny",
        expected_status: 200,
        expected_bytes: Some(24),
        body_kind: "tiny",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "http_1mb",
        path: "/bytes/1mb",
        expected_status: 200,
        expected_bytes: Some(1024 * 1024),
        body_kind: "1mb",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "sse_model",
        path: "/sse/model",
        expected_status: 200,
        expected_bytes: None,
        body_kind: "sse",
        required_text: Some("model.tool_call"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "model_json_response",
        path: "/model/response",
        expected_status: 200,
        expected_bytes: None,
        body_kind: "model_json",
        required_text: Some("tool_calls"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "credential_response",
        path: "/credential/response",
        expected_status: 200,
        expected_bytes: None,
        body_kind: "credential",
        required_text: None,
        secret_shaped_fixture: true,
    },
    Scenario {
        name: "denied_target",
        path: "/deny-target",
        expected_status: 200,
        expected_bytes: None,
        body_kind: "tiny",
        required_text: None,
        secret_shaped_fixture: false,
    },
];

#[derive(Debug, Serialize, Deserialize)]
struct Artifact {
    version: String,
    timestamp: f64,
    hostname: String,
    benchmark: String,
    mock_server_protocol: ProtocolReport,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProtocolReport {
    version: String,
    lane: String,
    base_url: String,
    total_requests: usize,
    concurrency: usize,
    timeout_ms: u64,
    selected_scenarios: Vec<String>,
    scenarios: Vec<ScenarioResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ScenarioResult {
    name: String,
    path: String,
    body_kind: String,
    total_requests: usize,
    concurrency: usize,
    successful: usize,
    failed: usize,
    total_duration_ms: f64,
    requests_per_sec: f64,
    transfer_bytes: u64,
    bytes_per_sec: f64,
    latency_ms: LatencySummary,
    errors: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret_shaped_fixture_seen: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_secret_stored_in_result: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LatencySummary {
    min: f64,
    max: f64,
    mean: f64,
    p50: f64,
    p95: f64,
    p99: f64,
}

#[derive(Debug)]
struct RequestSample {
    status: u16,
    size: usize,
    latency_ms: f64,
    error: Option<String>,
    required_text_present: bool,
    secret_shaped_fixture_seen: bool,
}

#[derive(Debug, Serialize)]
struct DeltaArtifact {
    version: String,
    timestamp: f64,
    benchmark: String,
    abstraction_delta: DeltaReport,
}

#[derive(Debug, Serialize)]
struct DeltaReport {
    host_artifact: String,
    guest_artifact: String,
    host_lane: String,
    guest_lane: String,
    scenarios: Vec<ScenarioDelta>,
}

#[derive(Debug, Serialize)]
struct ScenarioDelta {
    name: String,
    host_requests_per_sec: f64,
    guest_requests_per_sec: f64,
    rps_ratio_guest_over_host: f64,
    host_bytes_per_sec: f64,
    guest_bytes_per_sec: f64,
    throughput_ratio_guest_over_host: f64,
    p50_delta_ms: f64,
    p95_delta_ms: f64,
    p99_delta_ms: f64,
    error_delta: isize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Protocol(ProtocolArgs {
        base_url: None,
        requests: 50_000,
        concurrency: 64,
        timeout_ms: 30_000,
        scenarios: None,
        lane: "host_direct".to_string(),
        json_out: PathBuf::from("/tmp/capsem-benchmark.json"),
    })) {
        Command::Protocol(args) => {
            let artifact = run_protocol(args).await?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        Command::Delta(args) => {
            let artifact = run_delta(args)?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
    }
    Ok(())
}

async fn run_protocol(args: ProtocolArgs) -> Result<Artifact> {
    if args.requests == 0 {
        bail!("--requests must be greater than zero");
    }
    if args.concurrency == 0 {
        bail!("--concurrency must be greater than zero");
    }
    let base_url = args
        .base_url
        .or_else(|| std::env::var("CAPSEM_MOCK_SERVER_BASE_URL").ok())
        .context("CAPSEM_MOCK_SERVER_BASE_URL or --base-url is required")?
        .trim_end_matches('/')
        .to_string();
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        bail!("--base-url must start with http:// or https://");
    }
    let selected = select_scenarios(args.scenarios.as_deref())?;
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(args.concurrency)
        .timeout(Duration::from_millis(args.timeout_ms))
        .build()
        .context("build HTTP client")?;

    let mut scenario_results = Vec::with_capacity(selected.len());
    for scenario in &selected {
        scenario_results.push(
            run_http_scenario(
                client.clone(),
                &base_url,
                *scenario,
                args.requests,
                args.concurrency,
                Duration::from_millis(args.timeout_ms),
            )
            .await?,
        );
    }

    let artifact = Artifact {
        version: VERSION.to_string(),
        timestamp: timestamp(),
        hostname: hostname(),
        benchmark: "capsem-bench-rs".to_string(),
        mock_server_protocol: ProtocolReport {
            version: "1.1-rust".to_string(),
            lane: args.lane,
            base_url,
            total_requests: args.requests,
            concurrency: args.concurrency,
            timeout_ms: args.timeout_ms,
            selected_scenarios: selected
                .iter()
                .map(|scenario| scenario.name.to_string())
                .collect(),
            scenarios: scenario_results,
        },
    };
    write_json(&args.json_out, &artifact)?;
    Ok(artifact)
}

async fn run_http_scenario(
    client: Client,
    base_url: &str,
    scenario: Scenario,
    total_requests: usize,
    concurrency: usize,
    timeout: Duration,
) -> Result<ScenarioResult> {
    let url = format!("{base_url}{}", scenario.path);
    let workers = concurrency.min(total_requests);
    let per_worker = total_requests / workers;
    let remainder = total_requests % workers;
    let started = Instant::now();
    let tasks = (0..workers).map(|idx| {
        let client = client.clone();
        let url = url.clone();
        let count = per_worker + usize::from(idx < remainder);
        tokio::spawn(async move {
            let mut out = Vec::with_capacity(count);
            for _ in 0..count {
                out.push(run_one_request(&client, &url, scenario, timeout).await);
            }
            out
        })
    });
    let joined = try_join_all(tasks)
        .await
        .context("join benchmark workers")?;
    let wall_time = started.elapsed();
    let samples: Vec<RequestSample> = joined.into_iter().flatten().collect();
    Ok(summarize(
        scenario,
        &samples,
        wall_time,
        total_requests,
        concurrency,
    ))
}

async fn run_one_request(
    client: &Client,
    url: &str,
    scenario: Scenario,
    timeout: Duration,
) -> RequestSample {
    let started = Instant::now();
    match tokio::time::timeout(timeout, client.get(url).send()).await {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            match response.bytes().await {
                Ok(body) => {
                    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                    RequestSample {
                        status,
                        size: body.len(),
                        latency_ms,
                        error: None,
                        required_text_present: required_text_present(&body, scenario),
                        secret_shaped_fixture_seen: secret_fixture_seen(&body, scenario),
                    }
                }
                Err(error) => RequestSample {
                    status,
                    size: 0,
                    latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                    error: Some(format!("body: {error}")),
                    required_text_present: false,
                    secret_shaped_fixture_seen: false,
                },
            }
        }
        Ok(Err(error)) => RequestSample {
            status: 0,
            size: 0,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            error: Some(format!("request: {error}")),
            required_text_present: false,
            secret_shaped_fixture_seen: false,
        },
        Err(_) => RequestSample {
            status: 0,
            size: 0,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            error: Some("timeout".to_string()),
            required_text_present: false,
            secret_shaped_fixture_seen: false,
        },
    }
}

fn summarize(
    scenario: Scenario,
    samples: &[RequestSample],
    wall_time: Duration,
    total_requests: usize,
    concurrency: usize,
) -> ScenarioResult {
    let successful = samples
        .iter()
        .filter(|sample| result_ok(sample, scenario))
        .count();
    let mut errors = BTreeMap::new();
    for sample in samples {
        if let Some(error) = &sample.error {
            *errors.entry(error.clone()).or_insert(0) += 1;
        } else if sample.status != scenario.expected_status {
            *errors
                .entry(format!("status:{}", sample.status))
                .or_insert(0) += 1;
        } else if !sample.required_text_present {
            *errors
                .entry("required_text_missing".to_string())
                .or_insert(0) += 1;
        } else if let Some(expected_bytes) = scenario.expected_bytes {
            if sample.size != expected_bytes {
                *errors
                    .entry(format!("bytes:{}!={expected_bytes}", sample.size))
                    .or_insert(0) += 1;
            }
        }
    }
    let transfer_bytes = samples.iter().map(|sample| sample.size as u64).sum::<u64>();
    let duration_s = wall_time.as_secs_f64();
    let secret_seen = scenario.secret_shaped_fixture.then(|| {
        samples
            .iter()
            .any(|sample| sample.secret_shaped_fixture_seen)
    });
    ScenarioResult {
        name: scenario.name.to_string(),
        path: scenario.path.to_string(),
        body_kind: scenario.body_kind.to_string(),
        total_requests,
        concurrency,
        successful,
        failed: total_requests.saturating_sub(successful),
        total_duration_ms: round1(duration_s * 1000.0),
        requests_per_sec: round1(total_requests as f64 / duration_s),
        transfer_bytes,
        bytes_per_sec: round1(transfer_bytes as f64 / duration_s),
        latency_ms: latency_summary(samples.iter().map(|sample| sample.latency_ms).collect()),
        errors,
        secret_shaped_fixture_seen: secret_seen,
        raw_secret_stored_in_result: scenario.secret_shaped_fixture.then_some(false),
    }
}

fn run_delta(args: DeltaArgs) -> Result<DeltaArtifact> {
    let host = read_artifact(&args.host)?;
    let guest = read_artifact(&args.guest)?;
    let host_rows = rows_by_name(&host.mock_server_protocol.scenarios);
    let guest_rows = rows_by_name(&guest.mock_server_protocol.scenarios);
    let mut scenarios = Vec::new();
    for (name, host_row) in host_rows {
        let Some(guest_row) = guest_rows.get(name) else {
            continue;
        };
        scenarios.push(ScenarioDelta {
            name: name.to_string(),
            host_requests_per_sec: host_row.requests_per_sec,
            guest_requests_per_sec: guest_row.requests_per_sec,
            rps_ratio_guest_over_host: ratio(guest_row.requests_per_sec, host_row.requests_per_sec),
            host_bytes_per_sec: host_row.bytes_per_sec,
            guest_bytes_per_sec: guest_row.bytes_per_sec,
            throughput_ratio_guest_over_host: ratio(
                guest_row.bytes_per_sec,
                host_row.bytes_per_sec,
            ),
            p50_delta_ms: round1(guest_row.latency_ms.p50 - host_row.latency_ms.p50),
            p95_delta_ms: round1(guest_row.latency_ms.p95 - host_row.latency_ms.p95),
            p99_delta_ms: round1(guest_row.latency_ms.p99 - host_row.latency_ms.p99),
            error_delta: guest_row.failed as isize - host_row.failed as isize,
        });
    }
    if scenarios.is_empty() {
        bail!("host and guest artifacts have no shared scenarios");
    }
    let artifact = DeltaArtifact {
        version: VERSION.to_string(),
        timestamp: timestamp(),
        benchmark: "capsem-bench-rs-delta".to_string(),
        abstraction_delta: DeltaReport {
            host_artifact: args.host.display().to_string(),
            guest_artifact: args.guest.display().to_string(),
            host_lane: host.mock_server_protocol.lane,
            guest_lane: guest.mock_server_protocol.lane,
            scenarios,
        },
    };
    write_json(&args.json_out, &artifact)?;
    Ok(artifact)
}

fn select_scenarios(selected: Option<&str>) -> Result<Vec<Scenario>> {
    let by_name = SCENARIOS
        .iter()
        .map(|scenario| (scenario.name, *scenario))
        .collect::<BTreeMap<_, _>>();
    let Some(selected) = selected else {
        return Ok(SCENARIOS.to_vec());
    };
    let mut out = Vec::new();
    for name in selected
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let Some(scenario) = by_name.get(name).copied() else {
            let valid = by_name.keys().copied().collect::<Vec<_>>().join(", ");
            bail!("unknown scenario {name:?}; valid: {valid}");
        };
        out.push(scenario);
    }
    if out.is_empty() {
        bail!("at least one scenario is required");
    }
    Ok(out)
}

fn result_ok(sample: &RequestSample, scenario: Scenario) -> bool {
    if sample.error.is_some() || sample.status != scenario.expected_status {
        return false;
    }
    if let Some(expected_bytes) = scenario.expected_bytes {
        if sample.size != expected_bytes {
            return false;
        }
    }
    sample.required_text_present
}

fn required_text_present(body: &[u8], scenario: Scenario) -> bool {
    scenario
        .required_text
        .map(|needle| {
            body.windows(needle.len())
                .any(|window| window == needle.as_bytes())
        })
        .unwrap_or(true)
}

fn secret_fixture_seen(body: &[u8], scenario: Scenario) -> bool {
    scenario.secret_shaped_fixture
        && body
            .windows(SECRET_SHAPED_MARKER.len())
            .any(|window| window == SECRET_SHAPED_MARKER.as_bytes())
}

fn latency_summary(mut values: Vec<f64>) -> LatencySummary {
    values.sort_by(|a, b| a.total_cmp(b));
    if values.is_empty() {
        return LatencySummary {
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            p50: 0.0,
            p95: 0.0,
            p99: 0.0,
        };
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    LatencySummary {
        min: round1(values[0]),
        max: round1(values[values.len() - 1]),
        mean: round1(mean),
        p50: round1(percentile(&values, 50.0)),
        p95: round1(percentile(&values, 95.0)),
        p99: round1(percentile(&values, 99.0)),
    }
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (percentile / 100.0) * (sorted.len().saturating_sub(1)) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let weight = rank - lo as f64;
        sorted[lo] * (1.0 - weight) + sorted[hi] * weight
    }
}

fn rows_by_name(rows: &[ScenarioResult]) -> BTreeMap<&str, &ScenarioResult> {
    rows.iter().map(|row| (row.name.as_str(), row)).collect()
}

fn read_artifact(path: &Path) -> Result<Artifact> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", path.display()))
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        round3(numerator / denominator)
    }
}

fn timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_scenarios_are_strict() {
        let selected = select_scenarios(Some("tiny_http,model_json_response")).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|scenario| scenario.name)
                .collect::<Vec<_>>(),
            vec!["tiny_http", "model_json_response"]
        );
        assert!(select_scenarios(Some("bogus")).is_err());
    }

    #[test]
    fn latency_percentiles_are_interpolated() {
        let summary = latency_summary(vec![1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(summary.min, 1.0);
        assert_eq!(summary.p50, 3.0);
        assert_eq!(summary.p95, 80.8);
        assert_eq!(summary.p99, 96.2);
        assert_eq!(summary.max, 100.0);
    }

    #[test]
    fn result_ok_checks_status_size_and_required_text() {
        let scenario = SCENARIOS
            .iter()
            .copied()
            .find(|scenario| scenario.name == "tiny_http")
            .unwrap();
        assert!(result_ok(
            &RequestSample {
                status: 200,
                size: 24,
                latency_ms: 1.0,
                error: None,
                required_text_present: true,
                secret_shaped_fixture_seen: false,
            },
            scenario
        ));
        assert!(!result_ok(
            &RequestSample {
                status: 200,
                size: 23,
                latency_ms: 1.0,
                error: None,
                required_text_present: true,
                secret_shaped_fixture_seen: false,
            },
            scenario
        ));
    }

    #[test]
    fn delta_computes_abstraction_cost() {
        let host = ScenarioResult {
            name: "tiny_http".to_string(),
            path: "/tiny".to_string(),
            body_kind: "tiny".to_string(),
            total_requests: 100,
            concurrency: 10,
            successful: 100,
            failed: 0,
            total_duration_ms: 10.0,
            requests_per_sec: 10_000.0,
            transfer_bytes: 2400,
            bytes_per_sec: 240_000.0,
            latency_ms: LatencySummary {
                min: 0.1,
                max: 2.0,
                mean: 0.5,
                p50: 0.4,
                p95: 1.0,
                p99: 1.5,
            },
            errors: BTreeMap::new(),
            secret_shaped_fixture_seen: None,
            raw_secret_stored_in_result: None,
        };
        let guest = ScenarioResult {
            requests_per_sec: 2_500.0,
            bytes_per_sec: 60_000.0,
            latency_ms: LatencySummary {
                p50: 1.4,
                p95: 5.0,
                p99: 9.5,
                ..host.latency_ms.clone()
            },
            failed: 2,
            ..host.clone()
        };
        let host_values = [host];
        let guest_values = [guest];
        let host_rows = rows_by_name(&host_values);
        let guest_rows = rows_by_name(&guest_values);
        let host_row = host_rows["tiny_http"];
        let guest_row = guest_rows["tiny_http"];
        assert_eq!(
            ratio(guest_row.requests_per_sec, host_row.requests_per_sec),
            0.25
        );
        assert_eq!(
            round1(guest_row.latency_ms.p95 - host_row.latency_ms.p95),
            4.0
        );
        assert_eq!(guest_row.failed as isize - host_row.failed as isize, 2);
    }
}
