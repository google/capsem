//! Gateway-backed CLI requests and explicitly scoped local service administration.

use std::collections::HashMap;
use std::ffi::OsString;
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

/// SDK and CLI consume the same public wire contract.
pub use capsem_api::{
    AssetStatus, ErrorResponse, ExecRequest, ExecResponse, ForkRequest, ForkResponse, HistoryResponse, ListResponse,
    LogsResponse, PersistRequest, ProvisionRequest, ProvisionResponse, PurgeRequest, PurgeResponse, RunRequest,
    SandboxInfo as SessionInfo, UpdateStatusResponse, VmLifecycleState,
};
#[cfg(test)]
pub use capsem_api::{SupplyChainEvidence, UpdateCompatibilityState, UpdateTrackState, UpdateTrackStatus};

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum ApiResponse<T> {
    Ok(T),
    Err(ErrorResponse),
}

impl<T> ApiResponse<T> {
    pub fn into_result(self) -> Result<T> {
        match self {
            Self::Ok(value) => Ok(value),
            Self::Err(error) => Err(anyhow::anyhow!(error.error)),
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
pub(crate) fn isolation_mode_active() -> bool {
    std::env::var("CAPSEM_HOME").map(|v| !v.is_empty()).unwrap_or(false)
}

fn direct_spawn_service_args(
    paths: &paths::CapsemPaths,
    isolated: bool,
    lifetime: DirectServiceLifetime,
) -> Vec<OsString> {
    let mut args = vec![
        "--foreground".into(),
        "--assets-dir".into(),
        paths.assets_dir.clone().into_os_string(),
        "--process-binary".into(),
        paths.process_bin.clone().into_os_string(),
        "--ledger-binary".into(),
        paths.ledger_bin.clone().into_os_string(),
        "--proxy-binary".into(),
        paths.proxy_bin.clone().into_os_string(),
    ];
    if isolated {
        args.extend([OsString::from("--gateway-port"), OsString::from("0")]);
    }
    if lifetime == DirectServiceLifetime::BoundToCommand {
        args.extend([
            OsString::from("--parent-pid"),
            OsString::from(std::process::id().to_string()),
        ]);
    }
    args
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

/// How `ServiceAdminClient::connect_with_timeout` should interpret socket-absent
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

/// Lifetime of a service that the CLI must spawn directly.
///
/// Installed service-manager units remain persistent regardless of this
/// value. The distinction applies only to the unmanaged fallback: `run` is a
/// one-shot operation and must not turn one command into a hidden daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectServiceLifetime {
    Persistent,
    BoundToCommand,
}

pub struct ServiceAdminClient {
    uds_path: PathBuf,
    auto_launch: bool,
    direct_service_lifetime: DirectServiceLifetime,
    owned_direct_service: tokio::sync::Mutex<Option<tokio::process::Child>>,
}

impl ServiceAdminClient {
    pub fn new(uds_path: PathBuf, auto_launch: bool) -> Self {
        Self {
            uds_path,
            auto_launch,
            direct_service_lifetime: DirectServiceLifetime::Persistent,
            owned_direct_service: tokio::sync::Mutex::new(None),
        }
    }

    pub(crate) fn with_direct_service_lifetime(
        uds_path: PathBuf,
        auto_launch: bool,
        direct_service_lifetime: DirectServiceLifetime,
    ) -> Self {
        Self {
            uds_path,
            auto_launch,
            direct_service_lifetime,
            owned_direct_service: tokio::sync::Mutex::new(None),
        }
    }

    /// Finish a request and then reap the unmanaged service this client
    /// spawned for a bounded one-shot command. Existing services and
    /// service-manager launches never populate `owned_direct_service` and are
    /// therefore left untouched.
    pub(crate) async fn finish_direct_request<T>(&self, request: Result<T>) -> Result<T> {
        let shutdown = self.shutdown_owned_direct_service().await;
        match (request, shutdown) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(shutdown_error)) => Err(error.context(format!(
                "the owned one-shot service also failed to shut down: {shutdown_error:#}"
            ))),
        }
    }

    async fn shutdown_owned_direct_service(&self) -> Result<()> {
        let Some(mut child) = self.owned_direct_service.lock().await.take() else {
            return Ok(());
        };

        if let Some(pid) = child.id() {
            let pid = capsem_foundation::unix::process::ProcessId::try_from(pid)
                .context("owned one-shot service returned an invalid process id")?;
            match capsem_foundation::unix::process::send_signal(
                pid,
                capsem_foundation::unix::process::Signal::Terminate,
            ) {
                Ok(
                    capsem_foundation::unix::process::SignalOutcome::Delivered
                    | capsem_foundation::unix::process::SignalOutcome::Gone,
                ) => {}
                Err(error) => {
                    let mut failure = anyhow::Error::new(error).context("failed to stop the owned one-shot service");
                    if let Err(fallback_error) = child.start_kill() {
                        failure = failure.context(format!("fallback child kill also failed: {fallback_error}"));
                    }
                    if let Err(reap_error) = child.wait().await {
                        failure = failure.context(format!("fallback child reap also failed: {reap_error}"));
                    }
                    return Err(failure);
                }
            }
        }
        child
            .wait()
            .await
            .context("failed to reap the owned one-shot service")?;
        match std::fs::remove_file(&self.uds_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove owned service socket {}", self.uds_path.display()))
            }
        }
    }

    /// Connect to the service socket using the shared `poll_until`
    /// primitive. The 5 s deadline, 50ms-500ms exponential backoff, and
    /// "poll succeeded / poll timed out" tracing all come from
    /// `capsem_foundation::poll`; this function only provides the
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
        let opts = capsem_foundation::poll::PollOpts::new("service-connect", timeout);
        let uds_path = &self.uds_path;
        let outcome = capsem_foundation::poll::poll_until(opts, || async move {
            let attempt =
                tokio::time::timeout(std::time::Duration::from_millis(500), UnixStream::connect(uds_path)).await;
            match attempt {
                Ok(Ok(stream)) => Some(Ok(stream)),
                // Only startup can make an absent/refused socket available.
                Ok(Err(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) && matches!(mode, ConnectMode::AwaitStartup) =>
                {
                    None
                }
                Ok(Err(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    None
                }
                // Invalid paths, permissions and other permanent errors cannot
                // recover by waiting, even when the service is starting.
                Ok(Err(e)) => Some(Err(anyhow::anyhow!(
                    "service socket unavailable at {}: {e}",
                    uds_path.display()
                ))),
                // An individual connect attempt exceeded its 500ms budget.
                Err(_) => None,
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
        if service_install::service_explicitly_stopped() {
            anyhow::bail!("capsem service was explicitly stopped; run `capsem start` to start it");
        }
        info!("Service not responding, attempting to launch...");

        // If the service is registered with a service manager, use that exclusively.
        // Direct-spawning when a unit exists would create an unmanaged duplicate.
        // Isolation-mode guard skips this path: when CAPSEM_HOME is set the
        // caller runs against a non-default layout (e.g. `just test` under
        // cache/target/tests/home). The installed LaunchAgent / systemd unit was
        // registered against $HOME/.capsem, so kickstarting it would bind a
        // socket under the real home while this client polls the test home --
        // a guaranteed 5s timeout. Direct-spawn instead; the child inherits
        // CAPSEM_HOME.
        if !isolation_mode_active() && service_install::is_service_installed() {
            info!("Service unit installed, using service manager");
            match paths::try_start_via_service_manager().await {
                Ok(true) => {
                    info!("Service start requested via service manager");
                    return self.connect_with_timeout(ConnectMode::AwaitStartup).await.context(
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
        let paths = paths::discover_paths().context("cannot find capsem binaries for auto-launch")?;

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
        // them alive long after the CLI returns.
        //
        // Detach to a file rather than /dev/null. A panic, and any failure
        // before `telemetry::init` returns, reach stderr and nothing else --
        // exactly the cases where the service's own log is empty and the user
        // has a dead daemon to explain.
        //
        // Under `run/stderr/`, not beside the rotated streams: retention
        // prunes anything in the log directory matching `service*.log`, and
        // unlinking this file would not disturb the fd already open on it.
        // The service would go on writing panics into an inode nobody can
        // reach -- the same silence, harder to notice.
        let stderr_dir = capsem_foundation::paths::capsem_run_dir().join("stderr");
        let service_stderr = std::fs::create_dir_all(&stderr_dir)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(stderr_dir.join("service.log"))
            })
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());
        let mut child = tokio::process::Command::new(&paths.service_bin)
            .args(direct_spawn_service_args(
                &paths,
                isolation_mode_active(),
                self.direct_service_lifetime,
            ))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(service_stderr)
            .spawn()
            .context("failed to spawn capsem-service")?;

        match self.connect_with_timeout(ConnectMode::AwaitStartup).await {
            Ok(stream) => {
                info!("Service spawned and responding");
                if self.direct_service_lifetime == DirectServiceLifetime::BoundToCommand {
                    *self.owned_direct_service.lock().await = Some(child);
                } else {
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                }
                Ok(stream)
            }
            Err(e) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
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
        anyhow::ensure!(
            method == "GET" && matches!(path, "/system/status" | "/assets/status"),
            "direct service access is restricted to local status/asset administration"
        );
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

    /// Start the service this client would auto-launch, unless it already answers.
    pub(crate) async fn ensure_service(&self) -> Result<()> {
        if self.connect_with_timeout(ConnectMode::FailFast).await.is_ok() {
            return Ok(());
        }
        self.try_ensure_service().await.map(drop)
    }

    pub async fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("GET", path, None).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod networks;
mod stream;
pub use networks::*;
pub use stream::StreamEvent;
mod gateway;
pub use gateway::GatewayClient;

#[cfg(test)]
pub(crate) mod tests;
