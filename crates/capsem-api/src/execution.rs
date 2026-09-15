use crate::SandboxInfo;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, ToSchema)]
pub struct VmStatsSummaryResponse {
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub total_input_tokens: u64,
    pub total_thinking_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_calls: u64,
    pub total_estimated_cost: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ListResponse {
    pub sandboxes: Vec<SandboxInfo>,
}

/// Longest a single exec or run command may wait for its result, in seconds.
pub const MAX_EXEC_TIMEOUT_SECS: u64 = 60 * 60;

/// Exec timeout applied when a request does not name one. Absent used to mean
/// "wait forever"; it now means the ceiling.
pub const DEFAULT_EXEC_TIMEOUT_SECS: u64 = MAX_EXEC_TIMEOUT_SECS;
const _: () = assert!(DEFAULT_EXEC_TIMEOUT_SECS <= MAX_EXEC_TIMEOUT_SECS);

/// Resolve a request's exec timeout, refusing zero and values above the ceiling.
pub fn exec_timeout_secs(requested: Option<u64>) -> Result<u64, String> {
    match requested {
        None => Ok(DEFAULT_EXEC_TIMEOUT_SECS),
        Some(0) => Err("timeout_secs must be at least 1".to_string()),
        Some(secs) if secs > MAX_EXEC_TIMEOUT_SECS => {
            Err(format!("timeout_secs must be at most {MAX_EXEC_TIMEOUT_SECS}"))
        }
        Some(secs) => Ok(secs),
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecOutputEncoding {
    Utf8,
    Base64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ExecOutput {
    pub encoding: ExecOutputEncoding,
    pub data: String,
}

impl From<String> for ExecOutput {
    fn from(data: String) -> Self {
        Self {
            encoding: ExecOutputEncoding::Utf8,
            data,
        }
    }
}

impl From<&str> for ExecOutput {
    fn from(data: &str) -> Self {
        data.to_string().into()
    }
}

impl ExecOutput {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(data) => Self {
                encoding: ExecOutputEncoding::Utf8,
                data,
            },
            Err(error) => Self {
                encoding: ExecOutputEncoding::Base64,
                data: base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
            },
        }
    }

    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        match self.encoding {
            ExecOutputEncoding::Utf8 => Ok(self.data.as_bytes().to_vec()),
            ExecOutputEncoding::Base64 => base64::engine::general_purpose::STANDARD.decode(&self.data),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ExecResponse {
    pub stdout: ExecOutput,
    pub stderr: ExecOutput,
    pub exit_code: i32,
    /// The guest produced more output than the per-exec cap allows, so
    /// `stdout` is a prefix. Defaulted so an older client still decodes.
    #[serde(default)]
    pub truncated: bool,
}

impl ExecResponse {
    pub fn truncation_notice(&self) -> Option<&'static str> {
        self.truncated
            .then_some("capsem: guest output exceeded the capture limit; showing the retained prefix")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct RunRequest {
    pub command: String,
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Guest RAM in MiB. Falls back to the selected profile's VM resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// Guest CPU count. Falls back to the selected profile's VM resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    /// Environment variables to inject into the guest at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PersistRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PurgeRequest {
    #[serde(default)]
    pub all: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct PurgeResponse {
    pub purged: u32,
    pub persistent_purged: u32,
    pub ephemeral_purged: u32,
}
