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

/// Apply grep filtering to log-valued fields in a JSON object.
fn grep_log_fields(val: &mut Value, pattern: &str) {
    for key in ["logs", "serial_logs", "process_logs"] {
        if let Some(Value::String(s)) = val.get_mut(key) {
            *s = grep_lines(s, pattern);
        }
    }
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
        let bin_dir = exe_path.parent().unwrap();
        let service_bin = bin_dir.join("capsem-service");
        
        if !service_bin.exists() {
            error!(path = %service_bin.display(), "capsem-service binary not found near mcp binary");
            return Err(anyhow::anyhow!("capsem-service not found at {}", service_bin.display()));
        }

        // Try to find assets dir - assume it is in the project root if we are in target/debug
        let project_root = bin_dir.parent().unwrap().parent().unwrap();
        let assets_dir = project_root.join("assets").join(if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" });
        let process_bin = bin_dir.join("capsem-process");

        info!(service = %service_bin.display(), assets = %assets_dir.display(), "spawning service");
        
        let mut child = std::process::Command::new(&service_bin)
            .arg("--foreground")
            .arg("--assets-dir").arg(&assets_dir)
            .arg("--process-binary").arg(&process_bin)
            .spawn()?;

        // Wait up to 5s for socket
        for _ in 0..50 {
            if UnixStream::connect(&self.uds_path).await.is_ok() {
                info!("Service relaunched and responding");
                // Spawn a reaper for the service process so it doesn't become a zombie
                tokio::spawn(async move {
                    let _ = child.wait();
                });
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Err(anyhow::anyhow!("Service failed to start within 5s"))
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
        let body_bytes = res.collect().await?.to_bytes();
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
    /// Name for the VM. Named VMs are persistent ("if you name it, you keep it").
    /// If omitted, creates a temporary VM.
    name: Option<String>,
    #[serde(rename = "ramMb")]
    #[schemars(rename = "ramMb")]
    ram_mb: Option<u64>,
    #[serde(rename = "cpuCount")]
    #[schemars(rename = "cpuCount")]
    cpu_count: Option<u32>,
    version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct NameParams {
    /// Name of the persistent VM
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PersistParams {
    /// ID of the running ephemeral VM
    id: String,
    /// Name to assign (makes it persistent)
    name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct PurgeParams {
    /// Set to true to also destroy persistent VMs
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct RunParams {
    /// Shell command to execute
    command: String,
    /// Timeout in seconds (default 60)
    timeout: Option<u64>,
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
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct ServiceLogsParams {
    /// Case-insensitive substring filter applied to each log line
    grep: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct InspectParams {
    id: String,
    sql: String,
}

#[tool_router]
impl CapsemHandler {
    #[tool(name = "capsem_list", description = "List all VMs (running and stopped persistent) with ID, status, persistence, RAM, CPUs, and version")]
    async fn list(&self) -> Result<String, String> {
        match self.client.request::<Value, Value>("GET", "/list", None).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_vm_logs", description = "Get serial and process logs for a VM. Use grep to filter lines by substring")]
    async fn vm_logs(&self, Parameters(params): Parameters<LogsParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("GET", &format!("/logs/{}", params.id), None).await {
            Ok(mut val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                if let Some(pattern) = &params.grep {
                    grep_log_fields(&mut val, pattern);
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_service_logs", description = "Get the latest capsem-service logs (last ~100KB). Use grep to filter lines by substring")]
    async fn service_logs(&self, Parameters(params): Parameters<ServiceLogsParams>) -> Result<String, String> {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let run_dir = std::env::var("CAPSEM_RUN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(&home).join(".capsem/run"));
        let log_path = run_dir.join("service.log");

        if !log_path.exists() {
            return Err("Service log not found".to_string());
        }

        // Read last 100KB to avoid hitting MCP limits
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&log_path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let start = if len > 100_000 { len - 100_000 } else { 0 };
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;

        let mut buf = String::new();
        file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
        if let Some(pattern) = &params.grep {
            buf = grep_lines(&buf, pattern);
        }
        Ok(buf)
    }

    #[tool(name = "capsem_create", description = "Create a new VM. Named VMs are persistent. Returns VM ID. Defaults: 2048 MB RAM, 2 CPUs. If name already exists, returns error -- use capsem_resume instead")]
    async fn create(&self, Parameters(params): Parameters<CreateParams>) -> Result<String, String> {
        info!(?params, "capsem_create tool called");
        let persistent = params.name.is_some();
        let body = json!({
            "name": params.name,
            "ram_mb": params.ram_mb.unwrap_or(2048),
            "cpus": params.cpu_count.unwrap_or(2),
            "persistent": persistent,
        });
        match self.client.request::<Value, Value>("POST", "/provision", Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => {
                error!(error = %e, "provision request failed");
                Err(e.to_string())
            }
        }
    }

    #[tool(name = "capsem_info", description = "Get VM details: ID, PID, status, persistent, RAM, CPUs, version")]
    async fn info(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("GET", &format!("/info/{}", params.id), None).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_exec", description = "Run a shell command inside the VM. Returns stdout, stderr, exit_code. Default 30s timeout")]
    async fn exec(&self, Parameters(params): Parameters<ExecParams>) -> Result<String, String> {
        let body = build_exec_body(&params);
        match self.client.request::<Value, Value>("POST", &format!("/exec/{}", params.id), Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_read_file", description = "Read a file from the VM guest filesystem. Returns file content as text")]
    async fn read_file(&self, Parameters(params): Parameters<FileReadParams>) -> Result<String, String> {
        let body = json!({
            "path": params.path,
        });
        match self.client.request::<Value, Value>("POST", &format!("/read_file/{}", params.id), Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_write_file", description = "Write a file to the VM guest filesystem")]
    async fn write_file(&self, Parameters(params): Parameters<FileWriteParams>) -> Result<String, String> {
        match self.client.request::<FileWriteParams, Value>("POST", &format!("/write_file/{}", params.id), Some(params)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_inspect_schema", description = "Get CREATE TABLE statements for all telemetry DB tables. Call before capsem_inspect")]
    async fn inspect_schema(&self) -> Result<String, String> {
        Ok(capsem_logger::schema::CREATE_SCHEMA.to_string())
    }

    #[tool(name = "capsem_inspect", description = "Run a SQL query against a VM's telemetry session database. Returns columns and rows")]
    async fn inspect(&self, Parameters(params): Parameters<InspectParams>) -> Result<String, String> {
        match self.client.request::<InspectParams, Value>("POST", &format!("/inspect/{}", params.id), Some(params)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_delete", description = "Delete a VM permanently. Destroys all state including persistent data")]
    async fn delete(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("DELETE", &format!("/delete/{}", params.id), None).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_stop", description = "Stop a VM. Persistent VMs preserve their state; ephemeral VMs are destroyed")]
    async fn stop(&self, Parameters(params): Parameters<IdParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("POST", &format!("/stop/{}", params.id), Some(json!({}))).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_resume", description = "Resume a stopped persistent VM or get ID of a running one. Returns VM ID")]
    async fn resume(&self, Parameters(params): Parameters<NameParams>) -> Result<String, String> {
        match self.client.request::<Value, Value>("POST", &format!("/resume/{}", params.name), Some(json!({}))).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_persist", description = "Convert a running ephemeral VM to a persistent named VM")]
    async fn persist(&self, Parameters(params): Parameters<PersistParams>) -> Result<String, String> {
        let body = json!({ "name": params.name });
        match self.client.request::<Value, Value>("POST", &format!("/persist/{}", params.id), Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_purge", description = "Kill all temporary VMs. Set all=true to also destroy persistent VMs")]
    async fn purge(&self, Parameters(params): Parameters<PurgeParams>) -> Result<String, String> {
        let body = json!({ "all": params.all.unwrap_or(false) });
        match self.client.request::<Value, Value>("POST", "/purge", Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    #[tool(name = "capsem_run", description = "Run a command in a fresh temporary VM. VM is auto-provisioned and destroyed. Returns stdout, stderr, exit_code")]
    async fn run(&self, Parameters(params): Parameters<RunParams>) -> Result<String, String> {
        let body = json!({
            "command": params.command,
            "timeout_secs": params.timeout.unwrap_or(60),
        });
        match self.client.request::<Value, Value>("POST", "/run", Some(body)).await {
            Ok(val) => {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(err.to_string());
                }
                Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| format!("{:?}", val)))
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home).join(".capsem/run"));
    
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

    let uds_path = std::env::var("CAPSEM_UDS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| run_dir.join("service.sock"));
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
            "capsem_inspect", "capsem_delete", "capsem_stop", "capsem_resume",
            "capsem_persist", "capsem_purge", "capsem_run", "capsem_vm_logs",
            "capsem_service_logs",
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
}
