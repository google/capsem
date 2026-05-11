use serde::{Deserialize, Serialize};

/// Messages sent from capsem-service to capsem-process over the per-VM Unix Domain Socket.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceToProcess {
    /// Ping the process to check if it's alive and responsive.
    Ping,
    /// Send input bytes to the guest PTY.
    TerminalInput { data: Vec<u8> },
    /// Resize the guest PTY.
    TerminalResize { cols: u16, rows: u16 },
    /// Request the process to gracefully shut down the VM.
    Shutdown,
    /// Execute a command and wait for completion (structured).
    Exec { id: u64, command: String },
    /// Write a file to the guest.
    WriteFile {
        id: u64,
        path: String,
        data: Vec<u8>,
    },
    /// Read a file from the guest.
    ReadFile { id: u64, path: String },
    /// Request the process to reload its configuration from disk.
    ReloadConfig,
    /// Start streaming terminal output to this IPC connection.
    StartTerminalStream,
    /// Stop streaming terminal output. Sent by `capsem shell` on exit so
    /// the host stops queuing TerminalOutput frames that the client is no
    /// longer reading -- prevents late writes from leaking into the
    /// user's parent shell after raw mode is restored.
    StopTerminalStream,
    /// Quiescence: tell process to prepare guest for snapshot.
    PrepareSnapshot,
    /// Resume guest filesystem I/O after snapshot.
    Unfreeze,
    /// Suspend VM and save checkpoint to disk.
    Suspend { checkpoint_path: String },
    /// Resume VM from checkpoint (warm restore).
    Resume,
    /// Query MCP aggregator for server list with connection status.
    McpListServers { id: u64 },
    /// Query MCP aggregator for discovered tool catalog.
    McpListTools { id: u64 },
    /// Tell MCP aggregator to reconnect all servers with fresh config.
    McpRefreshTools { id: u64 },
    /// Call an MCP tool via the aggregator subprocess.
    ///
    /// `arguments_json` is the JSON-serialized argument object. We send it as
    /// a `String`, not a `serde_json::Value`, because the IPC transport
    /// (`tokio-unix-ipc` -> bincode) is not self-describing and bincode
    /// refuses `serde_json::Value::deserialize` (which calls
    /// `deserialize_any`). Without this, every `capsem_mcp_call` silently
    /// dropped the message in capsem-process and the service hit its 60s
    /// receive timeout.
    McpCallTool {
        id: u64,
        namespaced_name: String,
        arguments_json: String,
    },
}

/// Messages sent from capsem-process back to capsem-service over the per-VM UDS.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProcessToService {
    /// Response to Ping.
    Pong,
    /// Response to ReloadConfig.
    ReloadConfigResult {
        success: bool,
        error: Option<String>,
    },
    /// Output bytes from the guest PTY.
    TerminalOutput { data: Vec<u8> },
    /// State change notification (e.g. Booting -> Running).
    StateChanged {
        id: String,
        state: String,
        trigger: String,
    },
    /// Result of an Exec command.
    ExecResult {
        id: u64,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    /// Result of a WriteFile operation.
    WriteFileResult {
        id: u64,
        success: bool,
        error: Option<String>,
    },
    /// Result of a ReadFile operation.
    ReadFileResult {
        id: u64,
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    /// Guest requested shutdown (forwarded from capsem-sysutil via vsock:5004).
    ShutdownRequested { id: String },
    /// Guest requested suspend (forwarded from capsem-sysutil via vsock:5004).
    SuspendRequested { id: String },
    /// Guest quiescence complete: filesystem frozen, safe to snapshot.
    SnapshotReady { id: String },
    /// Response to McpListServers.
    McpServersResult {
        id: u64,
        servers: Vec<McpServerStatus>,
    },
    /// Response to McpListTools.
    McpToolsResult { id: u64, tools: Vec<McpToolStatus> },
    /// Response to McpRefreshTools.
    McpRefreshResult {
        id: u64,
        success: bool,
        error: Option<String>,
    },
    /// Response to McpCallTool. `result_json` is a JSON-serialized
    /// `serde_json::Value`, wrapped for the same bincode reason as
    /// `McpCallTool::arguments_json`.
    McpCallToolResult {
        id: u64,
        result_json: Option<String>,
        error: Option<String>,
    },
}

/// Status of an MCP server as reported through IPC.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServerStatus {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub source: String,
    pub is_stdio: bool,
    pub connected: bool,
    pub tool_count: usize,
}

/// Status of an MCP tool as reported through IPC.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpToolStatus {
    pub namespaced_name: String,
    pub original_name: String,
    pub description: Option<String>,
    pub server_name: String,
    pub annotations: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests;
