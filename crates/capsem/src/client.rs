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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionResponse {
    pub id: String,
    /// Where the per-VM `capsem-process` listens. Returned by the service
    /// so clients never have to recompute the SUN_LEN fallback. `None` only
    /// when talking to an older service that pre-dates this field.
    #[serde(default)]
    pub uds_path: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_status: Option<SessionProfileStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_pin: Option<SavedVmProfilePin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_health: Option<AssetHealth>,
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
    pub base_assets: Option<SavedVmBaseAssets>,
    #[serde(default)]
    pub profile_pin: Option<SavedVmProfilePin>,
    #[serde(default)]
    pub forked_from: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub profile_status: Option<SessionProfileStatus>,
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
    #[serde(default)]
    pub metrics_schema_version: Option<u32>,
    #[serde(default)]
    pub metrics_captured_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub configured_ram_mb: Option<u64>,
    #[serde(default)]
    pub configured_vcpus: Option<u32>,
    #[serde(default)]
    pub host_pid: Option<u32>,
    #[serde(default)]
    pub host_process_rss_bytes: Option<u64>,
    #[serde(default)]
    pub host_cpu_time_micros: Option<u64>,
    #[serde(default)]
    pub host_cpu_percent: Option<f64>,
    #[serde(default)]
    pub session_disk_bytes: Option<u64>,
    #[serde(default)]
    pub workspace_disk_bytes: Option<u64>,
    #[serde(default)]
    pub rootfs_overlay_bytes: Option<u64>,
    /// Tail of `process.log` from the last failed boot when
    /// `status == "Defunct"`. Rendered inline by `capsem list` so a
    /// crashed VM shows its own reason on screen.
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionProfileStatus {
    Current,
    NeedsUpdate,
    Deprecated,
    Revoked,
    Corrupted,
    Unknown,
}

impl SessionProfileStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::NeedsUpdate => "needs_update",
            Self::Deprecated => "deprecated",
            Self::Revoked => "revoked",
            Self::Corrupted => "corrupted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListResponse {
    #[serde(rename = "sandboxes")]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub asset_health: Option<AssetHealth>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AssetProgress {
    pub logical_name: String,
    pub bytes_done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    pub done: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AssetHealth {
    pub ready: bool,
    #[serde(default = "default_asset_state")]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_payload_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_assets: Vec<ProfileAssetProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AssetProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub saved_vm_dependencies: Vec<SavedVmAssetDependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at_unix_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfileAssetProvenance {
    pub logical_name: String,
    pub hash: String,
    pub source_url: String,
    pub size: u64,
    pub content_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedVmBaseAssets {
    pub asset_version: String,
    pub arch: String,
    pub kernel_hash: String,
    pub initrd_hash: String,
    pub rootfs_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_abi: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedVmProfilePin {
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_payload_hash: Option<String>,
    pub package_contract_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_assets: Option<SavedVmBaseAssets>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedVmAssetDependency {
    pub vm: String,
    pub asset_version: String,
    pub arch: String,
    pub missing: Vec<String>,
    pub recovery_hint: String,
}

fn default_asset_state() -> String {
    "unknown".to_string()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RunRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<String>,
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
    pub security_logs: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
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
        let (k, v) = kv.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid env format: expected KEY=VALUE, got: {}", kv)
        })?;
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
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
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
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                paths::try_start_via_service_manager(),
            )
            .await
            {
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "Service manager start timed out. \
                         Check logs or reinstall with `capsem install`"
                    ));
                }
                Ok(result) => match result {
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
                },
            }
        }

        // No service unit installed -- direct spawn fallback
        let paths =
            paths::discover_paths().context("cannot find capsem binaries for auto-launch")?;

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

        let connect = self.connect_with_timeout(ConnectMode::AwaitStartup);
        tokio::pin!(connect);

        match tokio::select! {
            result = &mut connect => result,
            status = child.wait() => status
                .context("failed to wait for capsem-service startup")
                .and_then(|status| {
                    Err(anyhow::anyhow!(
                        "capsem-service exited before becoming ready: {status}"
                    ))
                }),
        } {
            Ok(stream) => {
                info!("Service spawned and responding");
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                Ok(stream)
            }
            Err(e) => {
                let _ = child.kill().await;
                Err(e).context("capsem-service failed to start")
            }
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

    pub async fn put<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: T,
    ) -> Result<R> {
        self.request("PUT", path, Some(body)).await
    }

    pub async fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("GET", path, None).await
    }

    /// Like `request` but returns the raw response bytes + content-type
    /// instead of deserializing JSON. Used for the file-download
    /// endpoint which returns arbitrary binary payloads with a
    /// detected mime type.
    pub async fn request_bytes(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<(Vec<u8>, Option<String>)> {
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

        let mut builder = Request::builder()
            .method(method)
            .uri(format!("http://localhost{}", path));
        if let Some(ct) = content_type {
            builder = builder.header("Content-Type", ct);
        }
        let req = match body {
            Some(b) => builder.body(Full::new(Bytes::from(b)))?,
            None => builder.body(Full::new(Bytes::new()))?,
        };

        let res = sender.send_request(req).await?;
        let status = res.status();
        let resp_ct = res
            .headers()
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body_bytes = res.collect().await?.to_bytes().to_vec();

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

        Ok((body_bytes, resp_ct))
    }

    pub async fn delete<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("DELETE", path, None).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
