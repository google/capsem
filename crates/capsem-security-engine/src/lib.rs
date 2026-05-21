use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use thiserror::Error;

pub const SECURITY_EVENT_SCHEMA_VERSION: u32 = 1;
pub const RESOLVED_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventFamily {
    Dns,
    Http,
    Mcp,
    Model,
    File,
    Process,
    Credential,
    Vm,
    Profile,
    Conversation,
    Snapshot,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RedactionState {
    #[default]
    Raw,
    Redacted,
    SummaryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEngine {
    Network,
    File,
    Process,
    Conversation,
    Security,
    Vm,
    Profile,
    HostAi,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiAttributionScope {
    Host,
    Vm,
    Profile,
    Session,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiOriginKind {
    GuestNetwork,
    HostService,
    HostAdmin,
    HostWorkbench,
    TestFixture,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforceability {
    InlineBlockable,
    ObserveOnly,
    RemediationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackStatus {
    Active,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityPackIdentity {
    pub id: String,
    pub revision: String,
    pub hash: String,
    pub signature: String,
    pub status: PackStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEventCommon {
    pub event_id: String,
    #[serde(default)]
    pub parent_event_id: Option<String>,
    #[serde(default)]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub activity_id: Option<String>,
    #[serde(default)]
    pub sequence_no: Option<u64>,
    pub source_engine: SourceEngine,
    #[serde(default)]
    pub attribution_scope: AiAttributionScope,
    #[serde(default)]
    pub origin_kind: AiOriginKind,
    #[serde(default)]
    pub accounting_owner: Option<String>,
    pub enforceability: Enforceability,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
    pub timestamp_unix_ms: u64,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub profile_pack_ids: Vec<String>,
    #[serde(default)]
    pub enforcement_packs: Vec<SecurityPackIdentity>,
    #[serde(default)]
    pub detection_packs: Vec<SecurityPackIdentity>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub process_id: Option<String>,
    #[serde(default)]
    pub parent_process_id: Option<String>,
    #[serde(default)]
    pub exec_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub mcp_call_id: Option<String>,
    pub event_type: String,
    #[serde(default)]
    pub redaction_state: RedactionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEvent {
    pub schema_version: u32,
    pub common: SecurityEventCommon,
    pub subject: SecurityEventSubject,
    #[serde(default)]
    pub context: EventContext,
    #[serde(default)]
    pub trace: TraceSnapshot,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub findings: Vec<DetectionFinding>,
    #[serde(default)]
    pub decision: Option<SecurityDecision>,
    #[serde(default)]
    pub mutations: Vec<EventMutation>,
}

impl SecurityEvent {
    pub fn dns(common: SecurityEventCommon, subject: DnsSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Dns(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn http(common: SecurityEventCommon, subject: HttpSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Http(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn mcp(common: SecurityEventCommon, subject: McpSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Mcp(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn model(common: SecurityEventCommon, subject: ModelSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Model(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn file(common: SecurityEventCommon, subject: FileSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::File(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn process(common: SecurityEventCommon, subject: ProcessSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Process(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn conversation(common: SecurityEventCommon, subject: ConversationSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Conversation(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn snapshot(common: SecurityEventCommon, subject: SnapshotSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Snapshot(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn vm_lifecycle(common: SecurityEventCommon, subject: VmLifecycleSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::VmLifecycle(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn profile(common: SecurityEventCommon, subject: ProfileSecuritySubject) -> Self {
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Profile(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn event_family(&self) -> EventFamily {
        self.subject.event_family()
    }

    pub fn quota_dimensions(&self) -> QuotaDimensions {
        let mut dimensions = QuotaDimensions {
            profile_id: self.common.profile_id.clone(),
            profile_revision: self.common.profile_revision.clone(),
            vm_id: self.common.vm_id.clone(),
            session_id: self.common.session_id.clone(),
            user_id: self.common.user_id.clone(),
            source_engine: self.common.source_engine,
            attribution_scope: self.common.attribution_scope,
            origin_kind: self.common.origin_kind,
            accounting_owner: self.common.accounting_owner.clone(),
            event_family: self.event_family(),
            event_type: self.common.event_type.clone(),
            correlation_ids: CorrelationIds {
                trace_id: self.common.trace_id.clone(),
                span_id: self.common.span_id.clone(),
                parent_event_id: self.common.parent_event_id.clone(),
                stream_id: self.common.stream_id.clone(),
                activity_id: self.common.activity_id.clone(),
                sequence_no: self.common.sequence_no,
                process_id: self.common.process_id.clone(),
                exec_id: self.common.exec_id.clone(),
                turn_id: self.common.turn_id.clone(),
                message_id: self.common.message_id.clone(),
                tool_call_id: self.common.tool_call_id.clone(),
                mcp_call_id: self.common.mcp_call_id.clone(),
            },
            ..QuotaDimensions::default_for(self.event_family(), self.common.event_type.clone())
        };
        self.subject.apply_quota_dimensions(&mut dimensions);
        dimensions
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventContext {
    #[serde(default)]
    pub history: Vec<TraceHistoryEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSnapshot {
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub history: Vec<TraceHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceHistoryEntry {
    pub event_id: String,
    pub event_type: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityDecision {
    pub action: SecurityDecisionAction,
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityDecisionAction {
    Allow,
    Ask,
    Block,
    Rewrite,
    Throttle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EventMutation {
    ReplaceRegex {
        path: String,
        pattern: String,
        replacement: String,
        #[serde(default)]
        reason: Option<String>,
    },
    StripHeader {
        path: String,
        #[serde(default)]
        reason: Option<String>,
    },
}

impl EventMutation {
    pub fn path(&self) -> &str {
        match self {
            Self::ReplaceRegex { path, .. } | Self::StripHeader { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum SecurityEventSubject {
    Dns(DnsSecuritySubject),
    Http(HttpSecuritySubject),
    Mcp(McpSecuritySubject),
    Model(ModelSecuritySubject),
    File(FileSecuritySubject),
    Process(ProcessSecuritySubject),
    Credential(CredentialSecuritySubject),
    VmLifecycle(VmLifecycleSecuritySubject),
    Profile(ProfileSecuritySubject),
    Conversation(ConversationSecuritySubject),
    Snapshot(SnapshotSecuritySubject),
}

impl SecurityEventSubject {
    pub fn event_family(&self) -> EventFamily {
        match self {
            Self::Dns(_) => EventFamily::Dns,
            Self::Http(_) => EventFamily::Http,
            Self::Mcp(_) => EventFamily::Mcp,
            Self::Model(_) => EventFamily::Model,
            Self::File(_) => EventFamily::File,
            Self::Process(_) => EventFamily::Process,
            Self::Credential(_) => EventFamily::Credential,
            Self::VmLifecycle(_) => EventFamily::Vm,
            Self::Profile(_) => EventFamily::Profile,
            Self::Conversation(_) => EventFamily::Conversation,
            Self::Snapshot(_) => EventFamily::Snapshot,
        }
    }

    fn apply_quota_dimensions(&self, dimensions: &mut QuotaDimensions) {
        match self {
            Self::Dns(subject) => {
                dimensions.dns_domain_class = Some(subject.domain_class.clone());
            }
            Self::Http(subject) => {
                dimensions.http_host = Some(subject.host.clone());
                dimensions.http_method = Some(subject.method.clone());
                dimensions.http_path_class = Some(subject.path_class.clone());
                dimensions.request_bytes = Some(subject.request_bytes);
                dimensions.response_bytes = subject.response_bytes;
            }
            Self::Mcp(subject) => {
                dimensions.mcp_server = Some(subject.server_id.clone());
                dimensions.mcp_tool = Some(subject.tool_name.clone());
                if let Some(evidence) = subject.evidence.as_deref() {
                    dimensions.mcp_link_status = Some(evidence.link_status);
                    dimensions.linked_model_interaction_id =
                        evidence.linked_model_interaction_id.clone();
                    dimensions.linked_model_tool_call_id =
                        evidence.linked_model_tool_call_id.clone();
                }
            }
            Self::Model(subject) => {
                dimensions.provider = Some(subject.provider.clone());
                dimensions.model = Some(subject.model.clone());
                dimensions.estimated_input_tokens = subject.estimated_input_tokens;
                dimensions.estimated_output_tokens = subject.estimated_output_tokens;
                dimensions.estimated_cost_micros = subject.estimated_cost_micros;
                if let Some(evidence) = subject.evidence.as_deref() {
                    dimensions.ai_api_family = Some(evidence.api_family);
                    dimensions.evidence_parse_status = Some(evidence.parse_status);
                    dimensions.evidence_status = Some(evidence.evidence_status);
                    dimensions.model_tool_call_count = Some(evidence.tool_calls.len() as u64);
                    dimensions.model_tool_result_count = Some(evidence.tool_results.len() as u64);
                    dimensions.model_mcp_execution_count =
                        Some(evidence.mcp_executions.len() as u64);
                    dimensions.model_linked_mcp_tool_call_count = Some(
                        evidence
                            .tool_calls
                            .iter()
                            .filter(|tool_call| tool_call.linked_mcp_call_id.is_some())
                            .count() as u64,
                    );
                }
            }
            Self::File(_)
            | Self::Process(_)
            | Self::Credential(_)
            | Self::VmLifecycle(_)
            | Self::Profile(_)
            | Self::Conversation(_)
            | Self::Snapshot(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsSecuritySubject {
    pub qname: String,
    pub domain_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSecuritySubject {
    pub method: String,
    pub host: String,
    pub path_class: String,
    pub request_bytes: u64,
    #[serde(default)]
    pub response_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSecuritySubject {
    pub server_id: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<McpToolExecutionEvidence>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSecuritySubject {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub estimated_input_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<ModelInteractionEvidence>>,
}

impl ModelSecuritySubject {
    pub fn from_interaction_evidence(evidence: ModelInteractionEvidence) -> Self {
        Self {
            provider: evidence.provider.as_str().to_owned(),
            model: evidence.model.clone(),
            estimated_input_tokens: evidence.usage.input_tokens,
            estimated_output_tokens: evidence.usage.output_tokens,
            estimated_cost_micros: evidence.usage.estimated_cost_micros,
            evidence: Some(Box::new(evidence)),
        }
    }
}

impl McpSecuritySubject {
    pub fn from_execution_evidence(evidence: McpToolExecutionEvidence) -> Self {
        Self {
            server_id: evidence.server_id.clone(),
            tool_name: evidence.tool_name.clone(),
            evidence: Some(Box::new(evidence)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSecuritySubject {
    pub operation: String,
    pub path_class: String,
    #[serde(default)]
    pub byte_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSecuritySubject {
    pub operation: String,
    #[serde(default)]
    pub command_class: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSecuritySubject {
    pub operation: String,
    pub credential_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmLifecycleSecuritySubject {
    pub operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSecuritySubject {
    pub operation: String,
    pub profile_id: String,
    pub profile_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSecuritySubject {
    pub operation: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSecuritySubject {
    pub operation: String,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Openai,
    Anthropic,
    GoogleGemini,
    Unknown,
}

impl AiProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::GoogleGemini => "google_gemini",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiApiFamily {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
    GoogleGeminiContent,
    Mcp,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentsStatus {
    ValidJson,
    PartialJson,
    MalformedJson,
    NotJson,
    Redacted,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Complete,
    Partial,
    Malformed,
    Unsupported,
    Redacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Complete,
    Partial,
    Ambiguous,
    Orphaned,
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    NativeProviderTool,
    McpTool,
    LocalBuiltinTool,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Linked,
    UnlinkedPending,
    OrphanModelToolCall,
    OrphanMcpExecution,
    Ambiguous,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Proposed,
    Executed,
    Blocked,
    ReturnedToModel,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInteractionEvidence {
    pub interaction_id: String,
    pub trace_id: String,
    pub attribution_scope: AiAttributionScope,
    pub source_engine: SourceEngine,
    pub origin_kind: AiOriginKind,
    #[serde(default)]
    pub accounting_owner: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    pub provider: AiProvider,
    pub api_family: AiApiFamily,
    pub model: String,
    pub request: ModelRequestEvidence,
    #[serde(default)]
    pub response: Option<ModelResponseEvidence>,
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCallEvidence>,
    #[serde(default)]
    pub tool_results: Vec<ModelToolResultEvidence>,
    #[serde(default)]
    pub mcp_executions: Vec<McpToolExecutionEvidence>,
    #[serde(default)]
    pub usage: AiUsageEvidence,
    pub parse_status: ParseStatus,
    pub evidence_status: EvidenceStatus,
}

impl ModelInteractionEvidence {
    pub fn charges_vm_accounting(&self) -> bool {
        self.attribution_scope == AiAttributionScope::Vm && self.vm_id.is_some()
    }

    pub fn charges_host_accounting(&self) -> bool {
        self.attribution_scope == AiAttributionScope::Host
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestEvidence {
    pub request_id: String,
    pub provider: AiProvider,
    pub api_family: AiApiFamily,
    #[serde(default)]
    pub model: Option<String>,
    pub stream: bool,
    #[serde(default)]
    pub system_prompt_preview: Option<String>,
    pub message_count: u64,
    pub tools_declared_count: u64,
    pub raw_shape_version: String,
    pub unknown_fields_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResponseEvidence {
    pub response_id: String,
    #[serde(default)]
    pub provider_response_id: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub text_preview: Option<String>,
    #[serde(default)]
    pub thinking_preview: Option<String>,
    #[serde(default)]
    pub content_blocks: Vec<AiContentBlock>,
    #[serde(default)]
    pub usage: AiUsageEvidence,
    pub raw_shape_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiUsageEvidence {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
    #[serde(default)]
    pub details: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolCallEvidence {
    pub tool_call_id: String,
    pub index: u64,
    #[serde(default)]
    pub provider_call_id: Option<String>,
    pub raw_name: String,
    pub normalized_name: String,
    #[serde(default)]
    pub arguments_raw: Option<String>,
    #[serde(default)]
    pub arguments_json: Option<String>,
    pub arguments_status: ArgumentsStatus,
    pub origin: ToolOrigin,
    #[serde(default)]
    pub linked_mcp_call_id: Option<String>,
    pub status: ToolCallStatus,
    pub parse_confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolResultEvidence {
    pub tool_call_id: String,
    #[serde(default)]
    pub linked_mcp_call_id: Option<String>,
    pub content_kind: AiContentKind,
    #[serde(default)]
    pub content_preview: Option<String>,
    #[serde(default)]
    pub content_json: Option<String>,
    pub is_error: bool,
    pub result_status: ToolCallStatus,
    pub returned_to_model: bool,
    pub parse_confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolExecutionEvidence {
    pub mcp_call_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub namespaced_tool_name: String,
    pub transport: String,
    #[serde(default)]
    pub request_arguments_raw: Option<String>,
    #[serde(default)]
    pub request_arguments_json: Option<String>,
    pub result_kind: AiContentKind,
    #[serde(default)]
    pub result_preview: Option<String>,
    #[serde(default)]
    pub result_json: Option<String>,
    pub is_error: bool,
    pub latency_ms: u64,
    #[serde(default)]
    pub linked_model_interaction_id: Option<String>,
    #[serde(default)]
    pub linked_model_tool_call_id: Option<String>,
    pub link_status: LinkStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiContentKind {
    Text,
    Json,
    Image,
    File,
    ToolUse,
    ToolResult,
    Reasoning,
    CacheMarker,
    Redacted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiContentBlock {
    Text {
        text_preview: String,
    },
    Json {
        json_preview: String,
    },
    Image {
        mime_type: String,
        #[serde(default)]
        redacted: bool,
    },
    File {
        file_name: String,
        path_class: String,
    },
    ToolUse {
        tool_call_id: String,
        name: String,
    },
    ToolResult {
        tool_call_id: String,
        is_error: bool,
    },
    Reasoning {
        text_preview: String,
    },
    CacheMarker {
        marker: String,
    },
    Redacted {
        reason: String,
    },
    Unknown {
        #[serde(default)]
        raw_type: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationIds {
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
    #[serde(default)]
    pub parent_event_id: Option<String>,
    #[serde(default)]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub activity_id: Option<String>,
    #[serde(default)]
    pub sequence_no: Option<u64>,
    #[serde(default)]
    pub process_id: Option<String>,
    #[serde(default)]
    pub exec_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub mcp_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaDimensions {
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    pub source_engine: SourceEngine,
    pub attribution_scope: AiAttributionScope,
    pub origin_kind: AiOriginKind,
    #[serde(default)]
    pub accounting_owner: Option<String>,
    pub event_family: EventFamily,
    pub event_type: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub ai_api_family: Option<AiApiFamily>,
    #[serde(default)]
    pub evidence_parse_status: Option<ParseStatus>,
    #[serde(default)]
    pub evidence_status: Option<EvidenceStatus>,
    #[serde(default)]
    pub model_tool_call_count: Option<u64>,
    #[serde(default)]
    pub model_tool_result_count: Option<u64>,
    #[serde(default)]
    pub model_mcp_execution_count: Option<u64>,
    #[serde(default)]
    pub model_linked_mcp_tool_call_count: Option<u64>,
    #[serde(default)]
    pub mcp_server: Option<String>,
    #[serde(default)]
    pub mcp_tool: Option<String>,
    #[serde(default)]
    pub mcp_link_status: Option<LinkStatus>,
    #[serde(default)]
    pub linked_model_interaction_id: Option<String>,
    #[serde(default)]
    pub linked_model_tool_call_id: Option<String>,
    #[serde(default)]
    pub http_host: Option<String>,
    #[serde(default)]
    pub http_method: Option<String>,
    #[serde(default)]
    pub http_path_class: Option<String>,
    #[serde(default)]
    pub dns_domain_class: Option<String>,
    #[serde(default)]
    pub estimated_input_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
    #[serde(default)]
    pub request_bytes: Option<u64>,
    #[serde(default)]
    pub response_bytes: Option<u64>,
    pub correlation_ids: CorrelationIds,
}

impl QuotaDimensions {
    fn default_for(event_family: EventFamily, event_type: String) -> Self {
        Self {
            profile_id: None,
            profile_revision: None,
            vm_id: None,
            session_id: None,
            user_id: None,
            source_engine: SourceEngine::Security,
            attribution_scope: AiAttributionScope::Unknown,
            origin_kind: AiOriginKind::Unknown,
            accounting_owner: None,
            event_family,
            event_type,
            provider: None,
            model: None,
            ai_api_family: None,
            evidence_parse_status: None,
            evidence_status: None,
            model_tool_call_count: None,
            model_tool_result_count: None,
            model_mcp_execution_count: None,
            model_linked_mcp_tool_call_count: None,
            mcp_server: None,
            mcp_tool: None,
            mcp_link_status: None,
            linked_model_interaction_id: None,
            linked_model_tool_call_id: None,
            http_host: None,
            http_method: None,
            http_path_class: None,
            dns_domain_class: None,
            estimated_input_tokens: None,
            estimated_output_tokens: None,
            estimated_cost_micros: None,
            request_bytes: None,
            response_bytes: None,
            correlation_ids: CorrelationIds::default(),
        }
    }

    pub fn charges_vm_accounting(&self) -> bool {
        self.attribution_scope == AiAttributionScope::Vm && self.vm_id.is_some()
    }

    pub fn charges_host_accounting(&self) -> bool {
        self.attribution_scope == AiAttributionScope::Host
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityResult {
    pub event_id: String,
    pub action: SecurityAction,
    pub resolved_event: ResolvedSecurityEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSecurityEvent {
    pub schema_version: u32,
    pub event: SecurityEvent,
    #[serde(default)]
    pub steps: Vec<ResolvedEventStep>,
    #[serde(default)]
    pub plugin_transforms: Vec<PluginTransformRecord>,
    #[serde(default)]
    pub detection_findings: Vec<DetectionFinding>,
    pub final_action: SecurityAction,
    #[serde(default)]
    pub emitter_results: Vec<EmitterResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedEventStep {
    pub kind: ResolvedEventStepKind,
    pub status: StepStatus,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedEventStepKind {
    Preprocessor,
    PluginCallback,
    EnforcementMatch,
    Confirm,
    RateLimitCheck,
    DetectionMatch,
    Postprocessor,
    EmitterDelivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Applied,
    Matched,
    Skipped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "detail", rename_all = "snake_case")]
pub enum SecurityAction {
    Continue,
    Ask(AskPlan),
    Rewrite(RewritePatch),
    Block(BlockResponse),
    Throttle(ThrottlePlan),
    Quarantine(QuarantinePlan),
    Restore(RestorePlan),
    DropConnection(DropReason),
    ObserveOnly,
    Error(SecurityError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskPlan {
    pub prompt_id: String,
    pub reason_code: String,
    pub default_action: Box<SecurityAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewritePatch {
    pub target: String,
    pub replacement_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockResponse {
    pub reason_code: String,
    #[serde(default)]
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThrottlePlan {
    pub delay_ms: u64,
    pub quota_id: String,
    pub scope: String,
    pub reason_code: String,
    #[serde(default)]
    pub provider_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinePlan {
    pub path_class: String,
    pub quarantine_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePlan {
    pub snapshot_id: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropReason {
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionFinding {
    pub finding_id: String,
    pub event_id: String,
    pub rule_id: String,
    pub pack_id: String,
    #[serde(default)]
    pub sigma_id: Option<String>,
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterResult {
    pub sink: String,
    pub status: StepStatus,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkRequirement {
    Required,
    BestEffort,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct EmitterError {
    message: String,
}

impl EmitterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait ResolvedEventSink {
    fn name(&self) -> &str;
    fn requirement(&self) -> SinkRequirement;
    fn emit(&mut self, event: &ResolvedSecurityEvent) -> Result<(), EmitterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkDelivery {
    pub sink: String,
    pub event_id: String,
    pub finding_ids: Vec<String>,
}

#[derive(Default)]
pub struct ResolvedEventEmitter {
    sinks: Vec<Box<dyn ResolvedEventSink>>,
    deliveries: Vec<SinkDelivery>,
}

impl ResolvedEventEmitter {
    pub fn add_sink(&mut self, sink: Box<dyn ResolvedEventSink>) {
        self.sinks.push(sink);
    }

    pub fn emit(&mut self, mut event: ResolvedSecurityEvent) -> EmitOutcome {
        event.emitter_results.clear();
        let mut required_sink_failed = false;
        for sink in &mut self.sinks {
            let sink_name = sink.name().to_owned();
            match sink.emit(&event) {
                Ok(()) => {
                    self.deliveries.push(SinkDelivery {
                        sink: sink_name.clone(),
                        event_id: event.event.common.event_id.clone(),
                        finding_ids: event
                            .detection_findings
                            .iter()
                            .map(|finding| finding.finding_id.clone())
                            .collect(),
                    });
                    event.emitter_results.push(EmitterResult {
                        sink: sink_name,
                        status: StepStatus::Applied,
                        error: None,
                    });
                }
                Err(error) => {
                    if sink.requirement() == SinkRequirement::Required {
                        required_sink_failed = true;
                    }
                    event.emitter_results.push(EmitterResult {
                        sink: sink_name,
                        status: StepStatus::Error,
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        EmitOutcome {
            resolved_event: event,
            required_sink_failed,
        }
    }

    pub fn deliveries(&self) -> &[SinkDelivery] {
        &self.deliveries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitOutcome {
    pub resolved_event: ResolvedSecurityEvent,
    pub required_sink_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEnginePhase {
    Preprocessor,
    Enforcement,
    Confirm,
    Detection,
    Postprocessor,
}

impl SecurityEnginePhase {
    fn step_kind(self) -> ResolvedEventStepKind {
        match self {
            Self::Preprocessor => ResolvedEventStepKind::Preprocessor,
            Self::Enforcement => ResolvedEventStepKind::EnforcementMatch,
            Self::Confirm => ResolvedEventStepKind::Confirm,
            Self::Detection => ResolvedEventStepKind::DetectionMatch,
            Self::Postprocessor => ResolvedEventStepKind::Postprocessor,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Preprocessor => "preprocessor_failed",
            Self::Enforcement => "enforcement_failed",
            Self::Confirm => "confirm_failed",
            Self::Detection => "detection_failed",
            Self::Postprocessor => "postprocessor_failed",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SecurityEngineError {
    #[error("{phase:?} phase failed: {message}")]
    PhaseFailed {
        phase: SecurityEnginePhase,
        message: String,
    },
    #[error("rule {rule_id} CEL compile failed: {message}")]
    CelCompileFailed { rule_id: String, message: String },
    #[error("rule {rule_id} CEL evaluation failed: {message}")]
    CelEvaluationFailed { rule_id: String, message: String },
    #[error("rule {rule_id} CEL result was not boolean: {actual}")]
    CelNonBooleanResult { rule_id: String, actual: String },
}

pub trait SecurityEventProcessor {
    fn name(&self) -> &str;
    fn process(&mut self, event: SecurityEvent) -> Result<SecurityEvent, SecurityEngineError>;
}

pub trait EnforcementEvaluator {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Option<SecurityDecision>, SecurityEngineError>;
}

pub trait ConfirmResolver {
    fn resolve(
        &mut self,
        event: &SecurityEvent,
        decision: &SecurityDecision,
    ) -> Result<SecurityDecision, SecurityEngineError>;
}

pub trait DetectionEvaluator {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Vec<DetectionFinding>, SecurityEngineError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelEnforcementRule {
    pub id: String,
    #[serde(default)]
    pub pack_id: Option<String>,
    pub condition: String,
    pub decision: SecurityDecisionAction,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug)]
pub struct CelEnforcementEvaluator {
    rules: Vec<CompiledCelEnforcementRule>,
}

#[derive(Debug)]
struct CompiledCelEnforcementRule {
    rule: CelEnforcementRule,
    program: cel::Program,
}

impl CelEnforcementEvaluator {
    pub fn compile(rules: Vec<CelEnforcementRule>) -> Result<Self, SecurityEngineError> {
        let mut compiled_rules = Vec::with_capacity(rules.len());
        for rule in rules {
            let program = cel::Program::compile(&rule.condition).map_err(|error| {
                SecurityEngineError::CelCompileFailed {
                    rule_id: rule.id.clone(),
                    message: error.to_string(),
                }
            })?;
            compiled_rules.push(CompiledCelEnforcementRule { rule, program });
        }
        Ok(Self {
            rules: compiled_rules,
        })
    }
}

impl EnforcementEvaluator for CelEnforcementEvaluator {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Option<SecurityDecision>, SecurityEngineError> {
        for compiled in &self.rules {
            if compiled.evaluate(event)? {
                return Ok(Some(SecurityDecision {
                    action: compiled.rule.decision,
                    rule: Some(compiled.rule.id.clone()),
                    pack_id: compiled.rule.pack_id.clone(),
                    reason: compiled.rule.reason.clone(),
                    terminal: compiled.rule.decision != SecurityDecisionAction::Allow,
                }));
            }
        }
        Ok(None)
    }
}

impl CompiledCelEnforcementRule {
    fn evaluate(&self, event: &SecurityEvent) -> Result<bool, SecurityEngineError> {
        evaluate_cel_bool(&self.rule.id, &self.program, event)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelDetectionRule {
    pub id: String,
    pub pack_id: String,
    #[serde(default)]
    pub sigma_id: Option<String>,
    pub title: String,
    pub condition: String,
    pub severity: Severity,
    pub confidence: Confidence,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug)]
pub struct CelDetectionEvaluator {
    rules: Vec<CompiledCelDetectionRule>,
}

#[derive(Debug)]
struct CompiledCelDetectionRule {
    rule: CelDetectionRule,
    program: cel::Program,
}

impl CelDetectionEvaluator {
    pub fn compile(rules: Vec<CelDetectionRule>) -> Result<Self, SecurityEngineError> {
        let mut compiled_rules = Vec::with_capacity(rules.len());
        for rule in rules {
            let program = cel::Program::compile(&rule.condition).map_err(|error| {
                SecurityEngineError::CelCompileFailed {
                    rule_id: rule.id.clone(),
                    message: error.to_string(),
                }
            })?;
            compiled_rules.push(CompiledCelDetectionRule { rule, program });
        }
        Ok(Self {
            rules: compiled_rules,
        })
    }
}

impl DetectionEvaluator for CelDetectionEvaluator {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Vec<DetectionFinding>, SecurityEngineError> {
        let mut findings = Vec::new();
        for compiled in &self.rules {
            if evaluate_cel_bool(&compiled.rule.id, &compiled.program, event)? {
                findings.push(DetectionFinding {
                    finding_id: format!("finding-{}-{}", event.common.event_id, compiled.rule.id),
                    event_id: event.common.event_id.clone(),
                    rule_id: compiled.rule.id.clone(),
                    pack_id: compiled.rule.pack_id.clone(),
                    sigma_id: compiled.rule.sigma_id.clone(),
                    title: compiled.rule.title.clone(),
                    severity: compiled.rule.severity,
                    confidence: compiled.rule.confidence,
                    tags: compiled.rule.tags.clone(),
                });
            }
        }
        Ok(findings)
    }
}

fn evaluate_cel_bool(
    rule_id: &str,
    program: &cel::Program,
    event: &SecurityEvent,
) -> Result<bool, SecurityEngineError> {
    let mut context = cel::Context::default();
    let event_value =
        cel::to_value(event).map_err(|error| SecurityEngineError::CelEvaluationFailed {
            rule_id: rule_id.to_owned(),
            message: error.to_string(),
        })?;
    context
        .add_variable("event", event_value)
        .map_err(|error| SecurityEngineError::CelEvaluationFailed {
            rule_id: rule_id.to_owned(),
            message: error.to_string(),
        })?;

    match program
        .execute(&context)
        .map_err(|error| SecurityEngineError::CelEvaluationFailed {
            rule_id: rule_id.to_owned(),
            message: error.to_string(),
        })? {
        cel::Value::Bool(value) => Ok(value),
        value => Err(SecurityEngineError::CelNonBooleanResult {
            rule_id: rule_id.to_owned(),
            actual: format!("{value:?}"),
        }),
    }
}

#[derive(Default)]
pub struct SecurityEngine {
    preprocessors: Vec<Box<dyn SecurityEventProcessor>>,
    enforcement: Option<Box<dyn EnforcementEvaluator>>,
    confirm: Option<Box<dyn ConfirmResolver>>,
    detection: Option<Box<dyn DetectionEvaluator>>,
    postprocessors: Vec<Box<dyn SecurityEventProcessor>>,
}

impl SecurityEngine {
    pub fn add_preprocessor(&mut self, processor: Box<dyn SecurityEventProcessor>) {
        self.preprocessors.push(processor);
    }

    pub fn set_enforcement(&mut self, enforcement: Box<dyn EnforcementEvaluator>) {
        self.enforcement = Some(enforcement);
    }

    pub fn set_confirm(&mut self, confirm: Box<dyn ConfirmResolver>) {
        self.confirm = Some(confirm);
    }

    pub fn set_detection(&mut self, detection: Box<dyn DetectionEvaluator>) {
        self.detection = Some(detection);
    }

    pub fn add_postprocessor(&mut self, processor: Box<dyn SecurityEventProcessor>) {
        self.postprocessors.push(processor);
    }

    pub fn evaluate(
        &mut self,
        mut event: SecurityEvent,
    ) -> Result<SecurityResult, SecurityEngineError> {
        let mut steps = Vec::new();

        for processor in &mut self.preprocessors {
            match processor.process(event.clone()) {
                Ok(next_event) => {
                    event = next_event;
                    steps.push(phase_step(
                        SecurityEnginePhase::Preprocessor,
                        StepStatus::Applied,
                        None,
                        None,
                        Some(format!("{} applied", processor.name())),
                    ));
                }
                Err(error) => {
                    return Ok(error_result(
                        event,
                        steps,
                        SecurityEnginePhase::Preprocessor,
                        error,
                    ));
                }
            }
        }

        if let Some(enforcement) = &mut self.enforcement {
            match enforcement.evaluate(&event) {
                Ok(Some(decision)) => {
                    steps.push(phase_step(
                        SecurityEnginePhase::Enforcement,
                        StepStatus::Matched,
                        decision.rule.clone(),
                        decision.pack_id.clone(),
                        decision.reason.clone(),
                    ));
                    event.decision = Some(decision);
                }
                Ok(None) => {
                    steps.push(phase_step(
                        SecurityEnginePhase::Enforcement,
                        StepStatus::Skipped,
                        None,
                        None,
                        None,
                    ));
                }
                Err(error) => {
                    return Ok(error_result(
                        event,
                        steps,
                        SecurityEnginePhase::Enforcement,
                        error,
                    ));
                }
            }
        }

        if event
            .decision
            .as_ref()
            .is_some_and(|decision| decision.action == SecurityDecisionAction::Ask)
        {
            if let Some(confirm) = &mut self.confirm {
                let ask_decision = event.decision.clone().expect("decision checked above");
                match confirm.resolve(&event, &ask_decision) {
                    Ok(resolved_decision) => {
                        steps.push(phase_step(
                            SecurityEnginePhase::Confirm,
                            StepStatus::Applied,
                            resolved_decision.rule.clone(),
                            resolved_decision.pack_id.clone(),
                            resolved_decision.reason.clone(),
                        ));
                        event.decision = Some(resolved_decision);
                    }
                    Err(error) => {
                        return Ok(error_result(
                            event,
                            steps,
                            SecurityEnginePhase::Confirm,
                            error,
                        ));
                    }
                }
            } else {
                steps.push(phase_step(
                    SecurityEnginePhase::Confirm,
                    StepStatus::Skipped,
                    event
                        .decision
                        .as_ref()
                        .and_then(|decision| decision.rule.clone()),
                    event
                        .decision
                        .as_ref()
                        .and_then(|decision| decision.pack_id.clone()),
                    Some("no confirm resolver configured".into()),
                ));
            }
        }

        let mut detection_findings = Vec::new();
        if let Some(detection) = &mut self.detection {
            match detection.evaluate(&event) {
                Ok(findings) => {
                    let status = if findings.is_empty() {
                        StepStatus::Skipped
                    } else {
                        StepStatus::Matched
                    };
                    steps.push(phase_step(
                        SecurityEnginePhase::Detection,
                        status,
                        findings.first().map(|finding| finding.rule_id.clone()),
                        findings.first().map(|finding| finding.pack_id.clone()),
                        None,
                    ));
                    event.findings.extend(findings.clone());
                    detection_findings = findings;
                }
                Err(error) => {
                    return Ok(error_result(
                        event,
                        steps,
                        SecurityEnginePhase::Detection,
                        error,
                    ));
                }
            }
        }

        for processor in &mut self.postprocessors {
            match processor.process(event.clone()) {
                Ok(next_event) => {
                    event = next_event;
                    steps.push(phase_step(
                        SecurityEnginePhase::Postprocessor,
                        StepStatus::Applied,
                        None,
                        None,
                        Some(format!("{} applied", processor.name())),
                    ));
                }
                Err(error) => {
                    return Ok(error_result(
                        event,
                        steps,
                        SecurityEnginePhase::Postprocessor,
                        error,
                    ));
                }
            }
        }

        let action = security_action_from_event(&event);
        Ok(SecurityResult {
            event_id: event.common.event_id.clone(),
            action: action.clone(),
            resolved_event: ResolvedSecurityEvent {
                schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
                event,
                steps,
                plugin_transforms: Vec::new(),
                detection_findings,
                final_action: action,
                emitter_results: Vec::new(),
            },
        })
    }
}

fn phase_step(
    phase: SecurityEnginePhase,
    status: StepStatus,
    rule_id: Option<String>,
    pack_id: Option<String>,
    message: Option<String>,
) -> ResolvedEventStep {
    ResolvedEventStep {
        kind: phase.step_kind(),
        status,
        rule_id,
        pack_id,
        message,
    }
}

fn error_result(
    event: SecurityEvent,
    mut steps: Vec<ResolvedEventStep>,
    phase: SecurityEnginePhase,
    error: SecurityEngineError,
) -> SecurityResult {
    let message = error.to_string();
    steps.push(phase_step(
        phase,
        StepStatus::Error,
        None,
        None,
        Some(message.clone()),
    ));
    let action = SecurityAction::Error(SecurityError {
        code: phase.code().into(),
        message,
    });
    SecurityResult {
        event_id: event.common.event_id.clone(),
        action: action.clone(),
        resolved_event: ResolvedSecurityEvent {
            schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
            event,
            steps,
            plugin_transforms: Vec::new(),
            detection_findings: Vec::new(),
            final_action: action,
            emitter_results: Vec::new(),
        },
    }
}

fn security_action_from_event(event: &SecurityEvent) -> SecurityAction {
    match event.decision.as_ref().map(|decision| decision.action) {
        Some(SecurityDecisionAction::Ask) => SecurityAction::Ask(AskPlan {
            prompt_id: format!("ask-{}", event.common.event_id),
            reason_code: decision_reason_code(event, "ask"),
            default_action: Box::new(SecurityAction::Block(BlockResponse {
                reason_code: "ask_default_block".into(),
                rule_id: event
                    .decision
                    .as_ref()
                    .and_then(|decision| decision.rule.clone()),
            })),
        }),
        Some(SecurityDecisionAction::Block) => SecurityAction::Block(BlockResponse {
            reason_code: decision_reason_code(event, "blocked"),
            rule_id: event
                .decision
                .as_ref()
                .and_then(|decision| decision.rule.clone()),
        }),
        Some(SecurityDecisionAction::Rewrite) => SecurityAction::Rewrite(RewritePatch {
            target: "event.mutations".into(),
            replacement_ref: event.common.event_id.clone(),
        }),
        Some(SecurityDecisionAction::Throttle) => SecurityAction::Throttle(ThrottlePlan {
            delay_ms: 0,
            quota_id: event
                .decision
                .as_ref()
                .and_then(|decision| decision.rule.clone())
                .unwrap_or_else(|| "runtime".into()),
            scope: event
                .common
                .accounting_owner
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            reason_code: decision_reason_code(event, "throttled"),
            provider_source: Some("security_engine".into()),
        }),
        Some(SecurityDecisionAction::Allow) => SecurityAction::Continue,
        None if !event.mutations.is_empty() => SecurityAction::Rewrite(RewritePatch {
            target: "event.mutations".into(),
            replacement_ref: event.common.event_id.clone(),
        }),
        None => SecurityAction::Continue,
    }
}

fn decision_reason_code(event: &SecurityEvent, fallback: &str) -> String {
    event
        .decision
        .as_ref()
        .and_then(|decision| decision.reason.clone())
        .unwrap_or_else(|| fallback.into())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PluginValidationError {
    #[error("mutation target is not allowed for {event_type}: {path}")]
    MutationTargetNotAllowed { event_type: String, path: String },
    #[error("plugin attempted to change immutable event field: {field}")]
    ImmutableFieldChanged { field: &'static str },
    #[error("plugin attempted to remove prior event data: {field}")]
    PriorEventDataRemoved { field: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportProjection {
    Continue,
    Rewrote,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginIdentity {
    pub id: String,
    pub version: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTransformRecord {
    pub plugin: PluginIdentity,
    pub input_event_hash: String,
    pub output_event_hash: String,
}

pub fn canonical_event_hash(event: &SecurityEvent) -> String {
    let encoded = serde_json::to_vec(event).expect("SecurityEvent serialization should not fail");
    format!("blake3:{}", blake3::hash(&encoded).to_hex())
}

pub fn validate_plugin_output(event: &SecurityEvent) -> Result<(), PluginValidationError> {
    for mutation in &event.mutations {
        let path = mutation.path();
        if !mutation_target_allowed(&event.common.event_type, path) {
            return Err(PluginValidationError::MutationTargetNotAllowed {
                event_type: event.common.event_type.clone(),
                path: path.to_owned(),
            });
        }
    }
    Ok(())
}

pub fn validate_plugin_transform(
    plugin: &PluginIdentity,
    input: &SecurityEvent,
    output: &SecurityEvent,
) -> Result<PluginTransformRecord, PluginValidationError> {
    validate_plugin_output(output)?;
    validate_immutable_plugin_fields(input, output)?;
    validate_prior_event_data_preserved(input, output)?;
    Ok(PluginTransformRecord {
        plugin: plugin.clone(),
        input_event_hash: canonical_event_hash(input),
        output_event_hash: canonical_event_hash(output),
    })
}

pub fn project_transport_outcome(
    event: &SecurityEvent,
) -> Result<TransportProjection, PluginValidationError> {
    validate_plugin_output(event)?;
    match event.decision.as_ref().map(|decision| decision.action) {
        Some(SecurityDecisionAction::Block)
        | Some(SecurityDecisionAction::Ask)
        | Some(SecurityDecisionAction::Throttle) => Ok(TransportProjection::Stop),
        Some(SecurityDecisionAction::Rewrite) => Ok(TransportProjection::Rewrote),
        Some(SecurityDecisionAction::Allow) | None if !event.mutations.is_empty() => {
            Ok(TransportProjection::Rewrote)
        }
        Some(SecurityDecisionAction::Allow) | None => Ok(TransportProjection::Continue),
    }
}

fn validate_immutable_plugin_fields(
    input: &SecurityEvent,
    output: &SecurityEvent,
) -> Result<(), PluginValidationError> {
    if input.schema_version != output.schema_version {
        return Err(PluginValidationError::ImmutableFieldChanged {
            field: "schema_version",
        });
    }
    if input.common != output.common {
        return Err(PluginValidationError::ImmutableFieldChanged { field: "common" });
    }
    if input.subject != output.subject {
        return Err(PluginValidationError::ImmutableFieldChanged { field: "subject" });
    }
    if input.context != output.context {
        return Err(PluginValidationError::ImmutableFieldChanged { field: "context" });
    }
    if input.trace != output.trace {
        return Err(PluginValidationError::ImmutableFieldChanged { field: "trace" });
    }
    Ok(())
}

fn validate_prior_event_data_preserved(
    input: &SecurityEvent,
    output: &SecurityEvent,
) -> Result<(), PluginValidationError> {
    if !contains_all(&output.labels, &input.labels) {
        return Err(PluginValidationError::PriorEventDataRemoved { field: "labels" });
    }
    if !contains_all(&output.findings, &input.findings) {
        return Err(PluginValidationError::PriorEventDataRemoved { field: "findings" });
    }
    if !contains_all(&output.mutations, &input.mutations) {
        return Err(PluginValidationError::PriorEventDataRemoved { field: "mutations" });
    }
    Ok(())
}

fn contains_all<T: PartialEq>(haystack: &[T], needles: &[T]) -> bool {
    needles.iter().all(|needle| haystack.contains(needle))
}

fn mutation_target_allowed(event_type: &str, path: &str) -> bool {
    match event_type {
        "http.request" => {
            path.starts_with("subject.headers.")
                || path == "subject.url"
                || path == "subject.body.text"
        }
        "http.response" => path.starts_with("subject.headers.") || path == "subject.body.text",
        "model.request" => {
            path == "subject.messages[*].content" || path == "subject.tool_results[*].content"
        }
        "model.response" => {
            path == "subject.output_text" || path == "subject.tool_calls[*].arguments"
        }
        "mcp.request" => path == "subject.params.arguments",
        "mcp.response" => path == "subject.result.content",
        _ => false,
    }
}

pub const DEFAULT_BACKTEST_MATCH_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestEventRef {
    pub corpus: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub event_id: String,
    #[serde(default)]
    pub sequence_no: Option<u64>,
    pub timestamp_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedField {
    pub path: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BacktestOutcome {
    Matched,
    NoMatch,
    Mismatch { expected: String, actual: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestMatchRow {
    pub event_ref: BacktestEventRef,
    pub rule_id: String,
    pub pack_id: String,
    pub evidence_signature: String,
    #[serde(default)]
    pub matched_fields: Vec<MatchedField>,
    pub outcome: BacktestOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestResult {
    pub total_matches: usize,
    pub unique_evidence_matches: usize,
    pub truncated: bool,
    pub rows: Vec<BacktestMatchRow>,
}

pub fn dedupe_backtest_matches(rows: Vec<BacktestMatchRow>, limit: usize) -> BacktestResult {
    let total_matches = rows.len();
    let mut seen = HashSet::new();
    let mut unique_evidence_matches = 0;
    let mut deduped = Vec::new();

    for row in rows {
        if seen.insert(row.evidence_signature.clone()) {
            unique_evidence_matches += 1;
            if deduped.len() < limit {
                deduped.push(row);
            }
        }
    }

    BacktestResult {
        total_matches,
        unique_evidence_matches,
        truncated: unique_evidence_matches > deduped.len(),
        rows: deduped,
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuleRegistryError {
    #[error("rule compilation failed: {0}")]
    CompileFailed(String),
    #[error("runtime rule not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleScope {
    Profile,
    User,
    Corp,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleOrigin {
    Profile,
    User,
    Corp,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRuleMetadata {
    pub id: String,
    #[serde(default)]
    pub pack_id: Option<String>,
    pub scope: RuleScope,
    pub origin: RuleOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRuleRecord {
    pub metadata: RuntimeRuleMetadata,
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CompileStatus {
    Compiled,
    Error { message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRuleStats {
    pub match_count: u64,
    #[serde(default)]
    pub last_matched_event: Option<String>,
    #[serde(default)]
    pub last_matched_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRuleEntry {
    pub metadata: RuntimeRuleMetadata,
    pub source: String,
    pub enabled: bool,
    pub compile_status: CompileStatus,
    pub generation: u64,
    pub stats: RuntimeRuleStats,
    pub compiled_plan: String,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeRuleRegistry {
    rules: BTreeMap<String, RuntimeRuleEntry>,
}

impl RuntimeRuleRegistry {
    pub fn add_or_update<F>(
        &mut self,
        record: RuntimeRuleRecord,
        compile: F,
    ) -> Result<(), RuleRegistryError>
    where
        F: FnOnce(&str) -> Result<String, RuleRegistryError>,
    {
        let compiled_plan = compile(&record.source)?;
        let generation = self
            .rules
            .get(&record.metadata.id)
            .map_or(1, |entry| entry.generation + 1);
        let stats = self
            .rules
            .get(&record.metadata.id)
            .map_or_else(RuntimeRuleStats::default, |entry| entry.stats.clone());
        self.rules.insert(
            record.metadata.id.clone(),
            RuntimeRuleEntry {
                metadata: record.metadata,
                source: record.source,
                enabled: record.enabled,
                compile_status: CompileStatus::Compiled,
                generation,
                stats,
                compiled_plan,
            },
        );
        Ok(())
    }

    pub fn delete(&mut self, rule_id: &str) -> Result<RuntimeRuleEntry, RuleRegistryError> {
        self.rules
            .remove(rule_id)
            .ok_or_else(|| RuleRegistryError::NotFound(rule_id.to_owned()))
    }

    pub fn list(&self) -> Vec<&RuntimeRuleEntry> {
        self.rules.values().collect()
    }

    pub fn stats(&self, rule_id: &str) -> Result<&RuntimeRuleStats, RuleRegistryError> {
        self.rules
            .get(rule_id)
            .map(|entry| &entry.stats)
            .ok_or_else(|| RuleRegistryError::NotFound(rule_id.to_owned()))
    }

    pub fn record_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), RuleRegistryError> {
        let entry = self
            .rules
            .get_mut(rule_id)
            .ok_or_else(|| RuleRegistryError::NotFound(rule_id.to_owned()))?;
        entry.stats.match_count += 1;
        entry.stats.last_matched_event = Some(event_id.to_owned());
        entry.stats.last_matched_unix_ms = Some(timestamp_unix_ms);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
