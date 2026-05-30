use serde::{Deserialize, Serialize};

use crate::metrics::VmMetricsSnapshot;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSecurityRulesSnapshot {
    #[serde(default)]
    pub enforcement: Vec<RuntimeEnforcementRuleSnapshot>,
    #[serde(default)]
    pub detection: Vec<RuntimeDetectionRuleSnapshot>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRuleMatchSnapshot {
    pub rule_id: String,
    pub match_count: u64,
    #[serde(default)]
    pub last_matched_event: Option<String>,
    #[serde(default)]
    pub last_matched_unix_ms: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnforcementRuleSnapshot {
    pub id: String,
    #[serde(default)]
    pub pack_id: Option<String>,
    pub condition: String,
    pub decision: RuntimeSecurityDecisionAction,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDetectionRuleSnapshot {
    pub id: String,
    pub pack_id: String,
    #[serde(default)]
    pub sigma_id: Option<String>,
    pub title: String,
    pub condition: String,
    pub severity: RuntimeDetectionSeverity,
    pub confidence: RuntimeDetectionConfidence,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSecurityDecisionAction {
    Allow,
    Ask,
    Block,
    Rewrite,
    Throttle,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeDetectionSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeDetectionConfidence {
    Low,
    Medium,
    High,
}

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
    /// Request the process to reload its configuration from disk plus the
    /// service-owned runtime rule snapshot.
    ReloadConfig {
        runtime_rules: Option<RuntimeSecurityRulesSnapshot>,
    },
    /// Drain process-local runtime rule match deltas into the service registry.
    DrainRuntimeRuleMatches { id: u64 },
    /// Request the process's bounded live metrics snapshot.
    GetMetricsSnapshot { id: u64 },
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
    /// Response to DrainRuntimeRuleMatches.
    RuntimeRuleMatches {
        id: u64,
        matches: Vec<RuntimeRuleMatchSnapshot>,
    },
    /// Response to GetMetricsSnapshot.
    MetricsSnapshot {
        id: u64,
        snapshot: Box<VmMetricsSnapshot>,
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
    /// Deprecated compatibility frame. Guest-initiated shutdown is disabled.
    ShutdownRequested { id: String },
    /// Guest requested suspend (forwarded from capsem-sysutil via vsock:5004).
    SuspendRequested { id: String },
    /// Guest quiescence complete: filesystem frozen, safe to snapshot.
    SnapshotReady { id: String },
}

#[cfg(test)]
mod tests;
