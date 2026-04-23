//! HTTP-over-UDS client for the capsem service daemon.
//!
//! Contains the `UdsClient`, all request/response types shared with the
//! service API, and small helpers used across command handlers.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::UnixStream;
use tracing::{error, info};

use crate::{paths, service_install};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionRequest {
    pub name: Option<String>,
    pub ram_mb: u64,
    pub cpus: u32,
    #[serde(default)]
    pub persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "image")]
    pub from: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionResponse {
    pub id: String,
    /// Where the per-VM `capsem-process` listens. Returned by the service
    /// so clients never have to recompute the SUN_LEN fallback. `None` only
    /// when talking to an older service that pre-dates this field.
    #[serde(default)]
    pub uds_path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ForkResponse {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SessionInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub pid: u32,
    pub status: String,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub ram_mb: Option<u64>,
    #[serde(default)]
    pub cpus: Option<u32>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub forked_from: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub uptime_secs: Option<u64>,
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub total_output_tokens: Option<u64>,
    #[serde(default)]
    pub total_estimated_cost: Option<f64>,
    #[serde(default)]
    pub total_tool_calls: Option<u64>,
    #[serde(default)]
    pub total_mcp_calls: Option<u64>,
    #[serde(default)]
    pub total_requests: Option<u64>,
    #[serde(default)]
    pub allowed_requests: Option<u64>,
    #[serde(default)]
    pub denied_requests: Option<u64>,
    #[serde(default)]
    pub total_file_events: Option<u64>,
    #[serde(default)]
    pub model_call_count: Option<u64>,
    /// Tail of `process.log` from the last failed boot when
    /// `status == "Defunct"`. Rendered inline by `capsem list` so a
    /// crashed VM shows its own reason on screen.
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListResponse {
    #[serde(rename = "sandboxes")]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RunRequest {
    pub command: String,
    pub timeout_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PurgeRequest {
    pub all: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PurgeResponse {
    pub purged: u32,
    pub persistent_purged: u32,
    pub ephemeral_purged: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogsResponse {
    pub logs: String,
    pub serial_logs: Option<String>,
    pub process_logs: Option<String>,
}

/// A single command history entry from the service.
#[derive(Serialize, Deserialize, Debug)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub layer: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub details: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HistoryResponse {
    pub commands: Vec<HistoryEntry>,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecRequest {
    pub command: String,
    pub timeout_secs: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct ErrorResponse {
    error: String,
}

/// Wrapper for service API responses that may be success or error.
///
/// IMPORTANT: `Ok` must be listed before `Err` so serde's untagged
/// deserialization tries the success variant first.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum ApiResponse<T> {
    Ok(T),
    Err(ErrorResponse),
}

impl<T> ApiResponse<T> {
    pub fn into_result(self) -> Result<T> {
        match self {
            ApiResponse::Ok(t) => Ok(t),
            ApiResponse::Err(e) => Err(anyhow::anyhow!(e.error)),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `-e KEY=VALUE` arguments into a HashMap.
pub fn parse_env_vars(env: &[String]) -> Result<Option<HashMap<String, String>>> {
    if env.is_empty() {
        return Ok(None);
    }
    let mut map = HashMap::new();
    for kv in env {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid env format: expected KEY=VALUE, got: {}", kv))?;
        map.insert(k.to_string(), v.to_string());
    }
    Ok(Some(map))
}

/// Whether the caller is running under an explicit `CAPSEM_HOME` override.
/// In that mode the installed system-wide service unit (registered against
/// the real `$HOME/.capsem`) is the wrong target for auto-launch: it would
/// bind a socket under the default home while this client polls the
/// overridden layout. The auto-launch path direct-spawns instead so the
/// child service inherits `CAPSEM_HOME` and binds the socket the client
/// is actually watching.
fn isolation_mode_active() -> bool {
    std::env::var("CAPSEM_HOME")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Validate that a session identifier is safe for path construction.
/// Rejects identifiers containing path separators or traversal sequences.
pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("session identifier cannot be empty");
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
        anyhow::bail!("invalid session identifier: {}", id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// UDS Client
// ---------------------------------------------------------------------------

/// How `UdsClient::connect_with_timeout` should interpret socket-absent
/// errors (`NotFound` / `ConnectionRefused`).
///
/// Both values run the same 5s `poll_until` budget; they differ only in
/// whether a missing socket short-circuits the poll.
#[derive(Debug, Clone, Copy)]
pub enum ConnectMode {
    /// The service should already be running. Missing socket or
    /// connection refused is a permanent failure -- give up immediately
    /// so CLI calls don't sit for 5s when there's nothing to wait for.
    FailFast,
    /// We just asked the service to start (service manager or direct
    /// spawn). Missing socket is the expected state while the process
    /// boots and binds; keep polling until the socket appears or the
    /// deadline expires.
    AwaitStartup,
}

pub struct UdsClient {
    uds_path: PathBuf,
    auto_launch: bool,
}

impl UdsClient {
    pub fn new(uds_path: PathBuf, auto_launch: bool) -> Self {
        Self {
            uds_path,
            auto_launch,
        }
    }

    /// Connect to the service socket using the shared `poll_until`
    /// primitive. The 5 s deadline, 50ms-500ms exponential backoff, and
    /// "poll succeeded / poll timed out" tracing all come from
    /// `capsem_core::poll`; this function only provides the
    /// connect-attempt closure and the retryable-vs-permanent
    /// classification (see `ConnectMode`).
    async fn connect_with_timeout(&self, mode: ConnectMode) -> Result<UnixStream> {
        self.connect_with_timeout_for_test(mode, std::time::Duration::from_secs(5))
            .await
    }

    /// Same as `connect_with_timeout` but lets tests override the
    /// overall deadline so "timeout expired" paths complete fast.
    async fn connect_with_timeout_for_test(
        &self,
        mode: ConnectMode,
        timeout: std::time::Duration,
    ) -> Result<UnixStream> {
        let opts = capsem_core::poll::PollOpts::new("service-connect", timeout);
        let uds_path = &self.uds_path;
        let outcome = capsem_core::poll::poll_until(opts, || async move {
            let attempt = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                UnixStream::connect(uds_path),
            )
            .await;
            match attempt {
                Ok(Ok(stream)) => Some(Ok(stream)),
                // Retry loops must classify errors (see
                // /dev-rust-patterns lesson 19). `NotFound` /
                // `ConnectionRefused` are "service down" under FailFast
                // (permanent -> Some(Err)) but "socket not bound yet"
                // under AwaitStartup (retryable -> None).
                Ok(Err(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::ConnectionRefused
                    ) && matches!(mode, ConnectMode::FailFast) =>
                {
                    Some(Err(anyhow::anyhow!(
                        "service socket unavailable at {}: {e}",
                        uds_path.display()
                    )))
                }
                // Everything else (other io errors, per-attempt 500ms
                // timeout, NotFound/Refused under AwaitStartup) is
                // retryable.
                _ => None,
            }
        })
        .await;

        match outcome {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => Err(e),
            Err(timed_out) => Err(anyhow::anyhow!(
                "cannot connect to service at {} (timed out after {:?})",
                self.uds_path.display(),
                timed_out.timeout,
            )),
        }
    }

    /// Try to ensure the service is running, returning a live
    /// connection on success. Tries service manager (systemd/launchctl)
    /// if a unit is installed, falls back to direct spawn. Caller
    /// already verified the socket is unreachable.
    async fn try_ensure_service(&self) -> Result<UnixStream> {
        info!("Service not responding, attempting to launch...");

        // If the service is registered with a service manager, use that exclusively.
        // Direct-spawning when a unit exists would create an unmanaged duplicate.
        // Isolation-mode guard skips this path: when CAPSEM_HOME is set the
        // caller runs against a non-default layout (e.g. `just test` under
        // target/test-home). The installed LaunchAgent / systemd unit was
        // registered against $HOME/.capsem, so kickstarting it would bind a
        // socket under the real home while this client polls the test home --
        // a guaranteed 5s timeout. Direct-spawn instead; the child inherits
        // CAPSEM_HOME.
        if !isolation_mode_active() && service_install::is_service_installed() {
            info!("Service unit installed, using service manager");
            match paths::try_start_via_service_manager().await {
                Ok(true) => {
                    info!("Service start requested via service manager");
                    return self
                        .connect_with_timeout(ConnectMode::AwaitStartup)
                        .await
                        .context(
                            "Service manager started capsem but socket not ready. \
                             Check logs: journalctl --user -u capsem (Linux) or \
                             ~/Library/Logs/capsem/service.log (macOS)",
                        );
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!(
                        "Service unit found but service manager reports not installed"
                    ));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Service manager start failed: {}. \
                         Check logs or reinstall with `capsem install`",
                        e
                    ));
                }
            }
        }

        // No service unit installed -- direct spawn fallback
        let paths = paths::discover_paths()
            .context("cannot find capsem binaries for auto-launch")?;

        if !paths.service_bin.exists() {
            return Err(anyhow::anyhow!(
                "capsem-service not found at {}",
                paths.service_bin.display()
            ));
        }

        info!(
            service = %paths.service_bin.display(),
            assets = %paths.assets_dir.display(),
            "spawning service directly"
        );

        // Detach stdio. The spawned service inherits the CLI's stdout/stderr
        // otherwise, which holds those pipes open for its entire lifetime.
        // When the CLI is invoked by a Python harness using
        // `subprocess.run(..., capture_output=True)`, that pipe inheritance
        // turns every `capsem run` into a 120s hang: Python's communicate()
        // waits for EOF on stdout/stderr, but the detached service keeps
        // them alive long after the CLI returns. Service logs go to
        // `<run_dir>/service.log` regardless, so nothing useful is lost.
        let mut child = tokio::process::Command::new(&paths.service_bin)
            .arg("--foreground")
            .arg("--assets-dir")
            .arg(&paths.assets_dir)
            .arg("--process-binary")
            .arg(&paths.process_bin)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn capsem-service")?;

        match self.connect_with_timeout(ConnectMode::AwaitStartup).await {
            Ok(stream) => {
                info!("Service spawned and responding");
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                Ok(stream)
            }
            Err(e) => Err(e).context("capsem-service failed to start"),
        }
    }

    /// Unified HTTP request over UDS. On initial connect failure, tries
    /// `try_ensure_service` which already returns a live stream -- no
    /// redundant third connect needed.
    pub async fn request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<T>,
    ) -> Result<R> {
        let stream = match self.connect_with_timeout(ConnectMode::FailFast).await {
            Ok(s) => s,
            Err(e) if !self.auto_launch => {
                return Err(anyhow::anyhow!(
                    "cannot connect to service at {}: {e}",
                    self.uds_path.display()
                ));
            }
            Err(_) => self.try_ensure_service().await?,
        };

        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                error!("Connection failed: {:?}", err);
            }
        });

        let builder = Request::builder()
            .method(method)
            .uri(format!("http://localhost{}", path))
            .header("Content-Type", "application/json");

        let req = if let Some(b) = body {
            let json = serde_json::to_vec(&b)?;
            builder.body(Full::new(Bytes::from(json)))?
        } else {
            builder.body(Full::new(Bytes::new()))?
        };

        let res = sender.send_request(req).await?;
        let status = res.status();
        let body_bytes = res.collect().await?.to_bytes();

        // Check HTTP status before deserializing. Non-2xx responses are errors
        // regardless of body shape (fixes untagged enum mismatch when T = Value).
        if !status.is_success() {
            if let Ok(err) = serde_json::from_slice::<ErrorResponse>(&body_bytes) {
                return Err(anyhow::anyhow!(err.error));
            }
            return Err(anyhow::anyhow!(
                "request failed ({}): {}",
                status,
                String::from_utf8_lossy(&body_bytes)
            ));
        }

        serde_json::from_slice(&body_bytes).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse response: {e}. Body: {:?}",
                String::from_utf8_lossy(&body_bytes)
            )
        })
    }

    pub async fn post<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: T,
    ) -> Result<R> {
        self.request("POST", path, Some(body)).await
    }

    pub async fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("GET", path, None).await
    }

    pub async fn delete<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("DELETE", path, None).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_id ----------------------------------------------------------

    #[test]
    fn validate_id_normal() {
        assert!(validate_id("vm-abc123").is_ok());
    }

    #[test]
    fn validate_id_with_dots_no_traversal() {
        assert!(validate_id("vm.abc.123").is_ok());
    }

    #[test]
    fn validate_id_uuid_style() {
        assert!(validate_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn validate_id_rejects_empty() {
        let err = validate_id("").unwrap_err();
        assert!(err.to_string().contains("cannot be empty"), "{}", err);
    }

    #[test]
    fn validate_id_rejects_slash() {
        assert!(validate_id("../etc/passwd").is_err());
    }

    #[test]
    fn validate_id_rejects_backslash() {
        assert!(validate_id("..\\windows\\system32").is_err());
    }

    #[test]
    fn validate_id_rejects_dotdot() {
        assert!(validate_id("..").is_err());
    }

    #[test]
    fn validate_id_rejects_traversal_in_middle() {
        assert!(validate_id("foo/../bar").is_err());
    }

    #[test]
    fn validate_id_rejects_null_byte() {
        assert!(validate_id("vm\0evil").is_err());
    }

    #[test]
    fn validate_id_rejects_absolute_path() {
        assert!(validate_id("/tmp/evil").is_err());
    }

    // -- parse_env_vars -------------------------------------------------------

    #[test]
    fn parse_env_vars_empty() {
        assert_eq!(parse_env_vars(&[]).unwrap(), None);
    }

    #[test]
    fn parse_env_vars_single() {
        let vars = vec!["FOO=bar".to_string()];
        let map = parse_env_vars(&vars).unwrap().unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn parse_env_vars_multiple() {
        let vars = vec!["A=1".to_string(), "B=2".to_string()];
        let map = parse_env_vars(&vars).unwrap().unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("A").unwrap(), "1");
        assert_eq!(map.get("B").unwrap(), "2");
    }

    #[test]
    fn parse_env_vars_value_with_equals() {
        let vars = vec!["URL=http://host?a=1&b=2".to_string()];
        let map = parse_env_vars(&vars).unwrap().unwrap();
        assert_eq!(map.get("URL").unwrap(), "http://host?a=1&b=2");
    }

    #[test]
    fn parse_env_vars_empty_value() {
        let vars = vec!["EMPTY=".to_string()];
        let map = parse_env_vars(&vars).unwrap().unwrap();
        assert_eq!(map.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn parse_env_vars_missing_equals() {
        let vars = vec!["NOVAL".to_string()];
        let err = parse_env_vars(&vars).unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"));
    }

    #[test]
    fn parse_env_vars_second_entry_invalid() {
        let vars = vec!["OK=1".to_string(), "BAD".to_string()];
        assert!(parse_env_vars(&vars).is_err());
    }

    // -- ApiResponse ordering -------------------------------------------------

    #[test]
    fn api_response_ok_variant() {
        let json = r#"{"id":"vm-1"}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        let result = resp.into_result().unwrap();
        assert_eq!(result.id, "vm-1");
    }

    #[test]
    fn api_response_err_variant() {
        let json = r#"{"error":"sandbox not found"}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        let err = resp.into_result().unwrap_err();
        assert!(err.to_string().contains("sandbox not found"));
    }

    #[test]
    fn api_response_ok_tried_first() {
        // A response with an "error" field alongside valid fields should
        // still parse as Ok if the Ok type matches first.
        #[derive(Serialize, Deserialize, Debug)]
        struct HasError {
            error: String,
            extra: String,
        }
        let json = r#"{"error":"not-really","extra":"data"}"#;
        let resp: ApiResponse<HasError> = serde_json::from_str(json).unwrap();
        // Since Ok is tried first and HasError has both fields, it should match Ok
        match resp {
            ApiResponse::Ok(v) => {
                assert_eq!(v.error, "not-really");
                assert_eq!(v.extra, "data");
            }
            ApiResponse::Err(_) => panic!("should have parsed as Ok"),
        }
    }

    #[test]
    fn api_response_err_only_when_ok_fails() {
        // When the JSON only has "error" and the Ok type needs "id",
        // serde falls through to Err variant.
        let json = r#"{"error":"vm not found"}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        assert!(resp.into_result().is_err());
    }

    #[test]
    fn api_response_empty_error() {
        let json = r#"{"error":""}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        assert!(resp.into_result().is_err());
    }

    // -- Serde round-trips ----------------------------------------------------

    #[test]
    fn provision_request_serde() {
        let req = ProvisionRequest {
            name: Some("test".into()),
            ram_mb: 4096,
            cpus: 4,
            persistent: true,
            env: None,
            from: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.name, Some("test".into()));
        assert_eq!(req2.ram_mb, 4096);
        assert!(req2.persistent);
        assert!(req2.env.is_none());
    }

    #[test]
    fn provision_request_with_env() {
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());
        let req = ProvisionRequest {
            name: Some("test".into()),
            ram_mb: 2048,
            cpus: 2,
            persistent: true,
            env: Some(env),
            from: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("FOO"));
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.env.as_ref().unwrap().get("FOO").unwrap(), "bar");
    }

    #[test]
    fn provision_request_env_omitted_when_none() {
        let req = ProvisionRequest {
            name: None,
            ram_mb: 2048,
            cpus: 2,
            persistent: false,
            env: None,
            from: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("env"));
    }

    #[test]
    fn provision_request_with_from() {
        let req = ProvisionRequest {
            name: None,
            ram_mb: 2048,
            cpus: 2,
            persistent: false,
            env: None,
            from: Some("my-sandbox".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("my-sandbox"));
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.from, Some("my-sandbox".into()));
    }

    #[test]
    fn provision_request_from_omitted_when_none() {
        let req = ProvisionRequest {
            name: None,
            ram_mb: 2048,
            cpus: 2,
            persistent: false,
            env: None,
            from: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("from"));
    }

    #[test]
    fn list_response_empty_serde() {
        let resp = ListResponse {
            sessions: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        // Wire format uses "sandboxes" key
        assert!(json.contains("sandboxes"));
        let resp2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(resp2.sessions.is_empty());
    }

    #[test]
    fn list_response_with_entries() {
        let resp = ListResponse {
            sessions: vec![
                SessionInfo {
                    id: "vm-1".into(),
                    name: None,
                    pid: 100,
                    status: "Running".into(),
                    persistent: false,
                    ram_mb: Some(2048),
                    cpus: Some(2),
                    version: Some("0.16.1".into()),
                    forked_from: None,
                    description: None,
                    created_at: None,
                    uptime_secs: Some(3600),
                    total_input_tokens: None,
                    total_output_tokens: None,
                    total_estimated_cost: None,
                    total_tool_calls: None,
                    total_mcp_calls: None,
                    total_requests: None,
                    allowed_requests: None,
                    denied_requests: None,
                    total_file_events: None,
                    model_call_count: None,
                    last_error: None,
                },
                SessionInfo {
                    id: "mydev".into(),
                    name: Some("mydev".into()),
                    pid: 0,
                    status: "Stopped".into(),
                    persistent: true,
                    ram_mb: Some(4096),
                    cpus: Some(4),
                    version: None,
                    forked_from: None,
                    description: None,
                    created_at: None,
                    uptime_secs: None,
                    total_input_tokens: None,
                    total_output_tokens: None,
                    total_estimated_cost: None,
                    total_tool_calls: None,
                    total_mcp_calls: None,
                    total_requests: None,
                    allowed_requests: None,
                    denied_requests: None,
                    total_file_events: None,
                    model_call_count: None,
                    last_error: None,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.sessions.len(), 2);
        assert_eq!(resp2.sessions[0].id, "vm-1");
        assert!(!resp2.sessions[0].persistent);
        assert_eq!(resp2.sessions[1].id, "mydev");
        assert!(resp2.sessions[1].persistent);
    }

    #[test]
    fn list_response_as_api_response() {
        // The List endpoint should use ApiResponse wrapping
        let json = r#"{"sandboxes":[]}"#;
        let resp: ApiResponse<ListResponse> = serde_json::from_str(json).unwrap();
        let list = resp.into_result().unwrap();
        assert!(list.sessions.is_empty());
    }

    #[test]
    fn list_response_error_as_api_response() {
        let json = r#"{"error":"service unavailable"}"#;
        let resp: ApiResponse<ListResponse> = serde_json::from_str(json).unwrap();
        let err = resp.into_result().unwrap_err();
        assert!(err.to_string().contains("service unavailable"));
    }

    #[test]
    fn exec_request_serde() {
        let req = ExecRequest {
            command: "ls -la".into(),
            timeout_secs: 30,
        };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ExecRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.command, "ls -la");
        assert_eq!(req2.timeout_secs, 30);
    }

    #[test]
    fn exec_response_serde() {
        let resp = ExecResponse {
            stdout: "hello\n".into(),
            stderr: "".into(),
            exit_code: 0,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.stdout, "hello\n");
        assert_eq!(resp2.exit_code, 0);
    }

    #[test]
    fn exec_response_nonzero_exit() {
        let resp = ExecResponse {
            stdout: "".into(),
            stderr: "not found\n".into(),
            exit_code: 127,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.exit_code, 127);
        assert_eq!(resp2.stderr, "not found\n");
    }

    #[test]
    fn exec_response_negative_exit_code() {
        let resp = ExecResponse {
            stdout: "".into(),
            stderr: "killed".into(),
            exit_code: -1,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.exit_code, -1);
    }

    #[test]
    fn exec_response_signal_exit_code() {
        // SIGKILL = 137 in Docker-style convention
        let resp = ExecResponse {
            stdout: "".into(),
            stderr: "".into(),
            exit_code: 137,
        };
        assert_eq!(resp.exit_code, 137);
    }

    #[test]
    fn fork_request_serde() {
        let req = ForkRequest {
            name: "my-img".into(),
            description: Some("test image".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ForkRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.name, "my-img");
        assert_eq!(req2.description, Some("test image".into()));
    }

    #[test]
    fn fork_request_description_omitted_when_none() {
        let req = ForkRequest {
            name: "img".into(),
            description: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("description"));
    }

    #[test]
    fn purge_response_serde() {
        let resp = PurgeResponse {
            purged: 5,
            persistent_purged: 2,
            ephemeral_purged: 3,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: PurgeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.purged, 5);
        assert_eq!(resp2.persistent_purged, 2);
        assert_eq!(resp2.ephemeral_purged, 3);
    }

    #[test]
    fn run_request_serde() {
        let mut env = HashMap::new();
        env.insert("KEY".into(), "val".into());
        let req = RunRequest {
            command: "echo hi".into(),
            timeout_secs: 60,
            env: Some(env),
        };
        let json = serde_json::to_string(&req).unwrap();
        let req2: RunRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.command, "echo hi");
        assert_eq!(req2.timeout_secs, 60);
        assert_eq!(req2.env.unwrap().get("KEY").unwrap(), "val");
    }

    #[test]
    fn run_request_env_omitted_when_none() {
        let req = RunRequest {
            command: "ls".into(),
            timeout_secs: 30,
            env: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("env"));
    }

    #[test]
    fn logs_response_serde() {
        let resp = LogsResponse {
            logs: "boot log".into(),
            serial_logs: Some("serial output".into()),
            process_logs: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: LogsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.logs, "boot log");
        assert_eq!(resp2.serial_logs, Some("serial output".into()));
        assert!(resp2.process_logs.is_none());
    }

    #[test]
    fn session_info_defaults() {
        // Missing optional fields should deserialize with defaults
        let json = r#"{"id":"vm-1","pid":0,"status":"Running"}"#;
        let info: SessionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "vm-1");
        assert!(!info.persistent);
        assert!(info.ram_mb.is_none());
        assert!(info.cpus.is_none());
        assert!(info.version.is_none());
        assert!(info.name.is_none());
        assert!(info.created_at.is_none());
        assert!(info.uptime_secs.is_none());
        assert!(info.total_input_tokens.is_none());
        assert!(info.total_estimated_cost.is_none());
    }

    // -- connect_with_timeout : ConnectMode contract -------------------------
    //
    // Regression guards for the `capsem doctor` "Service manager started
    // capsem but socket not ready" bug. FailFast must exit immediately when
    // the socket doesn't exist; AwaitStartup must wait inside the 5s poll
    // budget for a just-starting service to bind.

    #[tokio::test]
    async fn connect_fail_fast_errors_immediately_on_missing_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ghost.sock");
        let client = UdsClient::new(sock.clone(), false);

        let start = std::time::Instant::now();
        let err = client
            .connect_with_timeout(ConnectMode::FailFast)
            .await
            .unwrap_err();
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "FailFast should short-circuit, not wait the poll budget (took {elapsed:?})"
        );
        let msg = format!("{err:#}");
        assert!(
            msg.contains("socket") || msg.contains(&*sock.display().to_string()),
            "error should mention the socket or path, got {msg}"
        );
    }

    #[tokio::test]
    async fn connect_await_startup_waits_for_late_binder() {
        use tokio::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("late.sock");
        let sock_for_bind = sock.clone();

        // Bind AFTER a delay -- simulates a service that was just
        // started and hasn't yet called UnixListener::bind.
        let binder = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            UnixListener::bind(&sock_for_bind).unwrap()
        });

        let client = UdsClient::new(sock.clone(), false);
        let stream = client
            .connect_with_timeout(ConnectMode::AwaitStartup)
            .await
            .expect("AwaitStartup must see the late bind within the 5s budget");
        drop(stream);

        // Keep the listener alive until after the connect returned
        // (otherwise the Drop could race with accept).
        drop(binder.await.unwrap());
    }

    #[tokio::test]
    async fn connect_await_startup_eventually_times_out() {
        // Nothing ever binds -- AwaitStartup must still return a
        // timeout error, not hang. Use a short override timeout so
        // the test completes in under a second.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("never.sock");
        let client = UdsClient::new(sock.clone(), false);

        let start = std::time::Instant::now();
        let err = client
            .connect_with_timeout_for_test(
                ConnectMode::AwaitStartup,
                std::time::Duration::from_millis(300),
            )
            .await
            .unwrap_err();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= std::time::Duration::from_millis(250),
            "should have polled until ~timeout, got {elapsed:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "should not exceed budget by much, got {elapsed:?}"
        );
        let msg = format!("{err:#}");
        assert!(
            msg.contains("timed out") || msg.contains("timeout"),
            "expected timeout error, got: {msg}"
        );
    }
}
