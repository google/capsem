use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct StatusResponse {
    pub service: String,
    pub vm_count: u32,
    pub vms: Vec<VmSummary>,
    /// Client-side measured latency (not from gateway). Set by the tray poller.
    #[serde(skip)]
    pub latency_ms: Option<u32>,
    /// Best-effort update status from `/update/status`. A failure here should
    /// not hide the ordinary session menu.
    #[serde(skip)]
    pub updates: Option<UpdateStatusResponse>,
    #[serde(skip)]
    pub update_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct VmSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct UpdateStatusResponse {
    #[serde(default)]
    pub checked_at: Option<u64>,
    #[serde(default)]
    pub channel_url: Option<String>,
    pub stale: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    pub binary: UpdateTrackStatus,
    pub assets: UpdateTrackStatus,
    pub profiles: UpdateTrackStatus,
    pub images: UpdateTrackStatus,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
pub struct UpdateTrackStatus {
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    pub update_available: bool,
    pub state: UpdateTrackState,
    pub compatibility: UpdateCompatibilityState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateTrackState {
    Current,
    UpdateAvailable,
    Unknown,
    NotPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCompatibilityState {
    Compatible,
    Unknown,
    NotApplicable,
}

pub struct GatewayClient {
    port: u16,
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl GatewayClient {
    /// Parse a port from `gateway.port` file contents (trimmed).
    fn parse_port_file(contents: &str) -> Result<u16> {
        contents.trim().parse::<u16>().context("invalid port in gateway.port")
    }

    /// Construct a client pointed at the loopback gateway on `port`.
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            base_url: format!("http://127.0.0.1:{port}"),
            token,
            client: reqwest::Client::new(),
        }
    }

    /// Construct a client against an arbitrary base URL (useful in tests so
    /// the client can target a locally-spawned HTTP server on a random port).
    #[cfg(test)]
    pub fn new_with_base_url(base_url: String, token: String) -> Self {
        Self {
            port: 0,
            base_url,
            token,
            client: reqwest::Client::new(),
        }
    }

    /// Discover gateway connection info from well-known file paths.
    /// If `port_override` is set, use that instead of reading the file.
    pub async fn discover(port_override: Option<u16>) -> Result<Self> {
        let run_dir = capsem_foundation::paths::capsem_run_dir();

        let port = match port_override {
            Some(p) => p,
            None => {
                let port_str = tokio::fs::read_to_string(run_dir.join("gateway.port"))
                    .await
                    .context("failed to read gateway.port")?;
                Self::parse_port_file(&port_str)?
            }
        };

        let token = tokio::fs::read_to_string(run_dir.join("gateway.token"))
            .await
            .context("failed to read gateway.token")?
            .trim()
            .to_string();

        debug!(port, "gateway discovered");

        Ok(Self::new(port, token))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn auth_header(&self) -> Result<HeaderValue> {
        HeaderValue::from_str(&format!("Bearer {}", self.token))
            .context("gateway token contains invalid header characters")
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .get(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header()?)
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    async fn post(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .post(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header()?)
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    async fn delete_req(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .delete(format!("{}{path}", self.base_url()))
            .header(AUTHORIZATION, self.auth_header()?)
            .send()
            .await
            .context("gateway request failed")?;

        if !resp.status().is_success() {
            bail!("gateway returned {}", resp.status());
        }
        Ok(resp)
    }

    pub async fn status(&self) -> Result<StatusResponse> {
        let start = std::time::Instant::now();
        let resp = self.get("/status").await?;
        let mut status: StatusResponse = resp.json().await.context("failed to parse status response")?;
        status.latency_ms = Some(start.elapsed().as_millis() as u32);
        match self.update_status().await {
            Ok(updates) => status.updates = Some(updates),
            Err(err) => status.update_error = Some(err.to_string()),
        }
        Ok(status)
    }

    pub async fn update_status(&self) -> Result<UpdateStatusResponse> {
        let resp = self.get("/update/status").await?;
        resp.json().await.context("failed to parse update status response")
    }

    pub async fn stop_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/vms/{id}/stop")).await?;
        Ok(())
    }

    pub async fn delete_vm(&self, id: &str) -> Result<()> {
        self.delete_req(&format!("/vms/{id}/delete")).await?;
        Ok(())
    }

    pub async fn suspend_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/vms/{id}/pause")).await?;
        Ok(())
    }

    pub async fn resume_vm(&self, id: &str) -> Result<()> {
        self.post(&format!("/vms/{id}/resume")).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
