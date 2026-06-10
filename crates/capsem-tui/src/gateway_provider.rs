use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::app::ControlAction;
use crate::model::{
    AppState, Attention, ProfileOption, ServiceState, ServiceStatus, SessionLifecycle,
    SessionStats, SessionSummary,
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
        let mut cached = self
            .token
            .lock()
            .map_err(|_| anyhow::anyhow!("capsem gateway token cache poisoned"))?;
        *cached = Some(token.clone());
        Ok(token)
    }

    fn clear_auth_token(&self) -> Result<()> {
        let mut cached = self
            .token
            .lock()
            .map_err(|_| anyhow::anyhow!("capsem gateway token cache poisoned"))?;
        *cached = None;
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
        let status = match fetch_status(&self.client, &self.base_url, &token).await {
            Ok(status) => status,
            Err(first_error) => {
                self.clear_auth_token()?;
                token = self.token().await.context(first_error)?;
                fetch_status(&self.client, &self.base_url, &token).await?
            }
        };
        let mut state = status_response_to_state(status, started.elapsed());
        state.profiles = self.profile_options(&token, &state).await;
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
        let token = self.token().await?;
        invoke_action(&self.client, &self.base_url, &token, action).await
    }

    async fn profile_options(&self, token: &str, state: &AppState) -> Vec<ProfileOption> {
        match fetch_profiles(&self.client, &self.base_url, token).await {
            Ok(profiles) if !profiles.is_empty() => profiles,
            _ => profiles_from_sessions(state),
        }
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

async fn fetch_profiles(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<Vec<ProfileOption>> {
    let response: ProfilesResponse = client
        .get(format!("{base_url}/profiles/list"))
        .bearer_auth(token)
        .send()
        .await
        .context("fetch capsem gateway profiles")?
        .error_for_status()
        .context("capsem gateway profiles request failed")?
        .json()
        .await
        .context("parse capsem gateway profiles response")?;
    Ok(response.into_options())
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
            control_message: None,
        },
        active_session_id,
        sessions,
        profiles: Vec::new(),
    }
}

fn profiles_from_sessions(state: &AppState) -> Vec<ProfileOption> {
    let mut profiles = Vec::new();
    for session in &state.sessions {
        if session.profile.is_empty()
            || profiles
                .iter()
                .any(|profile: &ProfileOption| profile.id == session.profile)
        {
            continue;
        }
        profiles.push(ProfileOption {
            id: session.profile.clone(),
            name: session.profile.clone(),
            description: None,
            is_default: profiles.is_empty(),
        });
    }
    profiles
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
        profile: vm
            .profile_id
            .clone()
            .or_else(|| vm.profile_status.clone())
            .unwrap_or_else(|| "default".to_string()),
        profile_status: vm.profile_status,
        can_resume: vm.can_resume,
        resume_blocked_reason: vm.resume_blocked_reason,
        branch: vm.profile_revision,
        persistent: vm.persistent,
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
            "ready" | "ok" | "installed" | "active" | "current"
        )
    }) {
        attention.push(Attention::CredentialIssue);
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
        ControlAction::CreateSession { name, profile_id } => {
            let response = client
                .post(join_url(base_url, &["vms", "create"])?)
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "name": name,
                    "persistent": true,
                    "profile_id": profile_id,
                }))
                .send()
                .await
                .context("create capsem session")?;
            let body = response_json(response).await?;
            let id = body
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("session");
            Ok(ActionOutcome {
                message: format!("created {id}"),
                focus_session: Some(id.to_string()),
            })
        }
        ControlAction::Fork { id, name } => {
            let response = client
                .post(join_url(base_url, &["vms", id, "fork"])?)
                .bearer_auth(token)
                .json(&serde_json::json!({ "name": name }))
                .send()
                .await
                .with_context(|| format!("fork capsem session {id}"))?;
            let body = response_json(response).await?;
            let fork_name = body
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(name);
            Ok(ActionOutcome {
                message: format!("forked {fork_name}"),
                focus_session: Some(fork_name.to_string()),
            })
        }
        ControlAction::Resume { name } => {
            post_empty(client, base_url, token, &["vms", name, "resume"]).await?;
            Ok(ActionOutcome {
                message: format!("resumed {name}"),
                focus_session: Some(name.clone()),
            })
        }
        ControlAction::Checkpoint { id } => {
            post_empty(client, base_url, token, &["vms", id, "pause"]).await?;
            Ok(ActionOutcome {
                message: format!("checkpointed {id}"),
                focus_session: Some(id.clone()),
            })
        }
        ControlAction::Suspend { id } => {
            post_empty(client, base_url, token, &["vms", id, "pause"]).await?;
            Ok(ActionOutcome {
                message: format!("suspended {id}"),
                focus_session: Some(id.clone()),
            })
        }
        ControlAction::Stop { id } => {
            post_empty(client, base_url, token, &["vms", id, "stop"]).await?;
            Ok(ActionOutcome {
                message: format!("stopped {id}"),
                focus_session: Some(id.clone()),
            })
        }
        ControlAction::Delete { id } => {
            let response = client
                .delete(join_url(base_url, &["vms", id, "delete"])?)
                .bearer_auth(token)
                .send()
                .await
                .with_context(|| format!("delete capsem session {id}"))?;
            response_json(response).await?;
            Ok(ActionOutcome {
                message: format!("deleted {id}"),
                focus_session: None,
            })
        }
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

async fn post_empty(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    path_segments: &[&str],
) -> Result<serde_json::Value> {
    let response = client
        .post(join_url(base_url, path_segments)?)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("post gateway action /{}", path_segments.join("/")))?;
    response_json(response).await
}

async fn response_json(response: reqwest::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let text = response
        .text()
        .await
        .context("read gateway action response body")?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("gateway action failed ({status}): {text}"));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(&text).context("parse gateway action response")
}

fn json_u64(body: &serde_json::Value, key: &str) -> u64 {
    body.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
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
    persistent: bool,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    profile_revision: Option<String>,
    #[serde(default)]
    profile_status: Option<String>,
    #[serde(default)]
    can_resume: bool,
    #[serde(default)]
    resume_blocked_reason: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ProfilesResponse {
    #[serde(default)]
    profiles: Vec<ProfileRecordResponse>,
}

impl ProfilesResponse {
    fn into_options(self) -> Vec<ProfileOption> {
        self.profiles
            .into_iter()
            .filter(ProfileRecordResponse::is_tui_launchable)
            .map(|record| {
                let id = record.id;
                ProfileOption {
                    is_default: false,
                    id,
                    name: record.name,
                    description: Some(record.description),
                }
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct ProfileRecordResponse {
    id: String,
    name: String,
    description: String,
    availability: ProfileAvailabilityResponse,
}

impl ProfileRecordResponse {
    fn is_tui_launchable(&self) -> bool {
        self.availability.shell
    }
}

#[derive(Debug, Deserialize)]
struct ProfileAvailabilityResponse {
    shell: bool,
}

#[cfg(test)]
pub(crate) fn state_from_status_json_for_test(raw: &str, latency: Duration) -> Result<AppState> {
    let response: StatusResponse = serde_json::from_str(raw)?;
    Ok(status_response_to_state(response, latency))
}
