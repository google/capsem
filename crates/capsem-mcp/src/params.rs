//! Tool parameter types for the capsem MCP server.
//!
//! `CreateParams` and `RunParams` carry the guest environment, which is where
//! callers put API keys. Their `Debug` is hand-written so a `?params` log
//! line names the keys and never the values: the derived form once wrote
//! `ANTHROPIC_API_KEY: "sk-..."` into `~/.capsem/run/mcp.log`.

use std::collections::HashMap;
use std::fmt;

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Environment as it may appear in a log line: every key, no value.
struct RedactedEnv<'a>(Option<&'a HashMap<String, String>>);

impl fmt::Debug for RedactedEnv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("None"),
            Some(env) => {
                let mut keys: Vec<&str> = env.keys().map(String::as_str).collect();
                keys.sort_unstable();
                f.debug_map()
                    .entries(keys.into_iter().map(|key| (key, "<redacted>")))
                    .finish()
            }
        }
    }
}

impl fmt::Debug for CreateParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateParams")
            .field("name", &self.name)
            .field("profile", &self.profile)
            .field("ram_mb", &self.ram_mb)
            .field("cpu_count", &self.cpu_count)
            .field("version", &self.version)
            .field("env", &RedactedEnv(self.env.as_ref()))
            .field("from", &self.from)
            .finish()
    }
}

impl fmt::Debug for RunParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunParams")
            .field("command", &self.command)
            .field("profile", &self.profile)
            .field("timeout", &self.timeout)
            .field("env", &RedactedEnv(self.env.as_ref()))
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct IdParams {
    pub(crate) id: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct CreateParams {
    /// Optional requested session name. If omitted, the service assigns a profile-scoped name.
    pub(crate) name: Option<String>,
    /// Profile to use. Defaults to code when omitted.
    pub(crate) profile: Option<String>,
    #[serde(rename = "ramMb")]
    #[schemars(rename = "ramMb")]
    pub(crate) ram_mb: Option<u64>,
    #[serde(rename = "cpuCount")]
    #[schemars(rename = "cpuCount")]
    pub(crate) cpu_count: Option<u32>,
    pub(crate) version: Option<String>,
    /// Environment variables to inject into the guest (e.g. {"API_KEY": "sk-..."})
    pub(crate) env: Option<HashMap<String, String>>,
    /// Clone state from an existing session. The new session inherits
    /// the source's disk state (workspace, rootfs overlay, session.db).
    pub(crate) from: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct ForkParams {
    /// ID or name of the session to fork
    pub(crate) id: String,
    /// Name for the new session
    pub(crate) name: String,
    /// Optional description
    pub(crate) description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct PersistParams {
    /// ID or name of the running session to save
    pub(crate) id: String,
    /// Name for the saved session
    pub(crate) name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct NameParams {
    /// Session name or id
    pub(crate) name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct PurgeParams {
    /// Set to true to purge every stopped/broken session the service considers purgeable
    #[serde(default)]
    pub(crate) all: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct RunParams {
    /// Shell command to execute
    pub(crate) command: String,
    /// Profile to use. Defaults to code when omitted.
    pub(crate) profile: Option<String>,
    /// Timeout in seconds (default 60)
    pub(crate) timeout: Option<u64>,
    /// Environment variables to inject into the guest at boot
    pub(crate) env: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct ExecParams {
    pub(crate) id: String,
    pub(crate) command: String,
    /// Timeout in seconds (default 30)
    pub(crate) timeout: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct FileReadParams {
    pub(crate) id: String,
    pub(crate) path: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct FileWriteParams {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct LogsParams {
    pub(crate) id: String,
    /// Case-insensitive substring filter applied to each log line
    pub(crate) grep: Option<String>,
    /// Return only the last N lines (applied after grep)
    pub(crate) tail: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct ServiceLogsParams {
    /// Case-insensitive substring filter applied to each log line
    pub(crate) grep: Option<String>,
    /// Return only the last N lines (applied after grep)
    pub(crate) tail: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct TriageMcpParams {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h",
    /// "7d", "300s", or RFC3339 ("2026-05-02T17:30:00Z").
    pub(crate) since: Option<String>,
    /// Max items per category. Default 20, max 200.
    pub(crate) limit: Option<u64>,
    /// Optional session id (reserved for the future session.db
    /// cross-reference; ignored today).
    pub(crate) id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct TimelineMcpParams {
    /// Session id.
    pub(crate) id: String,
    /// Filter to one trace_id (hex). Rows with NULL trace_id are also
    /// returned -- they pre-date W4's trace propagation but may still
    /// be relevant context.
    #[serde(rename = "traceId")]
    pub(crate) trace_id: Option<String>,
    /// Lookback window. Default "10m"; accepts "5m", "1h", "24h",
    /// "7d", "300s", or RFC3339.
    pub(crate) since: Option<String>,
    /// Max rows. Default 200, max 2000.
    pub(crate) limit: Option<u64>,
    /// Comma-separated subset of layers: "exec,tool,net,fs,model".
    /// Default all.
    pub(crate) layers: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct HostLogsMcpParams {
    /// One of: "service", "mcp", "gateway", "tray", "app".
    pub(crate) name: String,
    /// Substring filter applied per line.
    pub(crate) grep: Option<String>,
    /// Return only the last N lines (applied after grep).
    pub(crate) tail: Option<u64>,
    /// Max bytes to read from end of file. Default 100KB, max 5MB.
    #[serde(rename = "maxBytes")]
    pub(crate) max_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct McpToolsParams {
    /// Profile whose MCP configuration should be inspected
    pub(crate) profile: Option<String>,
    /// Filter tools by server name (optional)
    pub(crate) server: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct McpCallParams {
    /// Profile whose MCP tool should be called
    pub(crate) profile: Option<String>,
    /// Namespaced tool name (e.g. github__search_repos)
    pub(crate) name: String,
    /// JSON arguments for the tool call
    pub(crate) arguments: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
pub(crate) struct McpProfileParams {
    /// Profile whose MCP configuration should be inspected
    pub(crate) profile: Option<String>,
}
