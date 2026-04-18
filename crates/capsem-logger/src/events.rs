use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The outcome of a domain policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allowed,
    Denied,
    Error,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Denied => "denied",
            Decision::Error => "error",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "allowed" => Decision::Allowed,
            "denied" => Decision::Denied,
            "error" => Decision::Error,
            other => {
                tracing::warn!(value = other, "unknown decision string in DB, treating as Error");
                Decision::Error
            }
        }
    }
}

/// Serialize SystemTime as f64 epoch seconds (for frontend compatibility).
fn serialize_timestamp<S: serde::Serializer>(ts: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let epoch = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    s.serialize_f64(epoch.as_secs_f64())
}

/// Deserialize f64 epoch seconds back to SystemTime.
fn deserialize_timestamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let secs: f64 = serde::Deserialize::deserialize(d)?;
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs))
}

/// The type of filesystem action observed via inotify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Created,
    Modified,
    Deleted,
    Restored,
}

impl FileAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileAction::Created => "created",
            FileAction::Modified => "modified",
            FileAction::Deleted => "deleted",
            FileAction::Restored => "restored",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "created" => FileAction::Created,
            "modified" => FileAction::Modified,
            "deleted" => FileAction::Deleted,
            "restored" => FileAction::Restored,
            other => {
                tracing::warn!(value = other, "unknown file action string in DB, treating as Modified");
                FileAction::Modified
            }
        }
    }
}

/// A single filesystem event from the in-VM inotify watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub action: FileAction,
    pub path: String,
    pub size: Option<u64>,
}

/// A snapshot event (auto or manual) recorded for the stats UI.
/// Each row is self-contained: the fs_event range (start_fs_event_id, stop_fs_event_id]
/// lets the frontend compute per-snapshot file changes without directory walks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub slot: usize,
    pub origin: String,
    pub name: Option<String>,
    pub files_count: usize,
    pub start_fs_event_id: i64,
    pub stop_fs_event_id: i64,
}

/// A single network connection event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub domain: String,
    pub port: u16,
    pub decision: Decision,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub duration_ms: u64,
    pub matched_rule: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_body_preview: Option<String>,
    pub response_body_preview: Option<String>,
    pub conn_type: Option<String>,
}

/// A tool call emitted by the model in a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEntry {
    pub call_index: u32,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Option<String>,
    /// "native" (model built-in, executed in VM) or "mcp" (routed through MCP gateway).
    #[serde(default = "default_origin")]
    pub origin: String,
}

fn default_origin() -> String {
    "native".to_string()
}

/// A tool result sent back to the model in a subsequent request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseEntry {
    pub call_id: String,
    pub content_preview: Option<String>,
    pub is_error: bool,
}

/// A single MCP tool call event (one row per tools/call or tools/list request).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCall {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub server_name: String,
    pub method: String,
    pub tool_name: Option<String>,
    pub request_id: Option<String>,
    pub request_preview: Option<String>,
    pub response_preview: Option<String>,
    /// "allowed", "warned", "denied", "error"
    pub decision: String,
    pub duration_ms: u64,
    pub error_message: Option<String>,
    pub process_name: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

/// A denormalized AI model API call (one row per request+response cycle),
/// with nested tool data inserted into separate tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCall {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub provider: String,
    pub model: Option<String>,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub method: String,
    pub path: String,
    pub stream: bool,
    // Request metadata
    pub system_prompt_preview: Option<String>,
    pub messages_count: usize,
    pub tools_count: usize,
    pub request_bytes: u64,
    pub request_body_preview: Option<String>,
    // Response metadata
    pub message_id: Option<String>,
    pub status_code: Option<u16>,
    pub text_content: Option<String>,
    pub thinking_content: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_details: BTreeMap<String, u64>,
    pub duration_ms: u64,
    pub response_bytes: u64,
    // Cost estimate
    pub estimated_cost_usd: f64,
    // Trace grouping
    pub trace_id: Option<String>,
    // Nested tool data (inserted into separate tables)
    pub tool_calls: Vec<ToolCallEntry>,
    pub tool_responses: Vec<ToolResponseEntry>,
}

/// A structured exec command event (Layer 1: host-side recording of API-path commands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub exec_id: u64,
    pub command: String,
    /// Request origin: "mcp", "cli", "api", "frontend".
    pub source: String,
    pub mcp_call_id: Option<u64>,
    pub trace_id: Option<String>,
    pub process_name: Option<String>,
}

/// Completion data for a structured exec command (sent when GuestToHost::ExecDone arrives).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecEventComplete {
    pub exec_id: u64,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub pid: Option<u32>,
}

/// A kernel audit event (Layer 3: execve syscalls captured by auditd).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub comm: Option<String>,
    pub argv: String,
    pub cwd: Option<String>,
    pub tty: Option<String>,
    pub session_id: Option<u32>,
    pub audit_id: Option<String>,
    pub exec_event_id: Option<i64>,
    pub parent_exe: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn decision_roundtrip() {
        for decision in [Decision::Allowed, Decision::Denied, Decision::Error] {
            assert_eq!(Decision::parse_str(decision.as_str()), decision);
        }
    }

    #[test]
    fn decision_json_roundtrip() {
        let event = NetEvent {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            domain: "elie.net".to_string(),
            port: 443,
            decision: Decision::Allowed,
            process_name: None,
            pid: None,
            method: None,
            path: None,
            query: None,
            status_code: None,
            bytes_sent: 0,
            bytes_received: 0,
            duration_ms: 0,
            matched_rule: None,
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            conn_type: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: NetEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.decision, Decision::Allowed);
        assert_eq!(decoded.domain, "elie.net");
    }

    #[test]
    fn decision_unknown_str() {
        assert_eq!(Decision::parse_str("bogus"), Decision::Error);
        assert_eq!(Decision::parse_str(""), Decision::Error);
    }

    #[test]
    fn file_action_roundtrip() {
        for action in [FileAction::Created, FileAction::Modified, FileAction::Deleted] {
            assert_eq!(FileAction::parse_str(action.as_str()), action);
        }
    }

    #[test]
    fn file_action_unknown_str() {
        assert_eq!(FileAction::parse_str("bogus"), FileAction::Modified);
        assert_eq!(FileAction::parse_str(""), FileAction::Modified);
    }

    /// "error" must be an explicit match arm, not caught by the _ wildcard.
    /// This ensures adding future variants (e.g. Timeout) won't silently
    /// map their as_str() to Decision::Error via the catchall.
    #[test]
    fn decision_from_str_explicitly_matches_error() {
        // "error" should match explicitly, not via _ => Error.
        assert_eq!(Decision::parse_str("error"), Decision::Error);
        // Verify the roundtrip: as_str -> from_str for all variants.
        assert_eq!(Decision::parse_str("allowed"), Decision::Allowed);
        assert_eq!(Decision::parse_str("denied"), Decision::Denied);
        assert_eq!(Decision::parse_str("error"), Decision::Error);
    }
}
