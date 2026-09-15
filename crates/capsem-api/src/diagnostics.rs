use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct PersistResponse {
    pub success: bool,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct PanicEvent {
    pub ts: String,
    pub binary: String,
    pub thread: Option<String>,
    pub location: Option<String>,
    pub message: String,
    pub frames: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct ErrorEvent {
    pub ts: String,
    pub binary: String,
    pub level: String,
    pub target: Option<String>,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct SlowOpEvent {
    pub ts: String,
    pub binary: String,
    pub op: String,
    pub duration_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct PanicsResponse {
    pub panics: Vec<PanicEvent>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
pub struct HostTriageResponse {
    pub panics: Vec<PanicEvent>,
    pub errors: Vec<ErrorEvent>,
    pub slow_ops: Vec<SlowOpEvent>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct TriageResponse {
    pub since: String,
    pub session_id: Option<String>,
    pub host: HostTriageResponse,
    /// Session-ledger categories are extensible and retain their native JSON shape.
    pub session: serde_json::Value,
    pub rank: Vec<String>,
}

#[derive(Deserialize, Debug, Default, IntoParams)]
pub struct TriageQuery {
    /// Lookback window such as `30m`, `2h`, or an RFC3339 timestamp.
    pub since: Option<String>,
    /// Maximum events per category; the service caps this at 200.
    pub limit: Option<usize>,
    /// Optional VM identity used to include session-ledger diagnostics.
    pub id: Option<String>,
}
