//! What the protocol lane measures, and what it writes down.
//!
//! The scenario table and the artifact shapes, separated from the code that
//! drives them. `main.rs` was 1496 lines -- the crate's CLI, its scenario
//! catalogue, its HTTP and DNS engines and every arithmetic helper in one
//! file, which is the shape `[boundary.rust]` exists to catch.

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use std::collections::BTreeMap;

use crate::SECRET_SHAPED_MARKER;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Scenario {
    pub(crate) name: &'static str,
    pub(crate) transport: ScenarioTransport,
    pub(crate) path: &'static str,
    pub(crate) method: HttpMethod,
    pub(crate) request_body: Option<&'static str>,
    pub(crate) expected_status: u16,
    pub(crate) expected_bytes: Option<usize>,
    pub(crate) body_kind: &'static str,
    pub(crate) required_text: Option<&'static str>,
    pub(crate) secret_shaped_fixture: bool,
}

#[derive(Clone)]
pub(crate) struct HttpClients {
    pub(crate) primary: Client,
    pub(crate) retry: Client,
}

impl HttpClients {
    pub(crate) fn build(concurrency: usize, timeout: Duration) -> Result<Self> {
        let primary = Client::builder()
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(concurrency)
            .timeout(timeout)
            .build()
            .context("build pooled HTTP benchmark client")?;
        let retry = Client::builder()
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(0)
            .timeout(timeout)
            .build()
            .context("build isolated HTTP benchmark retry client")?;
        Ok(Self { primary, retry })
    }

    pub(crate) fn for_attempt(&self, attempt: usize) -> &Client {
        if attempt == 1 {
            &self.primary
        } else {
            &self.retry
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenarioTransport {
    Http,
    DnsUdp { qtype: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpMethod {
    Get,
    PostJson,
}

pub(crate) const SCENARIOS: &[Scenario] = &[
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
pub(crate) struct Artifact {
    pub(crate) version: String,
    pub(crate) timestamp: f64,
    pub(crate) hostname: String,
    pub(crate) benchmark: String,
    pub(crate) mock_server_protocol: ProtocolReport,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ProtocolReport {
    pub(crate) version: String,
    pub(crate) lane: String,
    pub(crate) base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dns_udp_addr: Option<String>,
    pub(crate) total_requests: usize,
    pub(crate) concurrency: usize,
    pub(crate) timeout_ms: u64,
    pub(crate) selected_scenarios: Vec<String>,
    pub(crate) scenarios: Vec<ScenarioResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ScenarioResult {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) body_kind: String,
    pub(crate) total_requests: usize,
    pub(crate) concurrency: usize,
    pub(crate) successful: usize,
    pub(crate) failed: usize,
    #[serde(default)]
    pub(crate) transport_retries: usize,
    pub(crate) total_duration_ms: f64,
    pub(crate) requests_per_sec: f64,
    pub(crate) transfer_bytes: u64,
    pub(crate) bytes_per_sec: f64,
    pub(crate) latency_ms: LatencySummary,
    /// Raw per-request latencies, kept for `capsem.bench.v1` and never
    /// serialized into this artifact: `capsem-bench` computes every statistic
    /// itself, so a collector that pre-summarizes hides the distribution.
    #[serde(skip)]
    pub(crate) latency_samples: Vec<f64>,
    pub(crate) errors: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) secret_shaped_fixture_seen: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) raw_secret_stored_in_result: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct LatencySummary {
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) mean: f64,
    pub(crate) p50: f64,
    pub(crate) p95: f64,
    pub(crate) p99: f64,
}

#[derive(Debug)]
pub(crate) struct RequestSample {
    pub(crate) status: u16,
    pub(crate) size: usize,
    pub(crate) latency_ms: f64,
    pub(crate) attempts: usize,
    pub(crate) error: Option<String>,
    pub(crate) required_text_present: bool,
    pub(crate) secret_shaped_fixture_seen: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeltaArtifact {
    pub(crate) version: String,
    pub(crate) timestamp: f64,
    pub(crate) benchmark: String,
    pub(crate) abstraction_delta: DeltaReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProtocolDeltaArtifact {
    pub(crate) version: String,
    pub(crate) timestamp: f64,
    pub(crate) benchmark: String,
    pub(crate) host: Artifact,
    pub(crate) guest: Artifact,
    pub(crate) abstraction_delta: DeltaReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeltaReport {
    pub(crate) host_artifact: String,
    pub(crate) guest_artifact: String,
    pub(crate) host_lane: String,
    pub(crate) guest_lane: String,
    pub(crate) scenarios: Vec<ScenarioDelta>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ScenarioDelta {
    pub(crate) name: String,
    pub(crate) host_requests_per_sec: f64,
    pub(crate) guest_requests_per_sec: f64,
    pub(crate) rps_ratio_guest_over_host: f64,
    pub(crate) host_bytes_per_sec: f64,
    pub(crate) guest_bytes_per_sec: f64,
    pub(crate) throughput_ratio_guest_over_host: f64,
    pub(crate) p50_delta_ms: f64,
    pub(crate) p95_delta_ms: f64,
    pub(crate) p99_delta_ms: f64,
    pub(crate) error_delta: isize,
}

pub(crate) fn select_scenarios(selected: Option<&str>) -> Result<Vec<Scenario>> {
    let by_name = SCENARIOS
        .iter()
        .map(|scenario| (scenario.name, *scenario))
        .collect::<BTreeMap<_, _>>();
    let Some(selected) = selected else {
        return Ok(SCENARIOS.to_vec());
    };
    let mut out = Vec::new();
    for name in selected.split(',').map(str::trim).filter(|name| !name.is_empty()) {
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

pub(crate) fn build_dns_query(qname: &str, qtype: u16, query_id: u16) -> Result<Vec<u8>> {
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

pub(crate) fn parse_dns_rcode(response: &[u8]) -> Option<u16> {
    (response.len() >= 4).then(|| u16::from(response[3] & 0x0F))
}

pub(crate) fn result_ok(sample: &RequestSample, scenario: Scenario) -> bool {
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

pub(crate) fn required_text_present(body: &[u8], scenario: Scenario) -> bool {
    scenario
        .required_text
        .map(|needle| body.windows(needle.len()).any(|window| window == needle.as_bytes()))
        .unwrap_or(true)
}

pub(crate) fn secret_fixture_seen(body: &[u8], scenario: Scenario) -> bool {
    scenario.secret_shaped_fixture
        && body
            .windows(SECRET_SHAPED_MARKER.len())
            .any(|window| window == SECRET_SHAPED_MARKER.as_bytes())
}

pub(crate) fn latency_summary(mut values: Vec<f64>) -> LatencySummary {
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

pub(crate) fn percentile(sorted: &[f64], percentile: f64) -> f64 {
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

pub(crate) fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub(crate) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
