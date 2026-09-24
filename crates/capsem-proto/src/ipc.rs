use serde::{Deserialize, Serialize};

/// Explicit host/guest file boundary action.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileBoundaryAction {
    Import,
    Export,
}

/// Messages sent from capsem-service to capsem-process over the per-VM Unix Domain Socket.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceToProcess {
    /// Ping the process to check if it's alive and responsive.
    Ping,
    /// Send input bytes to the guest PTY.
    TerminalInput {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// Resize the guest PTY.
    TerminalResize {
        cols: u16,
        rows: u16,
    },
    /// Request the process to gracefully shut down the VM.
    Shutdown,
    /// Execute a command and wait for completion (structured).
    Exec {
        id: u64,
        command: String,
    },
    /// Write a file to the guest.
    WriteFile {
        id: u64,
        path: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// Read a file from the guest.
    ReadFile {
        id: u64,
        path: String,
    },
    /// Record an explicit file import/export boundary through the process-owned
    /// security-event ledger.
    LogFileBoundary {
        id: u64,
        action: FileBoundaryAction,
        path: String,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        size: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// Request the process to reload its active profile from disk. Answered by
    /// `ConfigReloadResult` naming the digest of the bytes it applied.
    ReloadConfig {
        id: u64,
    },
    /// Start streaming terminal output to this IPC connection.
    StartTerminalStream,
    /// Stop streaming terminal output. Sent by `capsem shell` on exit so
    /// the host stops queuing TerminalOutput frames that the client is no
    /// longer reading -- prevents late writes from leaking into the
    /// user's parent shell after raw mode is restored.
    StopTerminalStream,
    /// Suspend VM and save checkpoint to disk.
    Suspend {
        checkpoint_path: String,
    },
    /// Clone this sandbox's state into `destination`, an empty session
    /// directory the service created. The owner freezes the guest's system
    /// filesystem for the copy and always thaws it, so the fork's overlay
    /// image is consistent and a service that disappears mid-fork cannot
    /// leave the guest frozen.
    CloneState {
        id: u64,
        destination: String,
    },
    /// Query MCP aggregator for server list with connection status.
    McpListServers {
        id: u64,
    },
    /// Query MCP aggregator for discovered tool catalog.
    McpListTools {
        id: u64,
    },
    /// Tell MCP aggregator to reconnect all servers with fresh config.
    McpRefreshTools {
        id: u64,
    },
    /// Call an MCP tool via the aggregator subprocess.
    ///
    /// `arguments_json` is the JSON-serialized argument object. Keeping the
    /// MCP boundary as JSON preserves the protocol payload exactly and avoids
    /// coupling internal IPC types to an MCP library's dynamic value shape.
    McpCallTool {
        id: u64,
        namespaced_name: String,
        arguments_json: String,
    },
    /// Execute with bounded live stdout/stderr, followed by ExecResult.
    ExecStream {
        id: u64,
        command: String,
    },
    /// Send bytes to one running exec's stdin.
    ExecStreamInput {
        id: u64,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// Deliver explicit in-band EOF to one running exec's stdin.
    ExecStreamCloseStdin {
        id: u64,
    },
    /// Cancel one running guest exec and its process group.
    CancelExec {
        id: u64,
    },
    /// Publish one loopback host TCP port into the `target` guest namespace.
    PublishPort {
        id: u64,
        host_port: u16,
        guest_port: u16,
        target: crate::PublicationTarget,
    },
    /// Declare an HTTP preview without opening a bypass listener.
    DeclarePreview {
        id: u64,
        listener_port: u16,
        guest_port: u16,
        target: crate::PublicationTarget,
    },
    /// Close a declared exposure. Loopback ids remain their host-port text;
    /// preview ids are owner-generated UUIDs.
    RevokeExposure {
        id: u64,
        exposure_id: String,
    },
    /// The owner's live publications and its generation.
    ListPublications {
        id: u64,
    },
    CreatePreviewSession {
        id: u64,
        exposure_id: String,
    },
    /// End every session of a preview exposure and the flows they admitted;
    /// the exposure itself stays declared.
    RevokePreviewSessions {
        id: u64,
        exposure_id: String,
    },
    ExchangePreviewBootstrap {
        id: u64,
        exposure_id: String,
        bootstrap_token: String,
    },
    AdmitPreviewConnection {
        id: u64,
        exposure_id: String,
        session_token: String,
        kind: crate::PreviewAdmissionKind,
    },
    /// Internal VM-owner request for one declared publication data stream.
    ConnectPort {
        flow: crate::router::FlowKey,
        port: u16,
        target: crate::PublicationTarget,
    },
    /// Internal VM-owner cancellation for bounded generation-bound flows.
    AbortPorts {
        flows: Vec<crate::router::FlowKey>,
    },
    /// The service is plugging this VM into a network's switch and wants the
    /// guest's stream for that network's cable. The owner evaluates its
    /// profile once, has the guest bring the cable up with `address`/`prefix`,
    /// then answers with the handoff socket the service should ask on, keyed
    /// by `token`; the stream comes back on that connection. `generation` is
    /// the attachment's, which only grows: the cable remembers the newest.
    LinkAttach {
        id: u64,
        token: String,
        network: String,
        network_name: String,
        address: std::net::Ipv4Addr,
        prefix: u8,
        generation: u32,
    },
    /// The VM left `network` as of `generation`: the owner forgets the
    /// network's cable, and the guest's tap for it goes away, unless a newer
    /// plug already took the cable over. Requests can reach the owner in
    /// either order; the generation, not arrival, decides.
    LinkDetach {
        id: u64,
        network: String,
        generation: u32,
    },
    /// Internal VM-owner request: bring a cable up in the guest.
    PlugCable {
        cable: u32,
        address: std::net::Ipv4Addr,
        prefix: u8,
    },
    /// Internal VM-owner request: take a cable down in the guest.
    UnplugCable {
        cable: u32,
    },
    /// Ask the VM owner to apply its effective policy and admit the primary
    /// audit row before the service opens a registry connection. Credentials
    /// are deliberately absent.
    AdmitContainerPull {
        id: u64,
        image: String,
        registry: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        digest: Option<String>,
    },
}

/// Messages sent from capsem-process back to capsem-service over the per-VM UDS.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProcessToService {
    /// Response to Ping.
    Pong,
    /// Output bytes from the guest PTY.
    TerminalOutput {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// State change notification (e.g. Booting -> Running).
    StateChanged { id: String, state: String, trigger: String },
    /// Result of an Exec command.
    ExecResult {
        id: u64,
        #[serde(with = "serde_bytes")]
        stdout: Vec<u8>,
        #[serde(with = "serde_bytes")]
        stderr: Vec<u8>,
        exit_code: i32,
        /// The guest wrote more output than the per-exec cap, so `stdout`
        /// holds the retained prefix only. Callers that render output need
        /// this to say so rather than presenting a short result as complete.
        #[serde(default, skip_serializing_if = "crate::sparse::is_default")]
        truncated: bool,
    },
    /// Result of a WriteFile operation.
    WriteFileResult {
        id: u64,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Result of a ReadFile operation.
    ReadFileResult {
        id: u64,
        #[serde(with = "crate::wire_bytes::option")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Vec<u8>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Result of an explicit file import/export boundary ledger write.
    LogFileBoundaryResult {
        id: u64,
        success: bool,
        #[serde(with = "crate::wire_bytes::option")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Vec<u8>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Guest requested shutdown (forwarded from capsem-sysutil via vsock:5004).
    ShutdownRequested { id: String },
    /// Guest requested suspend (forwarded from capsem-sysutil via vsock:5004).
    SuspendRequested { id: String },
    /// Result of CloneState: the clone's disk usage, or why it failed.
    CloneStateResult {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to McpListServers.
    McpServersResult { id: u64, servers: Vec<McpServerStatus> },
    /// Response to McpListTools.
    McpToolsResult { id: u64, tools: Vec<McpToolStatus> },
    /// Response to McpRefreshTools.
    McpRefreshResult {
        id: u64,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to McpCallTool. `result_json` preserves the MCP JSON value.
    McpCallToolResult {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Warm suspend failed before the durable checkpoint marker was written.
    /// Named MessagePack variants remain stable independently of source order.
    SuspendFailed { id: String, error: String },
    /// Live stdout or stderr for an ExecStream job. Each chunk is at most 8 KiB.
    ExecOutput {
        id: u64,
        channel: crate::ExecOutputChannel,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// One stdin frame of a streaming exec left the owner's queue for the
    /// guest; the service may send one more. See
    /// [`crate::exec_stream::EXEC_STDIN_WINDOW`].
    ExecInputConsumed { id: u64 },
    PortPublished {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        publication: Option<PublicationInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// The VM's rules refused the exposure, rather than it failing to open.
        policy_refused: bool,
    },
    /// Response to LinkAttach: where the service asks for the stream, or
    /// why this owner will not link.
    LinkAttachResult {
        id: u64,
        handoff_socket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to LinkDetach.
    LinkDetachResult {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    PreviewSessionCreated {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bootstrap_token: Option<String>,
        expires_in_seconds: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to RevokePreviewSessions: how many sessions ended.
    PreviewSessionsRevoked {
        id: u64,
        revoked: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    PreviewBootstrapExchanged {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
        expires_in_seconds: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    PreviewConnectionAdmitted {
        id: u64,
        handoff_socket: String,
        handoff_token: u64,
        owner_generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        policy_refused: bool,
    },
    /// Response to RevokeExposure: whether it was declared.
    ExposureRevoked {
        id: u64,
        revoked: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to ListPublications.
    PublicationList {
        id: u64,
        generation: u64,
        publications: Vec<PublicationInfo>,
    },
    /// The terminal stream on this connection stopped; no more TerminalOutput
    /// follows. Sent instead of going silent when the client fell behind.
    TerminalStreamEnded { reason: String },
    /// Result of `ReloadConfig`. On success `active_profile_digest` is the
    /// digest of the exact active-profile bytes now enforced; on failure the
    /// previous policy stays in force and `error` says why.
    ConfigReloadResult {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_profile_digest: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Result of owner-side policy and primary-audit admission for an OCI pull.
    ContainerPullAdmission {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        policy_refused: bool,
    },
}

impl ServiceToProcess {
    /// The id a reply to this request carries, when the request has one.
    ///
    /// Requests without an id are either answered with `Pong` (`Ping`) or not
    /// answered on the connection at all.
    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Exec { id, .. }
            | Self::ExecStream { id, .. }
            | Self::WriteFile { id, .. }
            | Self::ReadFile { id, .. }
            | Self::LogFileBoundary { id, .. }
            | Self::CloneState { id, .. }
            | Self::McpListServers { id }
            | Self::McpListTools { id }
            | Self::McpRefreshTools { id }
            | Self::McpCallTool { id, .. }
            | Self::PublishPort { id, .. }
            | Self::DeclarePreview { id, .. }
            | Self::RevokeExposure { id, .. }
            | Self::ListPublications { id }
            | Self::CreatePreviewSession { id, .. }
            | Self::RevokePreviewSessions { id, .. }
            | Self::ExchangePreviewBootstrap { id, .. }
            | Self::AdmitPreviewConnection { id, .. }
            | Self::LinkAttach { id, .. }
            | Self::LinkDetach { id, .. }
            | Self::AdmitContainerPull { id, .. }
            | Self::ReloadConfig { id } => Some(*id),
            _ => None,
        }
    }
}

impl ProcessToService {
    /// The request id this message finally answers.
    ///
    /// Lifecycle broadcasts (`StateChanged`, `ShutdownRequested`, ...) reach
    /// every IPC connection and answer nothing; `ExecOutput` is a chunk of a
    /// streaming exec, not its reply. Correlate on this, never on arrival order.
    pub fn reply_id(&self) -> Option<u64> {
        match self {
            Self::ExecResult { id, .. }
            | Self::WriteFileResult { id, .. }
            | Self::ReadFileResult { id, .. }
            | Self::LogFileBoundaryResult { id, .. }
            | Self::CloneStateResult { id, .. }
            | Self::McpServersResult { id, .. }
            | Self::McpToolsResult { id, .. }
            | Self::McpRefreshResult { id, .. }
            | Self::McpCallToolResult { id, .. }
            | Self::PortPublished { id, .. }
            | Self::ExposureRevoked { id, .. }
            | Self::PublicationList { id, .. }
            | Self::PreviewSessionCreated { id, .. }
            | Self::PreviewSessionsRevoked { id, .. }
            | Self::PreviewBootstrapExchanged { id, .. }
            | Self::PreviewConnectionAdmitted { id, .. }
            | Self::LinkAttachResult { id, .. }
            | Self::LinkDetachResult { id, .. }
            | Self::ContainerPullAdmission { id, .. }
            | Self::ConfigReloadResult { id, .. } => Some(*id),
            _ => None,
        }
    }
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

/// One live publication as its VM owner declares it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PublicationInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    pub guest_port: u16,
    pub target: crate::PublicationTarget,
    pub access: crate::PublicationAccess,
    pub router_pid: u32,
}

/// Status of an MCP tool as reported through IPC.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpToolStatus {
    pub namespaced_name: String,
    pub original_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub server_name: String,
    /// Typed so SDK and UI consumers get one stable annotation contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<crate::mcp_contracts::ToolAnnotations>,
}

#[cfg(test)]
mod tests;
