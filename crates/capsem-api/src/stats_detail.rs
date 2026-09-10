//! Bounded event detail returned by GET /vms/{id}/stats/detail.
//! Captured arguments, headers, bodies, and context retain their original text.
//! Provider/model identifiers, HTTP methods, and provider stop reasons are open namespaces.
use crate::{ExecSource, FileEventAction, ModelUsage, ToolDecision};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// Latest retained events (200 per model/tool/network/file family, 100 per
/// process/audit/credential family), session model totals, and captured bodies.
/// This is a bounded inspection report; it is not the complete session ledger.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VmStatsDetailResponse {
    pub interactions: crate::InteractionReport,
    pub model_stats: Vec<ModelUsage>,
    pub model_events: Vec<ModelEvent>,
    pub tool_events: Vec<ToolEvent>,
    pub http_events: Vec<HttpEvent>,
    pub dns_events: Vec<DnsEvent>,
    pub file_events: Vec<FileEvent>,
    pub process_events: Vec<ProcessEvent>,
    pub audit_events: Vec<AuditEvent>,
    pub credential_events: Vec<CredentialEvent>,
    /// Captured request/response bodies grouped by event ID.
    pub body_blobs: BTreeMap<String, Vec<EventBody>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelEvent {
    pub event_id: String,
    pub timestamp: String,
    pub provider: String,
    pub model: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: Option<u16>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub response_bytes: Option<u64>,
    pub stop_reason: Option<String>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolEvent {
    pub event_id: String,
    pub timestamp: Option<String>,
    pub process_name: Option<String>,
    pub server_name: String,
    pub tool_name: String,
    pub method: Option<String>,
    pub call_id: String,
    pub model_call_id: Option<i64>,
    pub model_parent_missing: bool,
    pub decision: ToolDecision,
    pub duration_ms: u64,
    pub bytes: u64,
    pub arguments: Option<String>,
    pub response_preview: Option<String>,
    pub error_message: Option<String>,
    pub source: ToolOrigin,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HttpEvent {
    pub event_id: String,
    pub timestamp: String,
    pub domain: String,
    pub port: Option<u16>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub decision: NetworkDecision,
    pub duration_ms: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub bytes_received: Option<u64>,
    pub matched_rule: Option<String>,
    pub policy_rule: Option<String>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DnsEvent {
    pub event_id: String,
    pub timestamp: String,
    pub qname: String,
    pub qtype: u16,
    pub qclass: u16,
    pub rcode: u16,
    pub decision: NetworkDecision,
    pub matched_rule: Option<String>,
    pub policy_rule: Option<String>,
    pub source_proto: Option<NetworkProtocol>,
    pub process_name: Option<String>,
    pub upstream_resolver_ms: Option<u64>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileEvent {
    pub event_id: String,
    pub timestamp: String,
    pub action: FileEventAction,
    pub path: String,
    pub size: Option<u64>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProcessEvent {
    pub event_id: String,
    pub timestamp: String,
    pub exec_id: u64,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout_bytes: Option<u64>,
    pub stderr_bytes: Option<u64>,
    pub source: ExecSource,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditEvent {
    pub event_id: String,
    pub timestamp: String,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub comm: Option<String>,
    pub argv: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub session_id: Option<u32>,
    pub tty: Option<String>,
    pub audit_id: Option<String>,
    pub exec_event_id: Option<i64>,
    pub parent_exe: Option<String>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CredentialEvent {
    pub event_id: String,
    pub timestamp: String,
    pub material_class: MaterialClass,
    pub source: String,
    pub event_type: Option<CredentialEventType>,
    pub origin: Option<CredentialEventType>,
    pub verb: CredentialOutcome,
    pub provider: Option<String>,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventBody {
    pub event_id: String,
    pub direction: BodyDirection,
    pub content_type: Option<String>,
    pub original_bytes: u64,
    pub stored_bytes: u64,
    pub truncated: bool,
    pub body_hash: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDecision {
    Allowed,
    Denied,
    Error,
    Redirected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    Model,
    Native,
    Mcp,
    Builtin,
    Local,
    McpProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BodyDirection {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaterialClass {
    Credential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOutcome {
    Captured,
    Brokered,
    Injected,
    Error,
}

/// Boundary that observed or injected credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum CredentialEventType {
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.response")]
    HttpResponse,
    #[serde(rename = "model.call")]
    ModelCall,
    #[serde(rename = "mcp.tool_call")]
    McpToolCall,
    #[serde(rename = "mcp.tool_list")]
    McpToolList,
    #[serde(rename = "mcp.event")]
    McpEvent,
    #[serde(rename = "dns.query")]
    DnsQuery,
    #[serde(rename = "file.event")]
    FileEvent,
    #[serde(rename = "file.import")]
    FileImport,
    #[serde(rename = "file.export")]
    FileExport,
    #[serde(rename = "process.exec")]
    ProcessExec,
    #[serde(rename = "process.exec_complete")]
    ProcessExecComplete,
    #[serde(rename = "process.audit")]
    ProcessAudit,
    #[serde(rename = "credential.substitution")]
    CredentialSubstitution,
    #[serde(rename = "security.rule")]
    SecurityRule,
    #[serde(rename = "security.ask")]
    SecurityAsk,
}
