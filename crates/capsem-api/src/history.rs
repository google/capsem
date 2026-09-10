use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryLayer {
    Exec,
    Audit,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryLayerFilter {
    #[default]
    All,
    Exec,
    Audit,
}

impl HistoryLayerFilter {
    pub fn includes(self, layer: HistoryLayer) -> bool {
        matches!(
            (self, layer),
            (Self::All, _) | (Self::Exec, HistoryLayer::Exec) | (Self::Audit, HistoryLayer::Audit)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecSource {
    Api,
    Cli,
    Mcp,
    Frontend,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExecHistoryDetails {
    pub source: ExecSource,
    pub trace_id: Option<String>,
    pub process_name: Option<String>,
    pub exec_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditHistoryDetails {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub comm: Option<String>,
    pub cwd: Option<String>,
    pub tty: Option<String>,
    pub session_id: Option<u32>,
    pub audit_id: Option<String>,
    pub parent_exe: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum HistoryDetails {
    Exec(ExecHistoryDetails),
    Audit(AuditHistoryDetails),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub layer: HistoryLayer,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub details: HistoryDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HistoryResponse {
    pub commands: Vec<HistoryEntry>,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryQuery {
    /// Maximum commands to return. Defaults to 500, capped at 2000.
    #[serde(default = "default_history_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    /// Literal, case-sensitive search over commands, previews and details.
    pub search: Option<String>,
    #[serde(default)]
    pub layer: HistoryLayerFilter,
}

fn default_history_limit() -> usize {
    500
}
