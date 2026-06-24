use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use rmcp::handler::server::{router::Router, wrapper::Parameters, ServerHandler};
use rmcp::model::{Implementation, InitializeResult, ServerCapabilities};
use rmcp::schemars::{self, JsonSchema};
use rmcp::ServiceExt;
use rmcp::{tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixStream;
use tracing::{error, info};

const DEFAULT_PROFILE_ID: &str = "code";

/// Case-insensitive line-level grep over a block of text.
fn grep_lines(text: &str, pattern: &str) -> String {
    let pat = pattern.to_lowercase();
    text.lines()
        .filter(|line| line.to_lowercase().contains(&pat))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build the JSON body for the service exec endpoint.
fn build_exec_body(params: &ExecParams) -> Value {
    json!({
        "command": params.command,
        "timeout_secs": params.timeout.unwrap_or(30),
    })
}

/// Return the last N lines of text.
fn tail_lines(text: &str, n: u64) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = n as usize;
    if lines.len() <= n {
        text.to_string()
    } else {
        lines[lines.len() - n..].join("\n")
    }
}

/// Apply tail to log-valued string fields in a JSON object.
fn tail_log_fields(val: &mut Value, n: u64) {
    for key in ["logs", "serial_logs", "process_logs"] {
        if let Some(Value::String(s)) = val.get_mut(key) {
            *s = tail_lines(s, n);
        }
    }
}

/// Apply grep filtering to log-valued fields in a JSON object.
fn grep_log_fields(val: &mut Value, pattern: &str) {
    for key in ["logs", "serial_logs", "process_logs"] {
        if let Some(Value::String(s)) = val.get_mut(key) {
            *s = grep_lines(s, pattern);
        }
    }
}

/// Render a service response to the shape MCP expects.
///
/// If the underlying request failed, returns the error string. Otherwise,
/// inspects the JSON value for an embedded `{"error": "..."}` field (the
/// service's error shape) and returns it as an Err; else pretty-prints the
/// JSON value. This centralizes the dispatch boilerplate that was duplicated
/// in every tool method.
fn format_service_response(result: Result<Value>) -> Result<String, String> {
    match result {
        Ok(val) => {
            if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                return Err(err.to_string());
            }
            Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Encoding set for a URL query VALUE, per RFC 3986:
/// - "unreserved" chars (ALPHA / DIGIT / `-` / `.` / `_` / `~`) pass through
/// - everything else (incl. `&`, `=`, `+`, `#`, `%`, space, `?`, controls,
///   non-ASCII) is percent-encoded
///
/// `percent-encoding`'s `NON_ALPHANUMERIC` over-encodes the unreserved set;
/// we list the to-encode chars explicitly so the round-trip preserves
/// human-readable values when they're already RFC-safe.
const QUERY_VALUE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Build a `?k1=v1&k2=v2` query string from a list of optional values,
/// percent-encoding each value with `QUERY_VALUE`. Drops `None` entries
/// (no key emitted, no trailing `&`). Returns "" if all are `None`.
///
/// Bug D: prior tools (`capsem_host_logs`, `capsem_panics`, `capsem_triage`,
/// `capsem_timeline`) did raw `format!("k={}&", value)` interpolation. A
/// grep containing a space (e.g. `"capsem-gateway spawned"`) blew up the
/// URL parser with "invalid uri character"; a grep containing `&` (e.g.
/// `"foo&bar"`) was silently truncated to `foo` because the server saw
/// it as two separate query params. This helper centralizes encoding so
/// reserved characters in values cannot collide with URL syntax.
fn query_string<S: AsRef<str>>(params: &[(&str, Option<S>)]) -> String {
    use percent_encoding::utf8_percent_encode;
    let mut parts = Vec::with_capacity(params.len());
    for (key, value) in params {
        if let Some(v) = value {
            parts.push(format!(
                "{}={}",
                key,
                utf8_percent_encode(v.as_ref(), QUERY_VALUE),
            ));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Body for POST /vms/create.
fn build_create_body(params: &CreateParams) -> Value {
    let persistent = params.name.is_some() || params.from.is_some();
    let mut body = json!({
        "name": params.name,
        "profile_id": DEFAULT_PROFILE_ID,
        "persistent": persistent,
    });
    if let Some(ram) = params.ram_mb {
        body["ram_mb"] = json!(ram);
    }
    if let Some(cpus) = params.cpu_count {
        body["cpus"] = json!(cpus);
    }
    if let Some(ref env) = params.env {
        body["env"] = json!(env);
    }
    if let Some(ref from) = params.from {
        body["from"] = json!(from);
    }
    body
}

/// Body for POST /run.
fn build_run_body(params: &RunParams) -> Value {
    let mut body = json!({
        "command": params.command,
        "profile_id": DEFAULT_PROFILE_ID,
        "timeout_secs": params.timeout.unwrap_or(60),
    });
    if let Some(ref env) = params.env {
        body["env"] = json!(env);
    }
    body
}

/// Body for POST /vms/{id}/fork.
fn build_fork_body(params: &ForkParams) -> Value {
    json!({
        "name": params.name,
        "description": params.description,
    })
}

/// Body for POST /vms/{id}/save.
fn build_persist_body(params: &PersistParams) -> Value {
    json!({ "name": params.name })
}

/// Body for POST /purge.
fn build_purge_body(params: &PurgeParams) -> Value {
    json!({ "all": params.all.unwrap_or(false) })
}

/// Body for POST /vms/{id}/files/read.
fn build_read_file_body(params: &FileReadParams) -> Value {
    json!({ "path": params.path })
}

/// Resolve the UDS path following the env-var precedence used by main().
fn resolve_uds_path(override_val: Option<&str>, run_dir: &std::path::Path) -> PathBuf {
    override_val
        .map(PathBuf::from)
        .unwrap_or_else(|| run_dir.join("service.sock"))
}

/// Resolve the capsem run directory from HOME + optional override.
///
/// `home` is accepted for backward compatibility with existing call sites and
/// unit tests; the actual resolution goes through
/// [`capsem_core::paths::capsem_run_dir`] so that `CAPSEM_HOME` is honored.
fn resolve_run_dir(_home: &str, override_val: Option<&str>) -> PathBuf {
    override_val
        .map(PathBuf::from)
        .unwrap_or_else(capsem_core::paths::capsem_run_dir)
}

struct UdsClient {
    uds_path: PathBuf,
}

impl UdsClient {
    fn new(uds_path: PathBuf) -> Self {
        Self { uds_path }
    }

    async fn try_ensure_service(&self) -> Result<()> {
        if UnixStream::connect(&self.uds_path).await.is_ok() {
            return Ok(());
        }

        info!("Service not responding, attempting to relaunch...");
        let exe_path = std::env::current_exe()?;
        let bin_dir = exe_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("exe path has no parent: {}", exe_path.display()))?;
        let service_bin = bin_dir.join("capsem-service");

        if !service_bin.exists() {
            error!(path = %service_bin.display(), "capsem-service binary not found near mcp binary");
            return Err(anyhow::anyhow!(
                "capsem-service not found at {}",
                service_bin.display()
            ));
        }

        // Assets: always <capsem_home>/assets/ (use `just install` or symlink for dev)
        let assets_dir = capsem_core::paths::capsem_assets_dir();
        let process_bin = bin_dir.join("capsem-process");

        info!(service = %service_bin.display(), assets = %assets_dir.display(), "spawning service");

        let mut child = tokio::process::Command::new(&service_bin)
            .arg("--foreground")
            .arg("--assets-dir")
            .arg(&assets_dir)
            .arg("--process-binary")
            .arg(&process_bin)
            .spawn()?;

        // Wait up to 5s for socket with exponential backoff
        let uds = self.uds_path.clone();
        capsem_core::poll::poll_until(
            capsem_core::poll::PollOpts::new("service-socket", std::time::Duration::from_secs(5)),
            || {
                let uds = uds.clone();
                async move { UnixStream::connect(&uds).await.ok() }
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

        info!("Service relaunched and responding");
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        Ok(())
    }

    async fn request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<T>,
    ) -> Result<R> {
        info!(method, path, "sending UDS request");

        // Try to connect, and if it fails, try to relaunch once.
        let stream = match UnixStream::connect(&self.uds_path).await {
            Ok(s) => s,
            Err(_) => {
                self.try_ensure_service().await?;
                UnixStream::connect(&self.uds_path).await?
            }
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

        let res = match sender.send_request(req).await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "failed to send request to service");
                return Err(e.into());
            }
        };
        let status = res.status();
        let body_bytes = res.collect().await?.to_bytes();
        if !status.is_success() {
            // Surface non-2xx as an error. The service returns JSON like
            // {"error": "..."} on failure; prefer that message over the raw
            // status line so callers see actionable detail.
            let msg = serde_json::from_slice::<Value>(&body_bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| String::from_utf8_lossy(&body_bytes).into_owned());
            error!(method, path, status = %status, body = %msg, "service returned non-success status");
            return Err(anyhow::anyhow!("{status}: {msg}"));
        }
        match serde_json::from_slice(&body_bytes) {
            Ok(r) => Ok(r),
            Err(e) => {
                error!(error = %e, body = %String::from_utf8_lossy(&body_bytes), "failed to parse service response");
                Err(e.into())
            }
        }
    }

    /// Send a request and return the raw response body as UTF-8 text.
    /// For endpoints like GET /service-logs that return plain text, not JSON.
    async fn request_text(&self, method: &str, path: &str) -> Result<String> {
        info!(method, path, "sending UDS text request");
        let stream = match UnixStream::connect(&self.uds_path).await {
            Ok(s) => s,
            Err(_) => {
                self.try_ensure_service().await?;
                UnixStream::connect(&self.uds_path).await?
            }
        };
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                error!("Connection failed: {:?}", err);
            }
        });
        let req = Request::builder()
            .method(method)
            .uri(format!("http://localhost{}", path))
            .body(Full::new(Bytes::new()))?;
        let res = sender.send_request(req).await?;
        let status = res.status();
        let body_bytes = res.collect().await?.to_bytes();
        if !status.is_success() {
            let msg = serde_json::from_slice::<Value>(&body_bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| String::from_utf8_lossy(&body_bytes).into_owned());
            return Err(anyhow::anyhow!("{status}: {msg}"));
        }
        Ok(String::from_utf8_lossy(&body_bytes).into_owned())
    }
}

#[derive(Clone)]
struct CapsemHandler {
    client: Arc<UdsClient>,
}

impl ServerHandler for CapsemHandler {
    fn get_info(&self) -> InitializeResult {
        let caps = ServerCapabilities::builder().enable_tools().build();
        let mut info = InitializeResult::new(caps);
        info.server_info = Implementation::new("capsem-mcp", env!("CARGO_PKG_VERSION"));
        info
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct IdParams {
    id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct CreateParams {
    /// Optional requested session name. If omitted, the service assigns a profile-scoped name.
    name: Option<String>,
    #[serde(rename = "ramMb")]
    #[schemars(rename = "ramMb")]
    ram_mb: Option<u64>,
    #[serde(rename = "cpuCount")]
    #[schemars(rename = "cpuCount")]
    cpu_count: Option<u32>,
    version: Option<String>,
    /// Environment variables to inject into the guest (e.g. {"API_KEY": "sk-..."})
    env: Option<HashMap<String, String>>,
    /// Clone state from an existing session. The new session inherits
    /// the source's disk state (workspace, rootfs overlay, session.db).
    from: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct ForkParams {
    /// ID or name of the session to fork
    id: String,
    /// Name for the new session
    name: String,
    /// Optional description
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PersistParams {
    /// ID or name of the running session to save
    id: String,
    /// Name for the saved session
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct NameParams {
    /// Session name or id
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PurgeParams {
    /// Set to true to purge every stopped/broken session the service considers purgeable
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct RunParams {
    /// Shell command to execute
    command: String,
    /// Timeout in seconds (default 60)
    timeout: Option<u64>,
    /// Environment variables to inject into the guest at boot
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct ExecParams {
    id: String,
    command: String,
    /// Timeout in seconds (default 30)
    timeout: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct FileReadParams {
    id: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct FileWriteParams {
    id: String,
    path: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct LogsParams {
    id: String,
    /// Case-insensitive substring filter applied to each log line
    grep: Option<String>,
    /// Return only the last N lines (applied after grep)
    tail: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct ServiceLogsParams {
    /// Case-insensitive substring filter applied to each log line
    grep: Option<String>,
    /// Return only the last N lines (applied after grep)
    tail: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct TriageMcpParams {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h",
    /// "7d", "300s", or RFC3339 ("2026-05-02T17:30:00Z").
    since: Option<String>,
    /// Max items per category. Default 20, max 200.
    limit: Option<u64>,
    /// Optional session id (reserved for the future session.db
    /// cross-reference; ignored today).
    id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct TimelineMcpParams {
    /// Session id.
    id: String,
    /// Filter to one trace_id (hex). Rows with NULL trace_id are also
    /// returned -- they pre-date W4's trace propagation but may still
    /// be relevant context.
    #[serde(rename = "traceId")]
    trace_id: Option<String>,
    /// Lookback window. Default "10m"; accepts "5m", "1h", "24h",
    /// "7d", "300s", or RFC3339.
    since: Option<String>,
    /// Max rows. Default 200, max 2000.
    limit: Option<u64>,
    /// Comma-separated subset of layers: "exec,tool,net,fs,model".
    /// Default all.
    layers: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct HostLogsMcpParams {
    /// One of: "service", "mcp", "gateway", "tray", "app".
    name: String,
    /// Substring filter applied per line.
    grep: Option<String>,
    /// Return only the last N lines (applied after grep).
    tail: Option<u64>,
    /// Max bytes to read from end of file. Default 100KB, max 5MB.
    #[serde(rename = "maxBytes")]
    max_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct McpToolsParams {
    /// Filter tools by server name (optional)
    server: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct McpCallParams {
    /// Namespaced tool name (e.g. github__search_repos)
    name: String,
    /// JSON arguments for the tool call
    arguments: Option<Value>,
}

#[tool_router]
impl CapsemHandler {
    #[tool(
        name = "capsem_list",
        description = "List sessions with ID, name, profile, status, resources, uptime, and telemetry"
    )]
    async fn list(&self) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>("GET", "/vms/list", None)
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_vm_logs",
        description = "Get serial and process logs for a session. Use grep to filter lines, tail to limit to last N lines"
    )]
    async fn vm_logs(&self, Parameters(params): Parameters<LogsParams>) -> Result<String, String> {
        match self
            .client
            .request::<Value, Value>("GET", &format!("/vms/{}/logs", params.id), None)
            .await
        {
            Ok(mut val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                if let Some(pattern) = &params.grep {
                    grep_log_fields(&mut val, pattern);
                }
                if let Some(n) = params.tail {
                    tail_log_fields(&mut val, n);
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(
        name = "capsem_service_logs",
        description = "Get the latest capsem-service logs (last ~100KB). Use grep to filter lines, tail to limit to last N lines"
    )]
    async fn service_logs(
        &self,
        Parameters(params): Parameters<ServiceLogsParams>,
    ) -> Result<String, String> {
        let mut buf = self
            .client
            .request_text("GET", "/service-logs")
            .await
            .map_err(|e| e.to_string())?;
        if let Some(pattern) = &params.grep {
            buf = grep_lines(&buf, pattern);
        }
        if let Some(n) = params.tail {
            buf = tail_lines(&buf, n);
        }
        Ok(buf)
    }

    #[tool(
        name = "capsem_panics",
        description = "Extract structured Rust panics + backtraces from all host log files (service.log, mcp.log, gateway.log, tray.log, capsem-app's latest <ts>.jsonl). Returns one record per panic with binary, location, message, and the first 16 stack frames. Run this FIRST when investigating an unexplained failure -- a single panic ranks higher than a hundred warns."
    )]
    async fn panics(
        &self,
        Parameters(params): Parameters<TriageMcpParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/panics{}",
            query_string(&[
                ("since", params.since.clone()),
                ("limit", params.limit.map(|n| n.to_string())),
            ]),
        );
        let resp = self
            .client
            .request::<Value, Value>("GET", &path, None)
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_triage",
        description = "Opinionated host triage summary: ranked list of recent panics, dropped IPC frames (target=ipc warns), 4xx/5xx server errors (target=service), and slow operations (target=fs op=fsync etc., >500ms). Reads ~/.capsem/run/{service,mcp,gateway,tray}.log and capsem-app's latest jsonl. Use this after capsem_panics to widen the search. Optional `id` parameter is reserved for the future session.db cross-reference (T3)."
    )]
    async fn triage(
        &self,
        Parameters(params): Parameters<TriageMcpParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/triage{}",
            query_string(&[
                ("since", params.since.clone()),
                ("limit", params.limit.map(|n| n.to_string())),
                ("id", params.id.clone()),
            ]),
        );
        let resp = self
            .client
            .request::<Value, Value>("GET", &path, None)
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_host_logs",
        description = "Read a host-side log file by symbolic name. Names: service, mcp, gateway, tray, app (latest jsonl in ~/.capsem/logs/). Use grep + tail to filter. Hard-coded allowlist; no path traversal."
    )]
    async fn host_logs(
        &self,
        Parameters(params): Parameters<HostLogsMcpParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/host-logs/{}{}",
            params.name,
            query_string(&[
                ("grep", params.grep.clone()),
                ("tail", params.tail.map(|n| n.to_string())),
                ("max_bytes", params.max_bytes.map(|n| n.to_string())),
            ]),
        );
        self.client
            .request_text("GET", &path)
            .await
            .map_err(|e| e.to_string())
    }

    #[tool(
        name = "capsem_timeline",
        description = "Render a unified time-ordered timeline for a session, joining exec/tool/net/fs/model events. Optional traceId filter follows one logical operation across layers (W6 added trace_id to every table; pre-W4 rows are NULL and surface alongside). Layers default to all five; pass a subset like `exec,tool` to scope. Use this AFTER capsem_triage / capsem_panics narrow the window."
    )]
    async fn timeline(
        &self,
        Parameters(params): Parameters<TimelineMcpParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/vms/{}/timeline{}",
            params.id,
            query_string(&[
                ("trace_id", params.trace_id.clone()),
                ("since", params.since.clone()),
                ("limit", params.limit.map(|n| n.to_string())),
                ("layers", params.layers.clone()),
            ]),
        );
        self.client
            .request_text("GET", &path)
            .await
            .map_err(|e| e.to_string())
    }

    #[tool(
        name = "capsem_create",
        description = "Create a new profile-owned session. Returns session ID. RAM/CPU default to the selected profile when unspecified. If name already exists, returns an error."
    )]
    async fn create(&self, Parameters(params): Parameters<CreateParams>) -> Result<String, String> {
        info!(?params, "capsem_create tool called");
        let body = build_create_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", "/vms/create", Some(body))
            .await;
        if let Err(ref e) = resp {
            error!(error = %e, "provision request failed");
        }
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_info",
        description = "Get session details: ID, name, profile, status, resources, version, and telemetry"
    )]
    async fn info(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>("GET", &format!("/vms/{}/info", params.id), None)
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_exec",
        description = "Run a shell command inside a session. Returns stdout, stderr, exit_code. Default 30s timeout"
    )]
    async fn exec(&self, Parameters(params): Parameters<ExecParams>) -> Result<String, String> {
        let body = build_exec_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", &format!("/vms/{}/exec", params.id), Some(body))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_read_file",
        description = "Read a file from a session's guest filesystem. Returns file content as text"
    )]
    async fn read_file(
        &self,
        Parameters(params): Parameters<FileReadParams>,
    ) -> Result<String, String> {
        let body = build_read_file_body(&params);
        let resp = self
            .client
            .request::<Value, Value>(
                "POST",
                &format!("/vms/{}/files/read", params.id),
                Some(body),
            )
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_write_file",
        description = "Write a file to a session's guest filesystem"
    )]
    async fn write_file(
        &self,
        Parameters(params): Parameters<FileWriteParams>,
    ) -> Result<String, String> {
        let path = format!("/vms/{}/files/write", params.id);
        let resp = self
            .client
            .request::<FileWriteParams, Value>("POST", &path, Some(params))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_delete",
        description = "Delete a session and destroy its state"
    )]
    async fn delete(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>("DELETE", &format!("/vms/{}/delete", params.id), None)
            .await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_stop", description = "Stop a session")]
    async fn stop(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>("POST", &format!("/vms/{}/stop", params.id), Some(json!({})))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_suspend",
        description = "Pause a session by saving RAM and CPU state"
    )]
    async fn suspend(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>(
                "POST",
                &format!("/vms/{}/pause", params.id),
                Some(json!({})),
            )
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_resume",
        description = "Resume a stopped session or get the ID of a running one"
    )]
    async fn resume(&self, Parameters(params): Parameters<NameParams>) -> Result<String, String> {
        let resp = self
            .client
            .request::<Value, Value>(
                "POST",
                &format!("/vms/{}/resume", params.name),
                Some(json!({})),
            )
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_persist",
        description = "Save a running session under a stable name"
    )]
    async fn persist(
        &self,
        Parameters(params): Parameters<PersistParams>,
    ) -> Result<String, String> {
        let body = build_persist_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", &format!("/vms/{}/save", params.id), Some(body))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_purge",
        description = "Purge stopped, broken, incompatible, or otherwise purgeable sessions"
    )]
    async fn purge(&self, Parameters(params): Parameters<PurgeParams>) -> Result<String, String> {
        let body = build_purge_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", "/purge", Some(body))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_run",
        description = "Run a command in a fresh profile-owned session managed by the service. Returns stdout, stderr, exit_code"
    )]
    async fn run(&self, Parameters(params): Parameters<RunParams>) -> Result<String, String> {
        let body = build_run_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", "/run", Some(body))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_fork",
        description = "Fork a running or stopped session into a new stopped session"
    )]
    async fn fork(&self, Parameters(params): Parameters<ForkParams>) -> Result<String, String> {
        info!(?params, "capsem_fork tool called");
        let body = build_fork_body(&params);
        let resp = self
            .client
            .request::<Value, Value>("POST", &format!("/vms/{}/fork", params.id), Some(body))
            .await;
        format_service_response(resp)
    }

    #[tool(
        name = "capsem_version",
        description = "Get capsem version info: MCP server version and service connectivity"
    )]
    async fn version(&self) -> Result<String, String> {
        let mcp_version = env!("CARGO_PKG_VERSION");
        let service_status = match self
            .client
            .request::<Value, Value>("GET", "/vms/list", None)
            .await
        {
            Ok(_) => "connected".to_string(),
            Err(e) => format!("unreachable: {}", e),
        };
        Ok(json!({
            "mcp_version": mcp_version,
            "service": service_status,
        })
        .to_string())
    }

    #[tool(
        name = "capsem_mcp_servers",
        description = "List configured MCP servers with connection status and tool counts"
    )]
    async fn mcp_servers(&self) -> Result<String, String> {
        let resp: Vec<Value> = self
            .client
            .request(
                "GET",
                &format!("/profiles/{}/mcp/servers/list", DEFAULT_PROFILE_ID),
                None::<&()>,
            )
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())
    }

    #[tool(
        name = "capsem_mcp_tools",
        description = "List discovered MCP tools across all connected servers. Filter by server name."
    )]
    async fn mcp_tools(
        &self,
        Parameters(params): Parameters<McpToolsParams>,
    ) -> Result<String, String> {
        let server_names = if let Some(ref filter) = params.server {
            vec![filter.clone()]
        } else {
            let servers: Vec<Value> = self
                .client
                .request(
                    "GET",
                    &format!("/profiles/{}/mcp/servers/list", DEFAULT_PROFILE_ID),
                    None::<&()>,
                )
                .await
                .map_err(|e| e.to_string())?;
            servers
                .into_iter()
                .filter_map(|server| server["name"].as_str().map(ToOwned::to_owned))
                .collect()
        };
        let mut tools = Vec::new();
        for server_name in server_names {
            let mut server_tools: Vec<Value> = self
                .client
                .request(
                    "GET",
                    &format!(
                        "/profiles/{}/mcp/servers/{}/tools/list",
                        DEFAULT_PROFILE_ID, server_name
                    ),
                    None::<&()>,
                )
                .await
                .map_err(|e| e.to_string())?;
            tools.append(&mut server_tools);
        }
        serde_json::to_string_pretty(&tools).map_err(|e| e.to_string())
    }

    #[tool(
        name = "capsem_mcp_call",
        description = "Call an MCP tool by namespaced name (e.g. github__search_repos) with JSON arguments"
    )]
    async fn mcp_call(
        &self,
        Parameters(params): Parameters<McpCallParams>,
    ) -> Result<String, String> {
        let (server_name, tool_name) = params.name.split_once("__").ok_or_else(|| {
            "MCP tool calls must use namespaced names like server__tool".to_string()
        })?;
        let args = params.arguments.unwrap_or(json!({}));
        let resp: Value = self
            .client
            .request(
                "POST",
                &format!(
                    "/profiles/{}/mcp/servers/{}/tools/{}/call",
                    DEFAULT_PROFILE_ID, server_name, tool_name
                ),
                Some(&args),
            )
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME")?;
    let run_dir = resolve_run_dir(&home, std::env::var("CAPSEM_RUN_DIR").ok().as_deref());

    let _ = std::fs::create_dir_all(&run_dir);

    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-mcp",
        sink: capsem_core::telemetry::LogSink::File {
            path: run_dir.join("mcp.log"),
        },
        default_filter: "info",
    })?;

    info!("capsem-mcp starting");

    let uds_path = resolve_uds_path(std::env::var("CAPSEM_UDS_PATH").ok().as_deref(), &run_dir);
    info!(?uds_path, "connecting to service");
    let client = Arc::new(UdsClient::new(uds_path));

    let handler = CapsemHandler { client };
    let tools = CapsemHandler::tool_router();
    info!("Registered {} tools", tools.list_all().len());
    let router = Router::new(handler.clone()).with_tools(tools);

    let transport = rmcp::transport::stdio();
    router.serve(transport).await?.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests;
