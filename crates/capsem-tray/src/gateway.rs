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
        contents
            .trim()
            .parse::<u16>()
            .context("invalid port in gateway.port")
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
        let run_dir = capsem_core::paths::capsem_run_dir();

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
        let mut status: StatusResponse = resp
            .json()
            .await
            .context("failed to parse status response")?;
        status.latency_ms = Some(start.elapsed().as_millis() as u32);
        match self.update_status().await {
            Ok(updates) => status.updates = Some(updates),
            Err(err) => status.update_error = Some(err.to_string()),
        }
        Ok(status)
    }

    pub async fn update_status(&self) -> Result<UpdateStatusResponse> {
        let resp = self.get("/update/status").await?;
        resp.json()
            .await
            .context("failed to parse update status response")
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
mod tests {
    use super::*;

    #[test]
    fn deserialize_status_response() {
        let json = r#"{
            "service": "running",
            "vm_count": 2,
            "vms": [
                {"id": "abc123", "name": "dev", "status": "running", "persistent": true},
                {"id": "def456", "name": null, "status": "stopped", "persistent": false}
            ]
        }"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.service, "running");
        assert_eq!(resp.vm_count, 2);
        assert_eq!(resp.vms.len(), 2);
        assert_eq!(resp.vms[0].name.as_deref(), Some("dev"));
        assert!(resp.vms[0].persistent);
        assert_eq!(resp.vms[1].name, None);
        assert!(!resp.vms[1].persistent);
    }

    #[test]
    fn deserialize_empty_vm_list() {
        let json = r#"{"service": "running", "vm_count": 0, "vms": []}"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vm_count, 0);
        assert!(resp.vms.is_empty());
    }

    #[test]
    fn deserialize_extra_fields_ignored() {
        let json = r#"{
            "service": "running",
            "vm_count": 0,
            "vms": [],
            "extra_field": "should be ignored"
        }"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vm_count, 0);
    }

    #[test]
    fn deserialize_vm_extra_fields_ignored() {
        let json = r#"{
            "id": "abc",
            "name": "test",
            "status": "running",
            "persistent": true,
            "ram_mb": 512,
            "cpus": 4
        }"#;
        let vm: VmSummary = serde_json::from_str(json).unwrap();
        assert_eq!(vm.id, "abc");
    }

    #[test]
    fn base_url_format() {
        let client = GatewayClient::new(19222, "test-token".into());
        assert_eq!(client.base_url(), "http://127.0.0.1:19222");
        assert_eq!(client.port(), 19222);
    }

    #[test]
    fn auth_header_format() {
        let client = GatewayClient::new(8080, "my-secret".into());
        assert_eq!(
            client.auth_header().unwrap().to_str().unwrap(),
            "Bearer my-secret"
        );
    }

    #[test]
    fn auth_header_rejects_invalid_token_without_panic() {
        let client = GatewayClient::new(8080, "bad\ntoken".into());
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.auth_header()))
                .expect("invalid token must be returned as an error, not a panic");
        assert!(result.is_err(), "invalid token should be rejected");
    }

    // -----------------------------------------------------------------------
    // parse_port_file: cheap guards against corrupt state
    // -----------------------------------------------------------------------

    #[test]
    fn parse_port_file_accepts_plain_number() {
        assert_eq!(GatewayClient::parse_port_file("19222").unwrap(), 19222);
    }

    #[test]
    fn parse_port_file_trims_whitespace() {
        assert_eq!(GatewayClient::parse_port_file("  19222\n").unwrap(), 19222);
    }

    #[test]
    fn parse_port_file_accepts_zero() {
        // Zero is technically valid; discovery may still fail later.
        assert_eq!(GatewayClient::parse_port_file("0").unwrap(), 0);
    }

    #[test]
    fn parse_port_file_rejects_non_numeric() {
        let err = GatewayClient::parse_port_file("abc").unwrap_err();
        assert!(err.to_string().contains("invalid port"));
    }

    #[test]
    fn parse_port_file_rejects_overflow() {
        let err = GatewayClient::parse_port_file("65536").unwrap_err();
        assert!(err.to_string().contains("invalid port"));
    }

    #[test]
    fn parse_port_file_rejects_negative() {
        let err = GatewayClient::parse_port_file("-1").unwrap_err();
        assert!(err.to_string().contains("invalid port"));
    }

    // -----------------------------------------------------------------------
    // Live HTTP against a throwaway local server
    // -----------------------------------------------------------------------

    use std::sync::{Arc, Mutex};

    /// Spawn a tiny TCP echo-style server that records the request line + auth
    /// header, responds with the supplied `body` on `method path`, and 500s
    /// on anything else. Returns (base_url, join_handle, captured_request_line).
    ///
    /// This is intentionally minimal -- no keep-alive, one response per connection.
    async fn spawn_http_probe(
        match_method: &'static str,
        match_path: &'static str,
        status: u16,
        body: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captures_clone = Arc::clone(&captures);

        let handle = tokio::spawn(async move {
            // Serve a single connection.
            if let Ok((mut sock, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                captures_clone.lock().unwrap().push(req.clone());

                // Extract request line + headers for matching.
                let first_line = req.lines().next().unwrap_or("");
                let matches = first_line.starts_with(&format!("{match_method} {match_path} "));
                let (code, reason, resp_body) = if matches {
                    (status, if status == 200 { "OK" } else { "NO" }, body)
                } else {
                    (500, "ERR", "mismatch")
                };
                let content_type = if resp_body.starts_with('{') || resp_body.starts_with('[') {
                    "application/json"
                } else {
                    "text/plain"
                };
                let response = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    content_type,
                    resp_body,
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        (format!("http://{addr}"), captures, handle)
    }

    async fn spawn_http_probe_sequence(
        routes: Vec<(&'static str, &'static str, u16, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captures_clone = Arc::clone(&captures);

        let handle = tokio::spawn(async move {
            for (match_method, match_path, status, body) in routes {
                if let Ok((mut sock, _)) = listener.accept().await {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    captures_clone.lock().unwrap().push(req.clone());

                    let first_line = req.lines().next().unwrap_or("");
                    let matches = first_line.starts_with(&format!("{match_method} {match_path} "));
                    let (code, reason, resp_body) = if matches {
                        (status, if status == 200 { "OK" } else { "NO" }, body)
                    } else {
                        (500, "ERR", "mismatch")
                    };
                    let content_type = if resp_body.starts_with('{') || resp_body.starts_with('[') {
                        "application/json"
                    } else {
                        "text/plain"
                    };
                    let response = format!(
                        "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        content_type,
                        resp_body,
                    );
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.shutdown().await;
                }
            }
        });

        (format!("http://{addr}"), captures, handle)
    }

    fn captured_auth(captures: &Arc<Mutex<Vec<String>>>) -> Option<String> {
        let g = captures.lock().unwrap();
        let req = g.first()?;
        req.lines().find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower.strip_prefix("authorization: ").map(|_| {
                // Return the ORIGINAL header value, not the lowercased one.
                l["authorization: ".len()..].to_string()
            })
        })
    }

    #[tokio::test]
    async fn status_parses_and_measures_latency() {
        let body = r#"{"service":"running","vm_count":1,"vms":[{"id":"abc","name":"dev","status":"running","persistent":true}]}"#;
        let (base, captures, handle) = spawn_http_probe("GET", "/status", 200, body).await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        let status = client.status().await.unwrap();
        handle.await.unwrap();

        assert_eq!(status.service, "running");
        assert_eq!(status.vm_count, 1);
        assert_eq!(status.vms.len(), 1);
        assert_eq!(status.vms[0].id, "abc");
        assert!(status.latency_ms.is_some(), "latency should be recorded");

        // Auth header was sent.
        let auth = captured_auth(&captures).expect("authorization header missing");
        assert_eq!(auth, "Bearer tok");
    }

    #[tokio::test]
    async fn status_attaches_update_status_when_available() {
        let status_body = r#"{"service":"running","vm_count":0,"vms":[]}"#;
        let update_body = r#"{
            "checked_at": 1718444400,
            "channel_url": "https://release.capsem.org/health.json",
            "stale": false,
            "binary": {
                "current": "1.4.0",
                "latest": "1.4.1",
                "update_available": true,
                "state": "update_available",
                "compatibility": "compatible"
            },
            "assets": {
                "current": "assets-1",
                "latest": "assets-1",
                "update_available": false,
                "state": "current",
                "compatibility": "compatible"
            },
            "profiles": {
                "current": "profiles-2030.0101.0",
                "latest": "profiles-2030.0101.1",
                "update_available": false,
                "state": "current",
                "blocked_reason": "requires binary 1.4.1 or newer",
                "compatibility": "unknown"
            },
            "images": {
                "update_available": false,
                "state": "not_published",
                "compatibility": "not_applicable"
            }
        }"#;
        let (base, captures, handle) = spawn_http_probe_sequence(vec![
            ("GET", "/status", 200, status_body),
            ("GET", "/update/status", 200, update_body),
        ])
        .await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());

        let status = client.status().await.unwrap();
        handle.await.unwrap();

        let updates = status.updates.expect("update status should be attached");
        assert!(updates.binary.update_available);
        assert_eq!(updates.binary.latest.as_deref(), Some("1.4.1"));
        assert_eq!(
            updates.profiles.blocked_reason.as_deref(),
            Some("requires binary 1.4.1 or newer")
        );
        assert_eq!(status.update_error, None);

        let captures = captures.lock().unwrap();
        assert!(captures[0].starts_with("GET /status "));
        assert!(captures[1].starts_with("GET /update/status "));
    }

    #[tokio::test]
    async fn status_propagates_http_error() {
        let (base, _, handle) = spawn_http_probe("GET", "/status", 500, "oops").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        let err = client.status().await.unwrap_err();
        handle.await.unwrap();
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn stop_vm_sends_post() {
        let (base, captures, handle) = spawn_http_probe("POST", "/vms/vm-42/stop", 200, "{}").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        client.stop_vm("vm-42").await.unwrap();
        handle.await.unwrap();
        let req = captures.lock().unwrap().first().cloned().unwrap();
        assert!(req.starts_with("POST /vms/vm-42/stop "));
    }

    #[tokio::test]
    async fn delete_vm_sends_delete() {
        let (base, captures, handle) =
            spawn_http_probe("DELETE", "/vms/vm-42/delete", 200, "{}").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        client.delete_vm("vm-42").await.unwrap();
        handle.await.unwrap();
        let req = captures.lock().unwrap().first().cloned().unwrap();
        assert!(req.starts_with("DELETE /vms/vm-42/delete "));
    }

    #[tokio::test]
    async fn suspend_vm_sends_post() {
        let (base, captures, handle) =
            spawn_http_probe("POST", "/vms/vm-42/pause", 200, "{}").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        client.suspend_vm("vm-42").await.unwrap();
        handle.await.unwrap();
        assert!(captures.lock().unwrap()[0].starts_with("POST /vms/vm-42/pause "));
    }

    #[tokio::test]
    async fn resume_vm_sends_post() {
        let (base, captures, handle) =
            spawn_http_probe("POST", "/vms/vm-42/resume", 200, "{}").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        client.resume_vm("vm-42").await.unwrap();
        handle.await.unwrap();
        assert!(captures.lock().unwrap()[0].starts_with("POST /vms/vm-42/resume "));
    }

    #[tokio::test]
    async fn stop_vm_errors_on_http_error_status() {
        let (base, _, handle) = spawn_http_probe("POST", "/vms/vm-x/stop", 404, "not found").await;
        let client = GatewayClient::new_with_base_url(base, "tok".into());
        let err = client.stop_vm("vm-x").await.unwrap_err();
        handle.await.unwrap();
        assert!(err.to_string().contains("404"));
    }

    #[tokio::test]
    async fn request_fails_cleanly_when_host_is_dead() {
        // Construct a client pointed at an unused port; bind+drop to grab one, then use it.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = GatewayClient::new_with_base_url(format!("http://{addr}"), "tok".into());
        let err = client.status().await.unwrap_err();
        // Any connection-level error message is acceptable; what matters is it's Err.
        assert!(!err.to_string().is_empty());
    }
}
