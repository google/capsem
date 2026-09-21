use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Symbolic host log streams. Paths supplied by clients are never accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostLogSource {
    Service,
    Mcp,
    Gateway,
    Tray,
    App,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogQuery {
    /// Keep lines containing this literal substring, before applying tail.
    pub grep: Option<String>,
    /// Return the last matching lines. Zero returns no lines.
    pub tail: Option<usize>,
    /// Read at most this many bytes per stream, capped at 5 MiB.
    /// Defaults to 100 KiB for host logs and 5 MiB for VM logs.
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostLogsResponse {
    pub source: HostLogSource,
    pub text: String,
}
