use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures::future::try_join_all;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

const VERSION: &str = "0.4.0-rust";
const SECRET_SHAPED_MARKER: &str = "capsem_test_";
const HTTP_REQUEST_ATTEMPTS: usize = 3;

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
    /// Run host-direct and guest-through-Capsem protocol lanes, then report delta.
    ProtocolDelta(ProtocolDeltaArgs),
    /// Compare host-direct and guest-through-Capsem artifacts.
    Delta(DeltaArgs),
}

#[derive(Parser, Debug)]
struct ProtocolArgs {
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    dns_udp_addr: Option<String>,
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

#[derive(Parser, Debug)]
struct ProtocolDeltaArgs {
    #[arg(long)]
    base_url: String,
    #[arg(long)]
    dns_udp_addr: Option<String>,
    #[arg(long)]
    guest_base_url: Option<String>,
    #[arg(long)]
    guest_dns_udp_addr: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    requests: usize,
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
    #[arg(long, default_value_t = 300)]
    guest_timeout_secs: u64,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    scenarios: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value = "capsem")]
    capsem_bin: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-protocol-delta.json")]
    json_out: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    name: &'static str,
    transport: ScenarioTransport,
    path: &'static str,
    method: HttpMethod,
    request_body: Option<&'static str>,
    expected_status: u16,
    expected_bytes: Option<usize>,
    body_kind: &'static str,
    required_text: Option<&'static str>,
    secret_shaped_fixture: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScenarioTransport {
    Http,
    DnsUdp { qtype: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HttpMethod {
    Get,
    PostJson,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "tiny_http",
        transport: ScenarioTransport::Http,
        path: "/tiny",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: Some(24),
        body_kind: "tiny",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "http_1mb",
        transport: ScenarioTransport::Http,
        path: "/bytes/1mb",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: Some(1024 * 1024),
        body_kind: "1mb",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "http_10mb",
        transport: ScenarioTransport::Http,
        path: "/bytes/10mb",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: Some(10 * 1024 * 1024),
        body_kind: "10mb",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "sse_model",
        transport: ScenarioTransport::Http,
        path: "/sse/model",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: None,
        body_kind: "sse",
        required_text: Some("model.tool_call"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "model_json_response",
        transport: ScenarioTransport::Http,
        path: "/model/response",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: None,
        body_kind: "model_json",
        required_text: Some("tool_calls"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "credential_response",
        transport: ScenarioTransport::Http,
        path: "/credential/response",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: None,
        body_kind: "credential",
        required_text: None,
        secret_shaped_fixture: true,
    },
    Scenario {
        name: "denied_target",
        transport: ScenarioTransport::Http,
        path: "/deny-target",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 200,
        expected_bytes: None,
        body_kind: "tiny",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "mcp_tools_list",
        transport: ScenarioTransport::Http,
        path: "/mcp",
        method: HttpMethod::PostJson,
        request_body: Some(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
        expected_status: 200,
        expected_bytes: None,
        body_kind: "mcp_jsonrpc",
        required_text: Some("fixture_lookup"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "mcp_tool_call",
        transport: ScenarioTransport::Http,
        path: "/mcp",
        method: HttpMethod::PostJson,
        request_body: Some(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fixture_lookup","arguments":{"query":"capsem-bench"}}}"#,
        ),
        expected_status: 200,
        expected_bytes: None,
        body_kind: "mcp_jsonrpc",
        required_text: Some("capsem-mock-server:mcp:fixture_lookup"),
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "dns_local_nxdomain",
        transport: ScenarioTransport::DnsUdp { qtype: 1 },
        path: "load-test.capsem-bogus",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 3,
        expected_bytes: None,
        body_kind: "dns_udp",
        required_text: None,
        secret_shaped_fixture: false,
    },
    Scenario {
        name: "dns_fixture_a",
        transport: ScenarioTransport::DnsUdp { qtype: 1 },
        path: "fixture.capsem.test",
        method: HttpMethod::Get,
        request_body: None,
        expected_status: 0,
        expected_bytes: None,
        body_kind: "dns_udp",
        required_text: None,
        secret_shaped_fixture: false,
    },
];

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Artifact {
    version: String,
    timestamp: f64,
    hostname: String,
    benchmark: String,
    mock_server_protocol: ProtocolReport,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProtocolReport {
    version: String,
    lane: String,
    base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dns_udp_addr: Option<String>,
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
    #[serde(default)]
    transport_retries: usize,
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
    attempts: usize,
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
struct ProtocolDeltaArtifact {
    version: String,
    timestamp: f64,
    benchmark: String,
    host: Artifact,
    guest: Artifact,
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
        dns_udp_addr: None,
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
        Command::ProtocolDelta(args) => {
            let artifact = run_protocol_delta(args).await?;
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
    let selected = select_scenarios(args.scenarios.as_deref())?;
    let needs_http = selected
        .iter()
        .any(|scenario| scenario.transport == ScenarioTransport::Http);
    let needs_dns = selected
        .iter()
        .any(|scenario| matches!(scenario.transport, ScenarioTransport::DnsUdp { .. }));
    let base_url = if needs_http {
        let base_url = args
            .base_url
            .or_else(|| std::env::var("CAPSEM_MOCK_SERVER_BASE_URL").ok())
            .context("CAPSEM_MOCK_SERVER_BASE_URL or --base-url is required for HTTP scenarios")?
            .trim_end_matches('/')
            .to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            bail!("--base-url must start with http:// or https://");
        }
        base_url
    } else {
        String::new()
    };
    let dns_udp_addr = if needs_dns {
        Some(
            args.dns_udp_addr
                .or_else(|| std::env::var("CAPSEM_MOCK_SERVER_DNS_UDP_ADDR").ok())
                .context(
                    "CAPSEM_MOCK_SERVER_DNS_UDP_ADDR or --dns-udp-addr is required for DNS scenarios",
                )?,
        )
    } else {
        None
    };
    let parsed_dns_udp_addr = dns_udp_addr
        .as_deref()
        .map(str::parse::<SocketAddr>)
        .transpose()
        .context("parse --dns-udp-addr")?;
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(args.concurrency)
        .timeout(Duration::from_millis(args.timeout_ms))
        .build()
        .context("build HTTP client")?;

    let mut scenario_results = Vec::with_capacity(selected.len());
    for scenario in &selected {
        let result = match scenario.transport {
            ScenarioTransport::Http => {
                run_http_scenario(
                    client.clone(),
                    &base_url,
                    *scenario,
                    args.requests,
                    args.concurrency,
                    Duration::from_millis(args.timeout_ms),
                )
                .await?
            }
            ScenarioTransport::DnsUdp { qtype } => {
                let addr = parsed_dns_udp_addr.context("DNS scenario missing UDP address")?;
                run_dns_scenario(
                    addr,
                    *scenario,
                    qtype,
                    args.requests,
                    args.concurrency,
                    Duration::from_millis(args.timeout_ms),
                )
                .await?
            }
        };
        scenario_results.push(result);
    }
    validate_successful_scenarios(&scenario_results, &args.lane)?;

    let artifact = Artifact {
        version: VERSION.to_string(),
        timestamp: timestamp(),
        hostname: hostname(),
        benchmark: "capsem-bench-rs".to_string(),
        mock_server_protocol: ProtocolReport {
            version: "1.1-rust".to_string(),
            lane: args.lane,
            base_url,
            dns_udp_addr,
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

async fn run_protocol_delta(args: ProtocolDeltaArgs) -> Result<ProtocolDeltaArtifact> {
    let selected = select_scenarios(args.scenarios.as_deref())?;
    let selected_names = selected
        .iter()
        .map(|scenario| scenario.name)
        .collect::<Vec<_>>()
        .join(",");
    let needs_dns = selected
        .iter()
        .any(|scenario| matches!(scenario.transport, ScenarioTransport::DnsUdp { .. }));
    let host_dns_udp_addr = args.dns_udp_addr.clone().or_else(|| {
        std::env::var("CAPSEM_MOCK_SERVER_DNS_UDP_ADDR")
            .ok()
            .filter(|value| !value.is_empty())
    });
    if needs_dns && host_dns_udp_addr.is_none() {
        bail!("selected DNS scenarios require --dns-udp-addr for the host lane");
    }
    let guest_dns_udp_addr = args.guest_dns_udp_addr.clone().or_else(|| {
        std::env::var("CAPSEM_GUEST_DNS_UDP_ADDR")
            .ok()
            .filter(|value| !value.is_empty())
    });
    if needs_dns && guest_dns_udp_addr.is_none() {
        bail!("selected DNS scenarios require --guest-dns-udp-addr; do not fake DNS lane parity");
    }
    let host_artifact = run_protocol(ProtocolArgs {
        base_url: Some(args.base_url.trim_end_matches('/').to_string()),
        dns_udp_addr: host_dns_udp_addr,
        requests: args.requests,
        concurrency: args.concurrency,
        timeout_ms: args.timeout_ms,
        scenarios: Some(selected_names.clone()),
        lane: "host_direct".to_string(),
        json_out: temp_artifact_path("host"),
    })
    .await?;

    let guest_base_url = args
        .guest_base_url
        .unwrap_or_else(|| args.base_url.trim_end_matches('/').to_string());
    let guest_command = guest_protocol_command(
        &guest_base_url,
        guest_dns_udp_addr.as_deref(),
        args.requests,
        args.concurrency,
        args.timeout_ms,
        &selected_names,
    );
    let guest_stdout = run_capsem_guest_command(
        &args.capsem_bin,
        args.session.as_deref(),
        &guest_command,
        args.guest_timeout_secs,
    )?;
    let guest_artifact = parse_guest_protocol_artifact(&guest_stdout)
        .context("parse guest capsem-bench protocol JSON")?;
    let delta = build_delta_report(
        "host_direct:inline".to_string(),
        "guest_capsem:inline".to_string(),
        &host_artifact,
        &guest_artifact,
    )?;
    let artifact = ProtocolDeltaArtifact {
        version: VERSION.to_string(),
        timestamp: timestamp(),
        benchmark: "capsem-bench-rs-protocol-delta".to_string(),
        host: host_artifact,
        guest: guest_artifact,
        abstraction_delta: delta,
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

async fn run_dns_scenario(
    dns_udp_addr: SocketAddr,
    scenario: Scenario,
    qtype: u16,
    total_requests: usize,
    concurrency: usize,
    timeout: Duration,
) -> Result<ScenarioResult> {
    let workers = concurrency.min(total_requests);
    let per_worker = total_requests / workers;
    let remainder = total_requests % workers;
    let started = Instant::now();
    let tasks = (0..workers).map(|idx| {
        let count = per_worker + usize::from(idx < remainder);
        tokio::spawn(async move {
            let socket = match UdpSocket::bind("0.0.0.0:0").await {
                Ok(socket) => socket,
                Err(error) => {
                    return vec![RequestSample {
                        status: 0,
                        size: 0,
                        latency_ms: 0.0,
                        attempts: 1,
                        error: Some(format!("dns bind: {error}")),
                        required_text_present: false,
                        secret_shaped_fixture_seen: false,
                    }];
                }
            };
            let mut out = Vec::with_capacity(count);
            for request_idx in 0..count {
                let query_id = ((idx as u32 * 4099 + request_idx as u32) & 0xFFFF) as u16;
                out.push(
                    run_one_dns_query(
                        &socket,
                        dns_udp_addr,
                        scenario.path,
                        qtype,
                        query_id,
                        timeout,
                    )
                    .await,
                );
            }
            out
        })
    });
    let joined = try_join_all(tasks)
        .await
        .context("join DNS benchmark workers")?;
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

async fn run_one_dns_query(
    socket: &UdpSocket,
    dns_udp_addr: SocketAddr,
    qname: &str,
    qtype: u16,
    query_id: u16,
    timeout: Duration,
) -> RequestSample {
    let query = match build_dns_query(qname, qtype, query_id) {
        Ok(query) => query,
        Err(error) => {
            return RequestSample {
                status: 0,
                size: 0,
                latency_ms: 0.0,
                attempts: 1,
                error: Some(format!("dns query: {error}")),
                required_text_present: false,
                secret_shaped_fixture_seen: false,
            };
        }
    };
    let started = Instant::now();
    if let Err(error) = socket.send_to(&query, dns_udp_addr).await {
        return RequestSample {
            status: 0,
            size: 0,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            attempts: 1,
            error: Some(format!("dns send: {error}")),
            required_text_present: false,
            secret_shaped_fixture_seen: false,
        };
    }
    let mut buf = [0_u8; 1500];
    match tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
        Ok(Ok((len, _peer))) => {
            let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
            let response = &buf[..len];
            let id_matches = response.len() >= 2 && response[..2] == query_id.to_be_bytes();
            let rcode = parse_dns_rcode(response).unwrap_or(2);
            RequestSample {
                status: rcode,
                size: len,
                latency_ms,
                attempts: 1,
                error: (!id_matches).then(|| "dns id mismatch".to_string()),
                required_text_present: true,
                secret_shaped_fixture_seen: false,
            }
        }
        Ok(Err(error)) => RequestSample {
            status: 0,
            size: 0,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            attempts: 1,
            error: Some(format!("dns recv: {error}")),
            required_text_present: false,
            secret_shaped_fixture_seen: false,
        },
        Err(_) => RequestSample {
            status: 0,
            size: 0,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            attempts: 1,
            error: Some("dns timeout".to_string()),
            required_text_present: false,
            secret_shaped_fixture_seen: false,
        },
    }
}

async fn run_one_request(
    client: &Client,
    url: &str,
    scenario: Scenario,
    timeout: Duration,
) -> RequestSample {
    let started = Instant::now();
    let mut last_request_error = None;
    for attempt in 1..=HTTP_REQUEST_ATTEMPTS {
        let request = match scenario.method {
            HttpMethod::Get => client.request(Method::GET, url),
            HttpMethod::PostJson => {
                let body = scenario.request_body.unwrap_or("{}");
                client
                    .request(Method::POST, url)
                    .header("content-type", "application/json")
                    .body(body.to_string())
            }
        };
        match tokio::time::timeout(timeout, request.send()).await {
            Ok(Ok(response)) => {
                let status = response.status().as_u16();
                return match response.bytes().await {
                    Ok(body) => {
                        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                        RequestSample {
                            status,
                            size: body.len(),
                            latency_ms,
                            attempts: attempt,
                            error: None,
                            required_text_present: required_text_present(&body, scenario),
                            secret_shaped_fixture_seen: secret_fixture_seen(&body, scenario),
                        }
                    }
                    Err(error) => RequestSample {
                        status,
                        size: 0,
                        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                        attempts: attempt,
                        error: Some(format!("body: {error}")),
                        required_text_present: false,
                        secret_shaped_fixture_seen: false,
                    },
                };
            }
            Ok(Err(error)) => {
                last_request_error = Some(format!("request: {error}"));
                if attempt < HTTP_REQUEST_ATTEMPTS {
                    tokio::task::yield_now().await;
                    continue;
                }
            }
            Err(_) => {
                return RequestSample {
                    status: 0,
                    size: 0,
                    latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                    attempts: attempt,
                    error: Some("timeout".to_string()),
                    required_text_present: false,
                    secret_shaped_fixture_seen: false,
                };
            }
        }
    }
    RequestSample {
        status: 0,
        size: 0,
        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
        attempts: HTTP_REQUEST_ATTEMPTS,
        error: Some(last_request_error.unwrap_or_else(|| "request failed".to_string())),
        required_text_present: false,
        secret_shaped_fixture_seen: false,
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
        transport_retries: samples
            .iter()
            .map(|sample| sample.attempts.saturating_sub(1))
            .sum(),
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
    let delta = build_delta_report(
        args.host.display().to_string(),
        args.guest.display().to_string(),
        &host,
        &guest,
    )?;
    let artifact = DeltaArtifact {
        version: VERSION.to_string(),
        timestamp: timestamp(),
        benchmark: "capsem-bench-rs-delta".to_string(),
        abstraction_delta: delta,
    };
    write_json(&args.json_out, &artifact)?;
    Ok(artifact)
}

fn build_delta_report(
    host_artifact: String,
    guest_artifact: String,
    host: &Artifact,
    guest: &Artifact,
) -> Result<DeltaReport> {
    validate_successful_scenarios(
        &host.mock_server_protocol.scenarios,
        &host.mock_server_protocol.lane,
    )?;
    validate_successful_scenarios(
        &guest.mock_server_protocol.scenarios,
        &guest.mock_server_protocol.lane,
    )?;
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
    Ok(DeltaReport {
        host_artifact,
        guest_artifact,
        host_lane: host.mock_server_protocol.lane.clone(),
        guest_lane: guest.mock_server_protocol.lane.clone(),
        scenarios,
    })
}

fn validate_successful_scenarios(rows: &[ScenarioResult], lane: &str) -> Result<()> {
    let failures = rows
        .iter()
        .filter(|row| row.failed > 0)
        .map(|row| {
            format!(
                "{} failed={}/{} errors={:?}",
                row.name, row.failed, row.total_requests, row.errors
            )
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        bail!(
            "benchmark lane {lane} has failed scenario requests; refusing to report poisoned numbers: {}",
            failures.join("; ")
        );
    }
    Ok(())
}

fn guest_protocol_command(
    base_url: &str,
    dns_udp_addr: Option<&str>,
    requests: usize,
    concurrency: usize,
    timeout_ms: u64,
    scenarios: &str,
) -> String {
    let mut parts = vec![
        "capsem-bench-rs".to_string(),
        "protocol".to_string(),
        "--base-url".to_string(),
        shell_quote(base_url),
        "--requests".to_string(),
        requests.to_string(),
        "--concurrency".to_string(),
        concurrency.to_string(),
        "--timeout-ms".to_string(),
        timeout_ms.to_string(),
        "--scenarios".to_string(),
        shell_quote(scenarios),
    ];
    if let Some(dns_udp_addr) = dns_udp_addr {
        parts.push("--dns-udp-addr".to_string());
        parts.push(shell_quote(dns_udp_addr));
    }
    parts.push("--json-out".to_string());
    parts.push("/tmp/capsem-benchmark.json".to_string());
    parts.extend([
        "&&".to_string(),
        "cat".to_string(),
        "/tmp/capsem-benchmark.json".to_string(),
    ]);
    parts.join(" ")
}

fn parse_guest_protocol_artifact(stdout: &str) -> Result<Artifact> {
    if let Ok(artifact) = serde_json::from_str::<Artifact>(stdout.trim()) {
        if artifact.benchmark == "capsem-bench-rs" {
            return Ok(artifact);
        }
        bail!(
            "guest protocol benchmark must be produced by capsem-bench-rs, got {}",
            artifact.benchmark
        );
    }
    let value = extract_first_json_value(stdout)
        .or_else(|| serde_json::from_str(stdout.trim()).ok())
        .with_context(|| {
            let preview = stdout.chars().take(600).collect::<String>();
            format!("parse guest capsem-bench-rs JSON from stdout preview: {preview:?}")
        })?;
    let artifact: Artifact =
        serde_json::from_value(value).context("parse guest capsem-bench-rs artifact")?;
    if artifact.benchmark != "capsem-bench-rs" {
        bail!(
            "guest protocol benchmark must be produced by capsem-bench-rs, got {}",
            artifact.benchmark
        );
    }
    Ok(artifact)
}

fn extract_first_json_value(output: &str) -> Option<serde_json::Value> {
    for (start, _) in output.match_indices('{') {
        let mut deserializer = serde_json::Deserializer::from_str(&output[start..]);
        if let Ok(value) = serde_json::Value::deserialize(&mut deserializer) {
            return Some(value);
        }
    }
    None
}

fn run_capsem_guest_command(
    capsem_bin: &Path,
    session: Option<&str>,
    command: &str,
    timeout_secs: u64,
) -> Result<String> {
    let mut cmd = StdCommand::new(capsem_bin);
    match session {
        Some(session) => {
            cmd.arg("exec")
                .arg(session)
                .arg(command)
                .arg("--timeout")
                .arg(timeout_secs.to_string());
        }
        None => {
            cmd.arg("run")
                .arg(command)
                .arg("--timeout")
                .arg(timeout_secs.to_string());
        }
    }
    let output = cmd
        .output()
        .with_context(|| format!("run guest benchmark via {}", capsem_bin.display()))?;
    if !output.status.success() {
        bail!(
            "guest capsem-bench failed via {}: status={}\nstdout:\n{}\nstderr:\n{}",
            capsem_bin.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("guest capsem-bench stdout was not UTF-8")
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

fn build_dns_query(qname: &str, qtype: u16, query_id: u16) -> Result<Vec<u8>> {
    let mut query = Vec::with_capacity(512);
    query.extend_from_slice(&query_id.to_be_bytes());
    query.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
    for label in qname.split('.').filter(|label| !label.is_empty()) {
        if label.len() > 63 {
            bail!("DNS label too long in {qname:?}: {label:?}");
        }
        query.push(u8::try_from(label.len()).expect("label length checked"));
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, (qtype >> 8) as u8, qtype as u8, 0, 1]);
    Ok(query)
}

fn parse_dns_rcode(response: &[u8]) -> Option<u16> {
    (response.len() >= 4).then(|| u16::from(response[3] & 0x0F))
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

fn temp_artifact_path(lane: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "capsem-bench-{lane}-{}-{}.json",
        std::process::id(),
        timestamp().to_bits()
    ))
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"@%_+=:,./-".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
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
                attempts: 1,
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
                attempts: 1,
                error: None,
                required_text_present: true,
                secret_shaped_fixture_seen: false,
            },
            scenario
        ));
    }

    #[test]
    fn dns_query_builder_and_rcode_parser_are_strict() {
        let query = build_dns_query("load-test.capsem-bogus", 1, 0xCAFE).unwrap();
        assert_eq!(&query[..2], b"\xCA\xFE");
        assert!(query
            .windows("capsem-bogus".len())
            .any(|w| w == b"capsem-bogus"));
        let mut response = vec![0xCA, 0xFE, 0x81, 0x83];
        response.extend_from_slice(&query[4..]);
        assert_eq!(parse_dns_rcode(&response), Some(3));
        assert!(build_dns_query(&format!("{}.test", "x".repeat(64)), 1, 1).is_err());
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
            transport_retries: 0,
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
        assert_eq!(guest_row.failed as isize - host_row.failed as isize, 0);
    }

    #[test]
    fn failed_scenarios_are_poisoned_benchmark_numbers() {
        let failed = ScenarioResult {
            name: "tiny_http".to_string(),
            path: "/tiny".to_string(),
            body_kind: "tiny".to_string(),
            total_requests: 100,
            concurrency: 10,
            successful: 0,
            failed: 100,
            transport_retries: 0,
            total_duration_ms: 10.0,
            requests_per_sec: 10_000.0,
            transfer_bytes: 0,
            bytes_per_sec: 0.0,
            latency_ms: LatencySummary {
                min: 0.1,
                max: 2.0,
                mean: 0.5,
                p50: 0.4,
                p95: 1.0,
                p99: 1.5,
            },
            errors: BTreeMap::from([("request:connection refused".to_string(), 100)]),
            secret_shaped_fixture_seen: None,
            raw_secret_stored_in_result: None,
        };
        let err = validate_successful_scenarios(&[failed], "host_direct").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("poisoned numbers"), "{message}");
        assert!(message.contains("tiny_http failed=100/100"), "{message}");
    }

    #[test]
    fn guest_protocol_command_uses_one_capsem_bench_invocation() {
        let command = guest_protocol_command(
            "http://127.0.0.1:3713",
            Some("127.0.0.1:3713"),
            50_000,
            64,
            30_000,
            "tiny_http,mcp_tool_call,dns_local_nxdomain",
        );
        assert!(
            command.starts_with("capsem-bench-rs protocol "),
            "{command}"
        );
        assert!(command.contains("--base-url http://127.0.0.1:3713"));
        assert!(command.contains("--dns-udp-addr 127.0.0.1:3713"));
        assert!(command.contains("--requests 50000"));
        assert!(command.contains("--concurrency 64"));
        assert!(command.contains("--timeout-ms 30000"));
        assert!(command.contains("--scenarios tiny_http,mcp_tool_call,dns_local_nxdomain"));
        assert!(command
            .ends_with("--json-out /tmp/capsem-benchmark.json && cat /tmp/capsem-benchmark.json"));
    }

    #[test]
    fn parse_guest_protocol_artifact_rejects_legacy_guest_wrapper_json() {
        let stdout = r#"mock-server-protocol base_url=http://127.0.0.1:3713 requests=100 concurrency=10
JSON results saved to /tmp/capsem-benchmark.json
{
          "version": "0.3.0",
          "timestamp": 1782339183.0,
          "hostname": "capsem",
          "mock_server_protocol": {
            "version": "1.0",
            "base_url": "http://127.0.0.1:3713",
            "total_requests": 100,
            "concurrency": 10,
            "timeout_s": 30.0,
            "selected_scenarios": ["tiny_http"],
            "scenarios": [{
              "name": "tiny_http",
              "path": "/tiny",
              "body_kind": "tiny",
              "method": "GET",
              "expected_status": 200,
              "total_requests": 100,
              "concurrency": 10,
              "successful": 100,
              "failed": 0,
              "total_duration_ms": 100.0,
              "requests_per_sec": 1000.0,
              "transfer_bytes": 2100,
              "bytes_per_sec": 21000.0,
              "latency_ms": {
                "min": 0.1,
                "max": 0.5,
                "mean": 0.2,
                "p50": 0.2,
                "p95": 0.4,
                "p99": 0.5
              },
              "errors": {},
              "secret_shaped_fixture_seen": null,
              "raw_secret_stored_in_result": null
            }],
            "websocket": []
          }
        }"#;
        let err = parse_guest_protocol_artifact(stdout).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("parse guest capsem-bench-rs artifact")
                || message.contains("must be produced by capsem-bench-rs"),
            "{message}"
        );
    }

    #[test]
    fn shell_quote_preserves_single_argument_boundaries() {
        assert_eq!(
            shell_quote("http://127.0.0.1:3713"),
            "http://127.0.0.1:3713"
        );
        assert_eq!(
            shell_quote("tiny_http,mcp_tool_call"),
            "tiny_http,mcp_tool_call"
        );
        assert_eq!(shell_quote("weird value"), "'weird value'");
        assert_eq!(shell_quote("can't"), "'can'\"'\"'t'");
    }

    #[test]
    fn build_delta_report_keeps_inline_artifact_identity() {
        let row = ScenarioResult {
            name: "tiny_http".to_string(),
            path: "/tiny".to_string(),
            body_kind: "tiny".to_string(),
            total_requests: 100,
            concurrency: 10,
            successful: 100,
            failed: 0,
            transport_retries: 0,
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
        let host = Artifact {
            version: VERSION.to_string(),
            timestamp: 1.0,
            hostname: "host".to_string(),
            benchmark: "capsem-bench-rs".to_string(),
            mock_server_protocol: ProtocolReport {
                version: "1.1-rust".to_string(),
                lane: "host_direct".to_string(),
                base_url: "http://127.0.0.1:3713".to_string(),
                dns_udp_addr: None,
                total_requests: 100,
                concurrency: 10,
                timeout_ms: 30_000,
                selected_scenarios: vec!["tiny_http".to_string()],
                scenarios: vec![row.clone()],
            },
        };
        let mut guest = Artifact {
            mock_server_protocol: ProtocolReport {
                lane: "guest_capsem".to_string(),
                scenarios: vec![ScenarioResult {
                    requests_per_sec: 5_000.0,
                    latency_ms: LatencySummary {
                        p50: 1.4,
                        p95: 3.0,
                        p99: 4.5,
                        ..row.latency_ms.clone()
                    },
                    ..row
                }],
                ..host.mock_server_protocol.clone()
            },
            ..host.clone()
        };
        guest.hostname = "guest".to_string();
        let delta = build_delta_report(
            "host:inline".to_string(),
            "guest:inline".to_string(),
            &host,
            &guest,
        )
        .unwrap();
        assert_eq!(delta.host_artifact, "host:inline");
        assert_eq!(delta.guest_artifact, "guest:inline");
        assert_eq!(delta.host_lane, "host_direct");
        assert_eq!(delta.guest_lane, "guest_capsem");
    }
}
