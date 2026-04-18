use anyhow::Result;
use rmcp::{tool, tool_router};
use rmcp::handler::server::{
    router::Router,
    wrapper::Parameters,
    ServerHandler,
};
use rmcp::model::{
    Implementation, InitializeResult, ServerCapabilities,
};
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::HashMap;
use tracing::{error, info};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use hyper::Request;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;
use rmcp::schemars::{self, JsonSchema};

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

/// Body for POST /provision.
fn build_create_body(params: &CreateParams) -> Value {
    let persistent = params.name.is_some();
    let mut body = json!({
        "name": params.name,
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

/// Body for POST /run (fresh temporary session).
fn build_run_body(params: &RunParams) -> Value {
    let mut body = json!({
        "command": params.command,
        "timeout_secs": params.timeout.unwrap_or(60),
    });
    if let Some(ref env) = params.env {
        body["env"] = json!(env);
    }
    body
}

/// Body for POST /fork/{id}.
fn build_fork_body(params: &ForkParams) -> Value {
    json!({
        "name": params.name,
        "description": params.description,
    })
}

/// Body for POST /persist/{id}.
fn build_persist_body(params: &PersistParams) -> Value {
    json!({ "name": params.name })
}

/// Body for POST /purge.
fn build_purge_body(params: &PurgeParams) -> Value {
    json!({ "all": params.all.unwrap_or(false) })
}

/// Body for POST /read_file/{id}.
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
fn resolve_run_dir(home: &str, override_val: Option<&str>) -> PathBuf {
    override_val
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".capsem/run"))
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
        let bin_dir = exe_path.parent()
            .ok_or_else(|| anyhow::anyhow!("exe path has no parent: {}", exe_path.display()))?;
        let service_bin = bin_dir.join("capsem-service");

        if !service_bin.exists() {
            error!(path = %service_bin.display(), "capsem-service binary not found near mcp binary");
            return Err(anyhow::anyhow!("capsem-service not found at {}", service_bin.display()));
        }

        // Assets: always ~/.capsem/assets/ (use `just install` or symlink for dev)
        let home = std::env::var("HOME")
            .map_err(|_| anyhow::anyhow!("HOME not set"))?;
        let assets_dir = std::path::PathBuf::from(home).join(".capsem").join("assets");
        let process_bin = bin_dir.join("capsem-process");

        info!(service = %service_bin.display(), assets = %assets_dir.display(), "spawning service");

        let mut child = tokio::process::Command::new(&service_bin)
            .arg("--foreground")
            .arg("--assets-dir").arg(&assets_dir)
            .arg("--process-binary").arg(&process_bin)
            .spawn()?;

        // Wait up to 5s for socket with exponential backoff
        let uds = self.uds_path.clone();
        capsem_core::poll::poll_until(
            capsem_core::poll::PollOpts::new("service-socket", std::time::Duration::from_secs(5)),
            || {
                let uds = uds.clone();
                async move {
                    UnixStream::connect(&uds).await.ok()
                }
            },
        ).await.map_err(|e| anyhow::anyhow!("{e}"))?;

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
}

#[derive(Clone)]
struct CapsemHandler {
    client: Arc<UdsClient>,
}

impl ServerHandler for CapsemHandler {
    fn get_info(&self) -> InitializeResult {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .build();
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
    /// Name for the session. Named sessions are persistent ("if you name it, you keep it").
    /// If omitted, creates a temporary session.
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
    /// Clone state from an existing persistent session. The new session inherits
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
struct NameParams {
    /// Name of the persistent session
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PersistParams {
    /// ID of the running ephemeral session
    id: String,
    /// Name to assign (makes it persistent)
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PurgeParams {
    /// Set to true to also destroy persistent sessions
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
struct InspectParams {
    id: String,
    sql: String,
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
    #[tool(name = "capsem_list", description = "List all sessions (running and stopped persistent) with ID, name, status, RAM, CPUs, uptime, and telemetry")]
    async fn list(&self) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("GET", "/list", None).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_vm_logs", description = "Get serial and process logs for a session. Use grep to filter lines, tail to limit to last N lines")]
    async fn vm_logs(&self, Parameters(params): Parameters<LogsParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("GET", &format!("/logs/{}", params.id), None).await {
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

    #[tool(name = "capsem_service_logs", description = "Get the latest capsem-service logs (last ~100KB). Use grep to filter lines, tail to limit to last N lines")]
    async fn service_logs(&self, Parameters(params): Parameters<ServiceLogsParams>) -> Result<String, String> {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let run_dir = resolve_run_dir(&home, std::env::var("CAPSEM_RUN_DIR").ok().as_deref());
        let log_path = run_dir.join("service.log");

        if !log_path.exists() {
            return Err("Service log not found".to_string());
        }

        // Read last 100KB to avoid hitting MCP limits -- spawn_blocking for file I/O
        let mut buf = tokio::task::spawn_blocking(move || -> Result<String, String> {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&log_path).map_err(|e| e.to_string())?;
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            let start = len.saturating_sub(100_000);
            file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
            let mut buf = String::new();
            file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf)
        }).await.map_err(|e| e.to_string())??;

        if let Some(pattern) = &params.grep {
            buf = grep_lines(&buf, pattern);
        }
        if let Some(n) = params.tail {
            buf = tail_lines(&buf, n);
        }
        Ok(buf)
    }

    #[tool(name = "capsem_create", description = "Create a new session. Named sessions are persistent. Returns session ID. RAM/CPU default to the user's configured VM settings (vm.resources.ram_gb / cpu_count) when unspecified. If name already exists, returns error -- use capsem_resume instead")]
    async fn create(&self, Parameters(params): Parameters<CreateParams>) -> Result<String, String> {
        info!(?params, "capsem_create tool called");
        let body = build_create_body(&params);
        let resp = self.client.request::<Value, Value>("POST", "/provision", Some(body)).await;
        if let Err(ref e) = resp {
            error!(error = %e, "provision request failed");
        }
        format_service_response(resp)
    }

    #[tool(name = "capsem_info", description = "Get session details: ID, name, status, persistent, RAM, CPUs, version, and telemetry")]
    async fn info(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("GET", &format!("/info/{}", params.id), None).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_exec", description = "Run a shell command inside a session. Returns stdout, stderr, exit_code. Default 30s timeout")]
    async fn exec(&self, Parameters(params): Parameters<ExecParams>) -> Result<String, String> {
        let body = build_exec_body(&params);
        let resp = self.client.request::<Value, Value>("POST", &format!("/exec/{}", params.id), Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_read_file", description = "Read a file from a session's guest filesystem. Returns file content as text")]
    async fn read_file(&self, Parameters(params): Parameters<FileReadParams>) -> Result<String, String> {
        let body = build_read_file_body(&params);
        let resp = self.client.request::<Value, Value>("POST", &format!("/read_file/{}", params.id), Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_write_file", description = "Write a file to a session's guest filesystem")]
    async fn write_file(&self, Parameters(params): Parameters<FileWriteParams>) -> Result<String, String> {
        let path = format!("/write_file/{}", params.id);
        let resp = self.client.request::<FileWriteParams, Value>("POST", &path, Some(params)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_inspect_schema", description = "Get CREATE TABLE statements for all telemetry DB tables. Call before capsem_inspect")]
    async fn inspect_schema(&self) -> Result<String, String> {
        Ok(capsem_logger::schema::CREATE_SCHEMA.to_string())
    }

    #[tool(name = "capsem_inspect", description = "Run a SQL query against a session's telemetry database. Returns columns and rows")]
    async fn inspect(&self, Parameters(params): Parameters<InspectParams>) -> Result<String, String> {
        let path = format!("/inspect/{}", params.id);
        let resp = self.client.request::<InspectParams, Value>("POST", &path, Some(params)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_delete", description = "Delete a session permanently. Destroys all state including persistent data")]
    async fn delete(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("DELETE", &format!("/delete/{}", params.id), None).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_stop", description = "Stop a session. Persistent sessions preserve their state; ephemeral sessions are destroyed")]
    async fn stop(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("POST", &format!("/stop/{}", params.id), Some(json!({}))).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_suspend", description = "Suspend a session. Saves RAM and CPU state. Requires persistent session")]
    async fn suspend(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("POST", &format!("/suspend/{}", params.id), Some(json!({}))).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_resume", description = "Resume a stopped persistent session or get ID of a running one. Returns session ID")]
    async fn resume(&self, Parameters(params): Parameters<NameParams>) -> Result<String, String> {
        let resp = self.client.request::<Value, Value>("POST", &format!("/resume/{}", params.name), Some(json!({}))).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_persist", description = "Convert a running ephemeral session to a persistent named session")]
    async fn persist(&self, Parameters(params): Parameters<PersistParams>) -> Result<String, String> {
        let body = build_persist_body(&params);
        let resp = self.client.request::<Value, Value>("POST", &format!("/persist/{}", params.id), Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_purge", description = "Kill all temporary sessions. Set all=true to also destroy persistent sessions")]
    async fn purge(&self, Parameters(params): Parameters<PurgeParams>) -> Result<String, String> {
        let body = build_purge_body(&params);
        let resp = self.client.request::<Value, Value>("POST", "/purge", Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_run", description = "Run a command in a fresh temporary session. Session is auto-provisioned and destroyed. Returns stdout, stderr, exit_code")]
    async fn run(&self, Parameters(params): Parameters<RunParams>) -> Result<String, String> {
        let body = build_run_body(&params);
        let resp = self.client.request::<Value, Value>("POST", "/run", Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_fork", description = "Fork a running or stopped session into a new stopped persistent session")]
    async fn fork(&self, Parameters(params): Parameters<ForkParams>) -> Result<String, String> {
        info!(?params, "capsem_fork tool called");
        let body = build_fork_body(&params);
        let resp = self.client.request::<Value, Value>("POST", &format!("/fork/{}", params.id), Some(body)).await;
        format_service_response(resp)
    }

    #[tool(name = "capsem_version", description = "Get capsem version info: MCP server version and service connectivity")]
    async fn version(&self) -> Result<String, String> {
        let mcp_version = env!("CARGO_PKG_VERSION");
        let service_status = match self.client.request::<Value, Value>("GET", "/list", None).await {
            Ok(_) => "connected".to_string(),
            Err(e) => format!("unreachable: {}", e),
        };
        Ok(json!({
            "mcp_version": mcp_version,
            "service": service_status,
        }).to_string())
    }

    #[tool(name = "capsem_mcp_servers", description = "List configured MCP servers with connection status and tool counts")]
    async fn mcp_servers(&self) -> Result<String, String> {
        let resp: Vec<Value> = self.client.request("GET", "/mcp/servers", None::<&()>).await
            .map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())
    }

    #[tool(name = "capsem_mcp_tools", description = "List discovered MCP tools across all connected servers. Filter by server name.")]
    async fn mcp_tools(&self, Parameters(params): Parameters<McpToolsParams>) -> Result<String, String> {
        let mut tools: Vec<Value> = self.client.request("GET", "/mcp/tools", None::<&()>).await
            .map_err(|e| e.to_string())?;
        if let Some(ref filter) = params.server {
            tools.retain(|t| t["server_name"].as_str() == Some(filter));
        }
        serde_json::to_string_pretty(&tools).map_err(|e| e.to_string())
    }

    #[tool(name = "capsem_mcp_call", description = "Call an MCP tool by namespaced name (e.g. github__search_repos) with JSON arguments")]
    async fn mcp_call(&self, Parameters(params): Parameters<McpCallParams>) -> Result<String, String> {
        let args = params.arguments.unwrap_or(json!({}));
        let resp: Value = self.client.request(
            "POST",
            &format!("/mcp/tools/{}/call", params.name),
            Some(&args),
        ).await.map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())
    }
}

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME")?;
    let run_dir = resolve_run_dir(&home, std::env::var("CAPSEM_RUN_DIR").ok().as_deref());

    let _ = std::fs::create_dir_all(&run_dir);
    
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("mcp.log"))?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_writer(Arc::new(log_file)))
        .init();

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
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // Param serde roundtrips
    // -----------------------------------------------------------------------

    #[test]
    fn create_params_camel_case() {
        let json = json!({"name": "test", "ramMb": 4096, "cpuCount": 4});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.name, Some("test".into()));
        assert_eq!(p.ram_mb, Some(4096));
        assert_eq!(p.cpu_count, Some(4));
    }

    #[test]
    fn create_params_all_optional() {
        let json = json!({});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.name, None);
        assert_eq!(p.ram_mb, None);
        assert_eq!(p.cpu_count, None);
    }

    #[test]
    fn create_params_serializes_camel() {
        let p = CreateParams {
            name: Some("vm".into()),
            ram_mb: Some(2048),
            cpu_count: Some(2),
            version: None,
            env: None,
            from: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("ramMb").is_some());
        assert!(v.get("cpuCount").is_some());
        // snake_case keys must NOT appear
        assert!(v.get("ram_mb").is_none());
        assert!(v.get("cpu_count").is_none());
    }

    #[test]
    fn exec_params_roundtrip() {
        let json = json!({"id": "vm-1", "command": "echo hi"});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.id, "vm-1");
        assert_eq!(p.command, "echo hi");
        assert_eq!(p.timeout, None);
    }

    #[test]
    fn exec_params_with_timeout() {
        let json = json!({"id": "vm-1", "command": "make build", "timeout": 120});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.timeout, Some(120));
    }

    #[test]
    fn file_read_params_roundtrip() {
        let json = json!({"id": "vm-1", "path": "/tmp/test.txt"});
        let p: FileReadParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.path, "/tmp/test.txt");
    }

    #[test]
    fn file_write_params_roundtrip() {
        let json = json!({"id": "vm-1", "path": "/tmp/test.txt", "content": "data"});
        let p: FileWriteParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.content, "data");
    }

    #[test]
    fn inspect_params_roundtrip() {
        let json = json!({"id": "vm-1", "sql": "SELECT 1"});
        let p: InspectParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.sql, "SELECT 1");
    }

    #[test]
    fn id_params_roundtrip() {
        let json = json!({"id": "my-vm"});
        let p: IdParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.id, "my-vm");
    }

    #[test]
    fn logs_params_with_grep() {
        let json = json!({"id": "vm-1", "grep": "error"});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("error".into()));
    }

    #[test]
    fn logs_params_without_grep() {
        let json = json!({"id": "vm-1"});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, None);
        assert_eq!(p.tail, None);
    }

    #[test]
    fn logs_params_with_tail() {
        let json = json!({"id": "vm-1", "tail": 50});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.tail, Some(50));
    }

    #[test]
    fn logs_params_with_grep_and_tail() {
        let json = json!({"id": "vm-1", "grep": "error", "tail": 20});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("error".into()));
        assert_eq!(p.tail, Some(20));
    }

    #[test]
    fn service_logs_params_with_tail() {
        let json = json!({"tail": 100});
        let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.tail, Some(100));
    }

    #[test]
    fn service_logs_params_with_grep() {
        let json = json!({"grep": "panic"});
        let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("panic".into()));
    }

    #[test]
    fn service_logs_params_empty() {
        let json = json!({});
        let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, None);
    }

    #[test]
    fn logs_params_grep_empty_string() {
        let json = json!({"id": "vm-1", "grep": ""});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("".into()));
    }

    #[test]
    fn logs_params_grep_special_chars() {
        let json = json!({"id": "vm-1", "grep": "[ERROR] (connection)"});
        let p: LogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("[ERROR] (connection)".into()));
    }

    #[test]
    fn service_logs_params_grep_special_chars() {
        let json = json!({"grep": "status=500"});
        let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.grep, Some("status=500".into()));
    }

    // -----------------------------------------------------------------------
    // grep_lines
    // -----------------------------------------------------------------------

    #[test]
    fn tail_lines_basic() {
        let text = "line 1\nline 2\nline 3\nline 4\nline 5";
        assert_eq!(tail_lines(text, 2), "line 4\nline 5");
    }

    #[test]
    fn tail_lines_more_than_available() {
        let text = "line 1\nline 2";
        assert_eq!(tail_lines(text, 10), text);
    }

    #[test]
    fn tail_lines_exact() {
        let text = "line 1\nline 2\nline 3";
        assert_eq!(tail_lines(text, 3), text);
    }

    #[test]
    fn tail_lines_empty() {
        assert_eq!(tail_lines("", 5), "");
    }

    #[test]
    fn tail_log_fields_applies_to_all() {
        let mut val = json!({
            "logs": "a\nb\nc\nd\ne",
            "serial_logs": "1\n2\n3\n4\n5",
            "process_logs": "x\ny\nz",
        });
        tail_log_fields(&mut val, 2);
        assert_eq!(val["logs"], "d\ne");
        assert_eq!(val["serial_logs"], "4\n5");
        assert_eq!(val["process_logs"], "y\nz");
    }

    // -----------------------------------------------------------------------

    #[test]
    fn grep_lines_filters_case_insensitive() {
        let text = "INFO starting\nERROR bad thing\nINFO ok\nError another";
        assert_eq!(grep_lines(text, "error"), "ERROR bad thing\nError another");
    }

    #[test]
    fn grep_lines_no_match() {
        let text = "INFO starting\nINFO ok";
        assert_eq!(grep_lines(text, "error"), "");
    }

    #[test]
    fn grep_lines_empty_input() {
        assert_eq!(grep_lines("", "error"), "");
    }

    #[test]
    fn grep_lines_empty_pattern_matches_all() {
        let text = "line one\nline two\nline three";
        assert_eq!(grep_lines(text, ""), text);
    }

    #[test]
    fn grep_lines_single_line_match() {
        assert_eq!(grep_lines("only line", "only"), "only line");
    }

    #[test]
    fn grep_lines_single_line_no_match() {
        assert_eq!(grep_lines("only line", "missing"), "");
    }

    #[test]
    fn grep_lines_all_lines_match() {
        let text = "error one\nerror two\nerror three";
        assert_eq!(grep_lines(text, "error"), text);
    }

    #[test]
    fn grep_lines_mixed_case_pattern() {
        let text = "ERROR here\nerror there\nErrOr everywhere";
        assert_eq!(grep_lines(text, "ErRoR"), text);
    }

    #[test]
    fn grep_lines_special_chars_literal() {
        // grep_lines does substring matching, not regex -- special chars are literal
        let text = "rate is 99.9%\nrate is 100%\nno rate here";
        assert_eq!(grep_lines(text, "99.9%"), "rate is 99.9%");
    }

    #[test]
    fn grep_lines_regex_metacharacters_are_literal() {
        let text = "file.rs:10\nfilexrs:10\nno match";
        // "." should NOT match "x" -- it's substring, not regex
        assert_eq!(grep_lines(text, "file.rs"), "file.rs:10");
    }

    #[test]
    fn grep_lines_brackets_literal() {
        let text = "vec[0] = 1\nvec_0 = 1\nother";
        assert_eq!(grep_lines(text, "[0]"), "vec[0] = 1");
    }

    #[test]
    fn grep_lines_unicode() {
        let text = "normal line\nline with \u{00e9}m\u{00f8}ji\nanother";
        assert_eq!(grep_lines(text, "\u{00e9}m\u{00f8}"), "line with \u{00e9}m\u{00f8}ji");
    }

    #[test]
    fn grep_lines_preserves_line_order() {
        let text = "c third\na first\nb second";
        assert_eq!(grep_lines(text, ""), "c third\na first\nb second");
    }

    #[test]
    fn grep_lines_trailing_newline() {
        // A trailing newline produces an empty last line -- should not appear in output
        let text = "error here\ninfo there\n";
        assert_eq!(grep_lines(text, "error"), "error here");
    }

    #[test]
    fn grep_lines_whitespace_pattern() {
        let text = "  indented\nnot indented\n\ttabbed";
        assert_eq!(grep_lines(text, "\t"), "\ttabbed");
    }

    // -----------------------------------------------------------------------
    // build_exec_body
    // -----------------------------------------------------------------------

    #[test]
    fn exec_body_default_timeout() {
        let params = ExecParams { id: "vm-1".into(), command: "ls".into(), timeout: None };
        let body = build_exec_body(&params);
        assert_eq!(body["command"], "ls");
        assert_eq!(body["timeout_secs"], 30);
        // id must NOT leak into the body -- it goes in the URL path
        assert!(body.get("id").is_none());
    }

    #[test]
    fn exec_body_custom_timeout() {
        let params = ExecParams { id: "vm-1".into(), command: "make".into(), timeout: Some(120) };
        let body = build_exec_body(&params);
        assert_eq!(body["timeout_secs"], 120);
    }

    #[test]
    fn exec_body_zero_timeout() {
        let params = ExecParams { id: "vm-1".into(), command: "echo".into(), timeout: Some(0) };
        let body = build_exec_body(&params);
        assert_eq!(body["timeout_secs"], 0);
    }

    // -----------------------------------------------------------------------
    // grep_log_fields
    // -----------------------------------------------------------------------

    #[test]
    fn grep_log_fields_filters_all_log_keys() {
        let mut val = json!({
            "logs": "INFO boot\nERROR crash\nINFO done",
            "serial_logs": "serial: ok\nserial: ERROR fail",
            "process_logs": "proc started\nproc ERROR exit",
        });
        grep_log_fields(&mut val, "error");
        assert_eq!(val["logs"], "ERROR crash");
        assert_eq!(val["serial_logs"], "serial: ERROR fail");
        assert_eq!(val["process_logs"], "proc ERROR exit");
    }

    #[test]
    fn grep_log_fields_missing_optional_keys() {
        // serial_logs and process_logs may be absent
        let mut val = json!({ "logs": "INFO ok\nERROR bad" });
        grep_log_fields(&mut val, "error");
        assert_eq!(val["logs"], "ERROR bad");
        assert!(val.get("serial_logs").is_none());
        assert!(val.get("process_logs").is_none());
    }

    #[test]
    fn grep_log_fields_leaves_non_log_keys() {
        let mut val = json!({
            "logs": "INFO ok\nERROR bad",
            "id": "vm-1",
            "status": "running",
        });
        grep_log_fields(&mut val, "error");
        assert_eq!(val["logs"], "ERROR bad");
        // Non-log keys must be untouched
        assert_eq!(val["id"], "vm-1");
        assert_eq!(val["status"], "running");
    }

    #[test]
    fn grep_log_fields_no_match_empties_strings() {
        let mut val = json!({ "logs": "INFO ok\nDEBUG fine" });
        grep_log_fields(&mut val, "panic");
        assert_eq!(val["logs"], "");
    }

    // -----------------------------------------------------------------------
    // UDS path resolution
    // -----------------------------------------------------------------------

    #[test]
    fn uds_path_override_logic() {
        // Test the resolution logic without touching real env vars.
        // If override is Some, use it. If None, fall back to run_dir/service.sock.
        let resolve = |override_val: Option<&str>, run_dir: &str| -> PathBuf {
            override_val
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(run_dir).join("service.sock"))
        };
        assert_eq!(
            resolve(Some("/tmp/custom.sock"), "/ignored"),
            PathBuf::from("/tmp/custom.sock"),
        );
        assert_eq!(
            resolve(None, "/home/user/.capsem/run"),
            PathBuf::from("/home/user/.capsem/run/service.sock"),
        );
    }

    // -----------------------------------------------------------------------
    // inspect_schema
    // -----------------------------------------------------------------------

    #[test]
    fn inspect_schema_contains_create_table() {
        let schema = capsem_logger::schema::CREATE_SCHEMA;
        assert!(schema.contains("CREATE TABLE"));
        assert!(schema.contains("net_events"));
        assert!(schema.contains("model_calls"));
    }

    // -----------------------------------------------------------------------
    // Tool router
    // -----------------------------------------------------------------------

    #[test]
    fn tool_router_registers_all_tools() {
        let tools = CapsemHandler::tool_router();
        let names: Vec<String> = tools.list_all().iter().map(|t| t.name.to_string()).collect();
        let expected = [
            "capsem_list", "capsem_create", "capsem_info", "capsem_exec",
            "capsem_read_file", "capsem_write_file", "capsem_inspect_schema",
            "capsem_inspect", "capsem_delete", "capsem_stop", "capsem_suspend", "capsem_resume",
            "capsem_persist", "capsem_purge", "capsem_run", "capsem_vm_logs",
            "capsem_service_logs", "capsem_version",
            "capsem_fork",
            "capsem_mcp_servers", "capsem_mcp_tools", "capsem_mcp_call",
        ];
        for name in &expected {
            assert!(names.contains(&name.to_string()), "Missing tool: {name}");
        }
        assert_eq!(names.len(), expected.len(), "Extra tools registered: {names:?}");
    }

    // -----------------------------------------------------------------------
    // Handler server info
    // -----------------------------------------------------------------------

    #[test]
    fn server_info_name_and_version() {
        let client = Arc::new(UdsClient::new(PathBuf::from("/dev/null")));
        let handler = CapsemHandler { client };
        let info = handler.get_info();
        assert_eq!(info.server_info.name, "capsem-mcp");
        assert!(!info.server_info.version.is_empty());
    }

    // -----------------------------------------------------------------------
    // Security: path construction safety
    // -----------------------------------------------------------------------

    #[test]
    fn path_construction_with_traversal() {
        // Verify how VM IDs flow into URL paths -- a malicious ID could cause path traversal
        let id = "../../../etc/passwd";
        let path = format!("/exec/{}", id);
        assert_eq!(path, "/exec/../../../etc/passwd");
        // This gets sent as an HTTP path; the service must validate the ID
    }

    #[test]
    fn path_construction_with_empty_id() {
        let id = "";
        let path = format!("/exec/{}", id);
        assert_eq!(path, "/exec/");
        // Empty IDs should be rejected by the service
    }

    #[test]
    fn path_construction_with_slashes() {
        let id = "vm/../../secret";
        let path = format!("/info/{}", id);
        assert!(path.contains("../"), "Path traversal attempt preserved in URL");
    }

    // -----------------------------------------------------------------------
    // Security: parameter edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn exec_params_empty_command() {
        let json = json!({"id": "vm-1", "command": ""});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.command, "");
    }

    #[test]
    fn exec_params_timeout_zero() {
        let json = json!({"id": "vm-1", "command": "echo", "timeout": 0});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.timeout, Some(0));
    }

    #[test]
    fn exec_params_timeout_large() {
        let json = json!({"id": "vm-1", "command": "train", "timeout": 3600});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.timeout, Some(3600));
    }

    #[test]
    fn exec_params_very_long_command() {
        let long_cmd = "a".repeat(100_000);
        let json = json!({"id": "vm-1", "command": long_cmd});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.command.len(), 100_000);
    }

    #[test]
    fn exec_params_shell_metacharacters() {
        let json = json!({"id": "vm-1", "command": "echo $(whoami) | base64; rm -rf /"});
        let p: ExecParams = serde_json::from_value(json).unwrap();
        assert!(p.command.contains("$(whoami)"));
        assert!(p.command.contains("rm -rf"));
    }

    #[test]
    fn file_read_params_path_traversal() {
        let json = json!({"id": "vm-1", "path": "../../etc/shadow"});
        let p: FileReadParams = serde_json::from_value(json).unwrap();
        assert!(p.path.contains(".."));
    }

    #[test]
    fn file_write_params_path_traversal() {
        let json = json!({"id": "vm-1", "path": "/etc/crontab", "content": "* * * * * evil"});
        let p: FileWriteParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.path, "/etc/crontab");
    }

    #[test]
    fn inspect_params_sql_injection() {
        let json = json!({"id": "vm-1", "sql": "SELECT 1; DROP TABLE net_events; --"});
        let p: InspectParams = serde_json::from_value(json).unwrap();
        assert!(p.sql.contains("DROP TABLE"));
        // Backend MUST use read-only connection
    }

    #[test]
    fn create_params_with_env() {
        let json = json!({"name": "test", "env": {"API_KEY": "sk-123", "DEBUG": "true"}});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        let env = p.env.unwrap();
        assert_eq!(env.get("API_KEY").unwrap(), "sk-123");
        assert_eq!(env.get("DEBUG").unwrap(), "true");
    }

    #[test]
    fn create_params_without_env() {
        let json = json!({"name": "test"});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        assert!(p.env.is_none());
    }

    #[test]
    fn create_params_zero_resources() {
        let json = json!({"ramMb": 0, "cpuCount": 0});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.ram_mb, Some(0));
        assert_eq!(p.cpu_count, Some(0));
    }

    #[test]
    fn create_params_huge_resources() {
        let json = json!({"ramMb": u64::MAX, "cpuCount": u32::MAX});
        let p: CreateParams = serde_json::from_value(json).unwrap();
        assert_eq!(p.ram_mb, Some(u64::MAX));
        assert_eq!(p.cpu_count, Some(u32::MAX));
    }

    #[test]
    fn id_params_with_null_bytes() {
        let json = json!({"id": "vm-\0-test"});
        let p: IdParams = serde_json::from_value(json).unwrap();
        assert!(p.id.contains('\0'));
    }

    // -----------------------------------------------------------------------
    // inspect_schema validates
    // -----------------------------------------------------------------------

    #[test]
    fn inspect_schema_has_all_tables() {
        let schema = capsem_logger::schema::CREATE_SCHEMA;
        for table in ["net_events", "model_calls", "tool_calls", "tool_responses", "mcp_calls", "fs_events", "snapshot_events"] {
            assert!(schema.contains(table), "Missing table in schema: {table}");
        }
    }

    // -----------------------------------------------------------------------
    // format_service_response: the common dispatch shape
    // -----------------------------------------------------------------------

    #[test]
    fn format_service_response_ok_pretty_prints() {
        let out = format_service_response(Ok(json!({"id": "vm-1", "status": "running"}))).unwrap();
        assert!(out.contains("\"id\""));
        assert!(out.contains("\"vm-1\""));
        assert!(out.contains('\n'), "pretty print should be multi-line");
    }

    #[test]
    fn format_service_response_ok_with_embedded_error_is_err() {
        let err = format_service_response(Ok(json!({"error": "vm not found"}))).unwrap_err();
        assert_eq!(err, "vm not found");
    }

    #[test]
    fn format_service_response_ok_with_non_string_error_field_is_ok() {
        // If the "error" field isn't a string, it's not the service's error shape; keep as-is.
        let out = format_service_response(Ok(json!({"error": 500, "msg": "fail"}))).unwrap();
        assert!(out.contains("\"error\""));
        assert!(out.contains("500"));
    }

    #[test]
    fn format_service_response_err_returns_message() {
        let err = format_service_response(Err(anyhow::anyhow!("conn reset"))).unwrap_err();
        assert!(err.contains("conn reset"));
    }

    #[test]
    fn format_service_response_null_value_is_ok() {
        let out = format_service_response(Ok(Value::Null)).unwrap();
        assert_eq!(out, "null");
    }

    #[test]
    fn format_service_response_array_value_is_ok() {
        let out = format_service_response(Ok(json!([1, 2, 3]))).unwrap();
        assert!(out.contains('1'));
        assert!(out.contains('2'));
        assert!(out.contains('3'));
    }

    // -----------------------------------------------------------------------
    // build_create_body
    // -----------------------------------------------------------------------

    #[test]
    fn create_body_named_is_persistent() {
        let p = CreateParams { name: Some("dev".into()), ..Default::default() };
        let body = build_create_body(&p);
        assert_eq!(body["name"], "dev");
        assert_eq!(body["persistent"], true);
    }

    #[test]
    fn create_body_unnamed_is_ephemeral() {
        let p = CreateParams::default();
        let body = build_create_body(&p);
        assert_eq!(body["persistent"], false);
        assert!(body["name"].is_null());
    }

    #[test]
    fn create_body_includes_resources_when_present() {
        let p = CreateParams {
            name: Some("dev".into()),
            ram_mb: Some(4096),
            cpu_count: Some(4),
            ..Default::default()
        };
        let body = build_create_body(&p);
        assert_eq!(body["ram_mb"], 4096);
        assert_eq!(body["cpus"], 4);
    }

    #[test]
    fn create_body_omits_resources_when_absent() {
        let p = CreateParams { name: Some("dev".into()), ..Default::default() };
        let body = build_create_body(&p);
        assert!(body.get("ram_mb").is_none());
        assert!(body.get("cpus").is_none());
    }

    #[test]
    fn create_body_includes_env_when_present() {
        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "sk-123".to_string());
        let p = CreateParams {
            name: Some("dev".into()),
            env: Some(env),
            ..Default::default()
        };
        let body = build_create_body(&p);
        assert_eq!(body["env"]["API_KEY"], "sk-123");
    }

    #[test]
    fn create_body_includes_from_clone_source() {
        let p = CreateParams {
            name: Some("new".into()),
            from: Some("src-vm".into()),
            ..Default::default()
        };
        let body = build_create_body(&p);
        assert_eq!(body["from"], "src-vm");
    }

    // -----------------------------------------------------------------------
    // build_run_body
    // -----------------------------------------------------------------------

    #[test]
    fn run_body_default_timeout_is_60() {
        let p = RunParams { command: "echo".into(), timeout: None, env: None };
        let body = build_run_body(&p);
        assert_eq!(body["command"], "echo");
        assert_eq!(body["timeout_secs"], 60);
    }

    #[test]
    fn run_body_custom_timeout() {
        let p = RunParams { command: "make build".into(), timeout: Some(900), env: None };
        let body = build_run_body(&p);
        assert_eq!(body["timeout_secs"], 900);
    }

    #[test]
    fn run_body_with_env() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let p = RunParams { command: "env".into(), timeout: None, env: Some(env) };
        let body = build_run_body(&p);
        assert_eq!(body["env"]["FOO"], "bar");
    }

    #[test]
    fn run_body_without_env_omits_key() {
        let p = RunParams { command: "env".into(), timeout: None, env: None };
        let body = build_run_body(&p);
        assert!(body.get("env").is_none());
    }

    // -----------------------------------------------------------------------
    // build_fork_body
    // -----------------------------------------------------------------------

    #[test]
    fn fork_body_with_description() {
        let p = ForkParams {
            id: "vm-1".into(),
            name: "fork-a".into(),
            description: Some("dev copy".into()),
        };
        let body = build_fork_body(&p);
        assert_eq!(body["name"], "fork-a");
        assert_eq!(body["description"], "dev copy");
    }

    #[test]
    fn fork_body_without_description() {
        let p = ForkParams { id: "vm-1".into(), name: "fork-a".into(), description: None };
        let body = build_fork_body(&p);
        assert_eq!(body["name"], "fork-a");
        assert!(body["description"].is_null());
    }

    // -----------------------------------------------------------------------
    // build_persist_body / build_purge_body / build_read_file_body
    // -----------------------------------------------------------------------

    #[test]
    fn persist_body_contains_name() {
        let p = PersistParams { id: "vm-1".into(), name: "promoted".into() };
        let body = build_persist_body(&p);
        assert_eq!(body["name"], "promoted");
        // id is in URL path, not body
        assert!(body.get("id").is_none());
    }

    #[test]
    fn purge_body_all_defaults_to_false() {
        let p = PurgeParams { all: None };
        let body = build_purge_body(&p);
        assert_eq!(body["all"], false);
    }

    #[test]
    fn purge_body_all_true_preserved() {
        let p = PurgeParams { all: Some(true) };
        let body = build_purge_body(&p);
        assert_eq!(body["all"], true);
    }

    #[test]
    fn read_file_body_contains_path_only() {
        let p = FileReadParams { id: "vm-1".into(), path: "/etc/hostname".into() };
        let body = build_read_file_body(&p);
        assert_eq!(body["path"], "/etc/hostname");
        assert!(body.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // resolve_uds_path / resolve_run_dir
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_uds_path_prefers_override() {
        let run_dir = std::path::Path::new("/ignored/run");
        assert_eq!(
            resolve_uds_path(Some("/tmp/custom.sock"), run_dir),
            PathBuf::from("/tmp/custom.sock"),
        );
    }

    #[test]
    fn resolve_uds_path_falls_back_to_run_dir() {
        let run_dir = std::path::Path::new("/home/u/.capsem/run");
        assert_eq!(
            resolve_uds_path(None, run_dir),
            PathBuf::from("/home/u/.capsem/run/service.sock"),
        );
    }

    #[test]
    fn resolve_run_dir_prefers_override() {
        assert_eq!(
            resolve_run_dir("/home/u", Some("/tmp/run")),
            PathBuf::from("/tmp/run"),
        );
    }

    #[test]
    fn resolve_run_dir_default_under_home() {
        assert_eq!(
            resolve_run_dir("/home/u", None),
            PathBuf::from("/home/u/.capsem/run"),
        );
    }
}
