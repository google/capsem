use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{
    AppState, Attention, ServiceState, ServiceStatus, SessionLifecycle, SessionStats,
    SessionSummary,
};
use crate::provider::StateProvider;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayProvider {
    base_url: String,
}

impl GatewayProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn default_base_url() -> String {
        if let Ok(url) = std::env::var("CAPSEM_GATEWAY_URL") {
            return url.trim_end_matches('/').to_string();
        }
        let port = gateway_port().unwrap_or(19222);
        format!("http://127.0.0.1:{port}")
    }

    pub async fn load_async(&self) -> Result<AppState> {
        let started = Instant::now();
        let client = reqwest::Client::new();
        let token = fetch_token(&client, &self.base_url).await?;
        let status = fetch_status(&client, &self.base_url, &token).await?;
        Ok(status_response_to_state(status, started.elapsed()))
    }
}

impl StateProvider for GatewayProvider {
    fn load(&self) -> Result<AppState> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build capsem-tui gateway provider runtime")?;
        runtime.block_on(self.load_async())
    }
}

async fn fetch_token(client: &reqwest::Client, base_url: &str) -> Result<String> {
    let response = client
        .get(format!("{base_url}/token"))
        .send()
        .await
        .context("fetch capsem gateway token")?
        .error_for_status()
        .context("capsem gateway token request failed")?;
    let token: TokenResponse = response
        .json()
        .await
        .context("parse capsem gateway token response")?;
    Ok(token.token)
}

async fn fetch_status(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<StatusResponse> {
    client
        .get(format!("{base_url}/status"))
        .bearer_auth(token)
        .send()
        .await
        .context("fetch capsem gateway status")?
        .error_for_status()
        .context("capsem gateway status request failed")?
        .json()
        .await
        .context("parse capsem gateway status response")
}

fn gateway_port() -> Option<u16> {
    let path = run_dir().join("gateway.port");
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn run_dir() -> PathBuf {
    if let Ok(run_dir) = std::env::var("CAPSEM_RUN_DIR") {
        return PathBuf::from(run_dir);
    }
    if let Ok(home) = std::env::var("CAPSEM_HOME") {
        return PathBuf::from(home).join("run");
    }
    std::env::var("HOME")
        .map(|home| PathBuf::from(home).join(".capsem/run"))
        .unwrap_or_else(|_| PathBuf::from(".capsem/run"))
}

fn status_response_to_state(status: StatusResponse, latency: Duration) -> AppState {
    let service_status = service_status_from_gateway(&status.service);
    let sessions = status
        .vms
        .into_iter()
        .map(vm_response_to_summary)
        .collect::<Vec<_>>();
    let active_session_id = sessions
        .first()
        .map(|session| session.id.clone())
        .unwrap_or_default();
    AppState {
        service: ServiceState {
            status: service_status,
            latency,
            last_event_age: Duration::ZERO,
            reconnect_attempt: None,
        },
        active_session_id,
        sessions,
    }
}

fn vm_response_to_summary(vm: VmSummary) -> SessionSummary {
    let lifecycle = lifecycle_from_status(&vm.status);
    let mut attention = attention_from_vm(&vm, lifecycle);
    attention.dedup();
    let id = vm.id;
    let title = vm.name.unwrap_or_else(|| id.clone());
    let tokens = vm
        .total_input_tokens
        .unwrap_or_default()
        .saturating_add(vm.total_output_tokens.unwrap_or_default());
    SessionSummary {
        id,
        title,
        repo_path: None,
        profile: vm.profile_id.unwrap_or_else(|| "default".to_string()),
        branch: vm.profile_revision,
        lifecycle,
        attention,
        stats: SessionStats {
            duration: Duration::from_secs(vm.uptime_secs.unwrap_or_default()),
            jobs: vm.total_tool_calls.unwrap_or_default().min(u16::MAX as u64) as u16,
            events: vm
                .total_requests
                .unwrap_or_default()
                .saturating_add(vm.total_file_events.unwrap_or_default())
                .min(u32::MAX as u64) as u32,
            tokens,
            cost_micros: cost_to_micros(vm.total_estimated_cost),
        },
    }
}

fn service_status_from_gateway(service: &str) -> ServiceStatus {
    match service.to_ascii_lowercase().as_str() {
        "running" => ServiceStatus::Online,
        "unavailable" => ServiceStatus::Degraded,
        "failed" => ServiceStatus::Failed,
        _ => ServiceStatus::Stale,
    }
}

fn lifecycle_from_status(status: &str) -> SessionLifecycle {
    match status.to_ascii_lowercase().as_str() {
        "running" => SessionLifecycle::Working,
        "suspended" => SessionLifecycle::Suspended,
        "defunct" | "failed" => SessionLifecycle::Failed,
        "stopped" => SessionLifecycle::Idle,
        _ => SessionLifecycle::Idle,
    }
}

fn attention_from_vm(vm: &VmSummary, lifecycle: SessionLifecycle) -> Vec<Attention> {
    let mut attention = Vec::new();
    if matches!(lifecycle, SessionLifecycle::Failed) {
        attention.push(Attention::StaleData);
    }
    if vm.denied_requests.unwrap_or_default() > 0 {
        attention.push(Attention::PolicyDeny);
    }
    if vm.profile_status.as_deref().is_some_and(|status| {
        !matches!(
            status.to_ascii_lowercase().as_str(),
            "ready" | "ok" | "installed" | "active"
        )
    }) {
        attention.push(Attention::StaleData);
    }
    attention
}

fn cost_to_micros(cost: Option<f64>) -> u64 {
    let Some(cost) = cost else {
        return 0;
    };
    if !cost.is_finite() || cost <= 0.0 {
        return 0;
    }
    (cost * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    service: String,
    vms: Vec<VmSummary>,
}

#[derive(Debug, Deserialize)]
struct VmSummary {
    id: String,
    #[serde(default)]
    name: Option<String>,
    status: String,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    profile_revision: Option<String>,
    #[serde(default)]
    profile_status: Option<String>,
    #[serde(default)]
    uptime_secs: Option<u64>,
    #[serde(default)]
    total_input_tokens: Option<u64>,
    #[serde(default)]
    total_output_tokens: Option<u64>,
    #[serde(default)]
    total_estimated_cost: Option<f64>,
    #[serde(default)]
    total_tool_calls: Option<u64>,
    #[serde(default)]
    total_requests: Option<u64>,
    #[serde(default)]
    denied_requests: Option<u64>,
    #[serde(default)]
    total_file_events: Option<u64>,
}

#[cfg(test)]
pub(crate) fn state_from_status_json_for_test(raw: &str, latency: Duration) -> Result<AppState> {
    let response: StatusResponse = serde_json::from_str(raw)?;
    Ok(status_response_to_state(response, latency))
}
