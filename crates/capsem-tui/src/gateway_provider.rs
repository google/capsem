use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_sdk::models::{HypervisorInfo, ServiceAvailability, UpdateStatusResponse, VmLifecycleState, VmSummary};
use capsem_sdk::transport::{CallOptions, Transport};
use serde::Deserialize;

use crate::app::ControlAction;
use crate::model::{
    AppState, Attention, ProfileOption, ServiceState, ServiceStatus, SessionLifecycle, SessionStats, SessionSummary,
    UpdateNotice, UpdateNoticeKind, UpdateTrack,
};
use crate::provider::StateProvider;

#[derive(Clone, Debug)]
pub struct GatewayProvider {
    base_url: String,
    client: reqwest::Client,
    token: Arc<Mutex<Option<String>>>,
}

impl PartialEq for GatewayProvider {
    fn eq(&self, other: &Self) -> bool {
        self.base_url == other.base_url
    }
}

impl Eq for GatewayProvider {}

impl GatewayProvider {
    fn auth_token(&self) -> Result<Option<String>> {
        self.token
            .lock()
            .map(|token| token.clone())
            .map_err(|_| anyhow::anyhow!("capsem gateway token cache poisoned"))
    }

    fn store_auth_token(&self, token: String) -> Result<String> {
        {
            let mut cached = self
                .token
                .lock()
                .map_err(|_| anyhow::anyhow!("capsem gateway token cache poisoned"))?;
            *cached = Some(token.clone());
        }
        Ok(token)
    }

    fn clear_auth_token(&self) -> Result<()> {
        {
            let mut cached = self
                .token
                .lock()
                .map_err(|_| anyhow::anyhow!("capsem gateway token cache poisoned"))?;
            *cached = None;
        }
        Ok(())
    }

    async fn token(&self) -> Result<String> {
        if let Some(token) = self.auth_token()? {
            return Ok(token);
        }
        let token = fetch_token(&self.client, &self.base_url).await?;
        self.store_auth_token(token)
    }
}

impl GatewayProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            token: Arc::new(Mutex::new(None)),
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
        let mut token = self.token().await?;
        let started = Instant::now();
        let status = match fetch_status(&self.base_url, &token).await {
            Ok(status) => status,
            Err(first_error) => {
                self.clear_auth_token()?;
                token = self.token().await.context(first_error)?;
                fetch_status(&self.base_url, &token).await?
            }
        };
        let mut state = status_response_to_state(status, started.elapsed());
        state.profiles = fetch_profiles(&self.base_url, &token).await.unwrap_or_default();
        Ok(state)
    }

    pub fn invoke(&self, action: &ControlAction) -> Result<ActionOutcome> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build capsem-tui gateway action runtime")?;
        runtime.block_on(self.invoke_async(action))
    }

    pub async fn invoke_async(&self, action: &ControlAction) -> Result<ActionOutcome> {
        if matches!(action, ControlAction::StartService) {
            return start_service().await;
        }
        if matches!(action, ControlAction::Update) {
            return update_with_binary(&capsem_binary()).await;
        }
        let token = self.token().await?;
        invoke_action(&self.client, &self.base_url, &token, action).await
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
    let token: TokenResponse = response.json().await.context("parse capsem gateway token response")?;
    Ok(token.token)
}

async fn fetch_status(base_url: &str, token: &str) -> Result<HypervisorInfo> {
    capsem_sdk::Hypervisor::new(base_url, token)?
        .info()
        .await
        .map_err(crate::sdk_actions::display_error)
}

async fn fetch_profiles(base_url: &str, token: &str) -> Result<Vec<ProfileOption>> {
    let transport = Transport::new(base_url, token, Duration::from_secs(30))?;
    let response = capsem_sdk::operations::list_profiles(&transport, CallOptions::default()).await?;
    Ok(response
        .profiles
        .into_iter()
        .filter(|record| record.availability.shell)
        .map(|record| ProfileOption {
            id: record.id,
            name: record.name,
            description: Some(record.description),
        })
        .collect())
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

fn status_response_to_state(status: HypervisorInfo, latency: Duration) -> AppState {
    let service_status = match status.service {
        ServiceAvailability::Running => ServiceStatus::Online,
        ServiceAvailability::Unavailable => ServiceStatus::Degraded,
    };
    let sessions = status.vms.into_iter().map(vm_response_to_summary).collect::<Vec<_>>();
    let active_session_id = sessions.first().map(|session| session.id.clone()).unwrap_or_default();
    AppState {
        service: ServiceState {
            status: service_status,
            latency,
            last_event_age: Duration::ZERO,
            reconnect_attempt: None,
            control_message: None,
        },
        active_session_id,
        sessions,
        profiles: Vec::new(),
        update_notice: Some(status.updates.map(update_response_to_notice).unwrap_or(UpdateNotice {
            kind: UpdateNoticeKind::Unavailable,
            channel_url: None,
        })),
    }
}

fn update_response_to_notice(status: UpdateStatusResponse) -> UpdateNotice {
    let mut tracks = Vec::new();
    if status.binary.update_available {
        tracks.push(UpdateTrack::Binary);
    }
    if status.assets.update_available {
        tracks.push(UpdateTrack::VmAssets);
    }
    if status.profiles.update_available {
        tracks.push(UpdateTrack::Profiles);
    }
    if status.images.update_available {
        tracks.push(UpdateTrack::Images);
    }
    let mut blocked = Vec::new();
    if status.binary.blocked_reason.is_some() {
        blocked.push(UpdateTrack::Binary);
    }
    if status.assets.blocked_reason.is_some() {
        blocked.push(UpdateTrack::VmAssets);
    }
    if status.profiles.blocked_reason.is_some() {
        blocked.push(UpdateTrack::Profiles);
    }
    if status.images.blocked_reason.is_some() {
        blocked.push(UpdateTrack::Images);
    }

    let kind = if !tracks.is_empty() && !blocked.is_empty() {
        UpdateNoticeKind::AvailableWithBlocked {
            available: tracks,
            blocked,
        }
    } else if !tracks.is_empty() {
        UpdateNoticeKind::Available(tracks)
    } else if !blocked.is_empty() {
        UpdateNoticeKind::Blocked(blocked)
    } else if status.last_error.is_some() {
        UpdateNoticeKind::Unavailable
    } else if status.stale {
        UpdateNoticeKind::Stale
    } else {
        UpdateNoticeKind::Current
    };

    UpdateNotice {
        kind,
        channel_url: status.channel_url,
    }
}

fn vm_response_to_summary(vm: VmSummary) -> SessionSummary {
    let lifecycle = lifecycle_from_status(vm.status);
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
        profile: vm.profile_id,
        // The overview does not report a VM's pinned profile revision or readiness.
        profile_status: None,
        can_resume: vm.can_resume,
        resume_blocked_reason: vm.resume_blocked_reason,
        branch: None,
        persistent: vm.persistent,
        lifecycle,
        attention,
        stats: SessionStats {
            duration: Duration::from_secs(vm.uptime_secs.unwrap_or_default()),
            jobs: vm.total_tool_calls.unwrap_or_default().min(u64::from(u16::MAX)) as u16,
            events: vm
                .total_requests
                .unwrap_or_default()
                .saturating_add(vm.total_file_events.unwrap_or_default())
                .min(u64::from(u32::MAX)) as u32,
            tokens,
            cost_micros: cost_to_micros(vm.total_estimated_cost),
        },
    }
}

fn lifecycle_from_status(status: VmLifecycleState) -> SessionLifecycle {
    match status {
        VmLifecycleState::Running => SessionLifecycle::Working,
        VmLifecycleState::Suspended => SessionLifecycle::Suspended,
        VmLifecycleState::Defunct | VmLifecycleState::Incompatible => SessionLifecycle::Failed,
        VmLifecycleState::Stopped => SessionLifecycle::Idle,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionOutcome {
    pub message: String,
    pub focus_session: Option<String>,
}

async fn invoke_action(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    action: &ControlAction,
) -> Result<ActionOutcome> {
    match action {
        ControlAction::StartService => start_service().await,
        ControlAction::Update => update_with_binary(&capsem_binary()).await,
        ControlAction::Purge { all } => {
            let response = client
                .post(join_url(base_url, &["purge"])?)
                .bearer_auth(token)
                .json(&serde_json::json!({ "all": all }))
                .send()
                .await
                .context("purge capsem sessions")?;
            let body = response_json(response).await?;
            let purged = json_u64(&body, "purged");
            let persistent = json_u64(&body, "persistent_purged");
            let ephemeral = json_u64(&body, "ephemeral_purged");
            let message = if *all {
                format!("purged {purged} sessions ({persistent} persistent, {ephemeral} temporary)")
            } else if persistent > 0 {
                format!("purged {purged} sessions ({persistent} broken persistent, {ephemeral} temporary)")
            } else {
                format!("purged {ephemeral} temporary sessions")
            };
            Ok(ActionOutcome {
                message,
                focus_session: None,
            })
        }
        action => crate::sdk_actions::invoke(base_url, token, action)
            .await
            .map_err(crate::sdk_actions::display_error),
    }
}

async fn start_service() -> Result<ActionOutcome> {
    start_service_with_binary(&capsem_binary()).await
}

pub(crate) async fn start_service_with_binary(binary: &Path) -> Result<ActionOutcome> {
    let output = tokio::process::Command::new(binary)
        .arg("start")
        .output()
        .await
        .with_context(|| format!("run {} start", binary.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        anyhow::bail!("capsem start failed: {detail}");
    }
    Ok(ActionOutcome {
        message: "service start requested".to_string(),
        focus_session: None,
    })
}

pub(crate) async fn update_with_binary(binary: &Path) -> Result<ActionOutcome> {
    let mut command = tokio::process::Command::new(binary);
    command.args(["update", "--yes"]);
    let output = command
        .output()
        .await
        .with_context(|| format!("run {} update", binary.display()))?;
    if !output.status.success() {
        let detail = command_detail(&output);
        anyhow::bail!("capsem update failed: {detail}");
    }
    Ok(ActionOutcome {
        message: command_summary(&output).unwrap_or_else(|| "Capsem update finished".to_string()),
        focus_session: None,
    })
}

fn command_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stdout
    }
}

fn command_summary(output: &std::process::Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn capsem_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CAPSEM_TUI_CAPSEM_BINARY") {
        return PathBuf::from(path);
    }
    let installed = home_dir().join(".capsem/bin/capsem");
    if installed.exists() {
        return installed;
    }
    PathBuf::from("capsem")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

async fn response_json(response: reqwest::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let text = response.text().await.context("read gateway action response body")?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("gateway action failed ({status}): {text}"));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(&text).context("parse gateway action response")
}

fn json_u64(body: &serde_json::Value, key: &str) -> u64 {
    body.get(key).and_then(serde_json::Value::as_u64).unwrap_or_default()
}

fn join_url(base_url: &str, path_segments: &[&str]) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!("{}/", base_url.trim_end_matches('/')))
        .context("parse capsem gateway base URL")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("capsem gateway URL cannot be a base"))?
        .extend(path_segments);
    Ok(url)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

#[cfg(test)]
pub(crate) fn state_from_status_json_for_test(raw: &str, latency: Duration) -> Result<AppState> {
    let response: HypervisorInfo = serde_json::from_str(raw)?;
    Ok(status_response_to_state(response, latency))
}

#[cfg(test)]
pub(crate) fn state_from_status_and_update_json_for_test(
    status_raw: &str,
    update_raw: &str,
    latency: Duration,
) -> Result<AppState> {
    let response: HypervisorInfo = serde_json::from_str(status_raw)?;
    let updates: UpdateStatusResponse = serde_json::from_str(update_raw)?;
    let mut state = status_response_to_state(response, latency);
    state.update_notice = Some(update_response_to_notice(updates));
    Ok(state)
}

#[cfg(test)]
mod tests;
