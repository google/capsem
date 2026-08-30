//! Driving the protocol lanes: HTTP, DNS, and the guest/host delta.
//!
//! Split out of `main.rs` for the reason `[boundary.rust]` states: a file
//! nothing can take in whole stops being reviewable, and the ceiling is what
//! notices before that happens rather than after.

use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{
    DeltaArgs, ProtocolArgs, ProtocolDeltaArgs, HTTP_REQUEST_ATTEMPTS,
    HTTP_RETRY_BACKOFF_BASE_MS, VERSION,
};
use tokio::net::UdpSocket;
use futures::future::try_join_all;

use crate::scenarios::*;

pub(crate) async fn run_protocol(args: ProtocolArgs) -> Result<Artifact> {
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
    let clients = HttpClients::build(args.concurrency, Duration::from_millis(args.timeout_ms))?;

    let mut scenario_results = Vec::with_capacity(selected.len());
    for scenario in &selected {
        let result = match scenario.transport {
            ScenarioTransport::Http => {
                run_http_scenario(
                    clients.clone(),
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

pub(crate) async fn run_protocol_delta(args: ProtocolDeltaArgs) -> Result<ProtocolDeltaArtifact> {
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
        // The delta lane compares two artifacts; recording is the caller's
        // decision, made once for the pair rather than twice inside it.
        record: None,
        channel: "unknown".to_string(),
        commit: "unknown".to_string(),
        profile: "code".to_string(),
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

pub(crate) async fn run_http_scenario(
    clients: HttpClients,
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
        let clients = clients.clone();
        let url = url.clone();
        let count = per_worker + usize::from(idx < remainder);
        tokio::spawn(async move {
            let mut out = Vec::with_capacity(count);
            for _ in 0..count {
                out.push(run_one_request(&clients, &url, scenario, timeout).await);
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

pub(crate) async fn run_dns_scenario(
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

pub(crate) async fn run_one_dns_query(
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

pub(crate) async fn run_one_request(
    clients: &HttpClients,
    url: &str,
    scenario: Scenario,
    timeout: Duration,
) -> RequestSample {
    let started = Instant::now();
    let mut last_request_error = None;
    for attempt in 1..=HTTP_REQUEST_ATTEMPTS {
        let client = clients.for_attempt(attempt);
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
                    Err(error) => {
                        last_request_error = Some(format!("body: {error}"));
                        if attempt < HTTP_REQUEST_ATTEMPTS {
                            sleep_before_http_retry(attempt).await;
                            continue;
                        }
                        RequestSample {
                            status,
                            size: 0,
                            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                            attempts: attempt,
                            error: last_request_error,
                            required_text_present: false,
                            secret_shaped_fixture_seen: false,
                        }
                    }
                };
            }
            Ok(Err(error)) => {
                last_request_error = Some(format!("request: {error}"));
                if attempt < HTTP_REQUEST_ATTEMPTS {
                    sleep_before_http_retry(attempt).await;
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

pub(crate) async fn sleep_before_http_retry(attempt: usize) {
    let delay_ms = HTTP_RETRY_BACKOFF_BASE_MS.saturating_mul(1_u64 << attempt.saturating_sub(1));
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
}

pub(crate) fn summarize(
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
        latency_samples: samples.iter().map(|sample| sample.latency_ms).collect(),
        errors,
        secret_shaped_fixture_seen: secret_seen,
        raw_secret_stored_in_result: scenario.secret_shaped_fixture.then_some(false),
    }
}

pub(crate) fn run_delta(args: DeltaArgs) -> Result<DeltaArtifact> {
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

pub(crate) fn build_delta_report(
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

pub(crate) fn validate_successful_scenarios(rows: &[ScenarioResult], lane: &str) -> Result<()> {
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

pub(crate) fn guest_protocol_command(
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

pub(crate) fn parse_guest_protocol_artifact(stdout: &str) -> Result<Artifact> {
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

pub(crate) fn extract_first_json_value(output: &str) -> Option<serde_json::Value> {
    for (start, _) in output.match_indices('{') {
        let mut deserializer = serde_json::Deserializer::from_str(&output[start..]);
        if let Ok(value) = serde_json::Value::deserialize(&mut deserializer) {
            return Some(value);
        }
    }
    None
}

pub(crate) fn run_capsem_guest_command(
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


pub(crate) fn rows_by_name(rows: &[ScenarioResult]) -> BTreeMap<&str, &ScenarioResult> {
    rows.iter().map(|row| (row.name.as_str(), row)).collect()
}

pub(crate) fn read_artifact(path: &Path) -> Result<Artifact> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", path.display()))
}

pub(crate) fn temp_artifact_path(lane: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "capsem-bench-{lane}-{}-{}.json",
        std::process::id(),
        timestamp().to_bits()
    ))
}

pub(crate) fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"@%_+=:,./-".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

pub(crate) fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        round3(numerator / denominator)
    }
}

pub(crate) fn timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(crate) fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}
