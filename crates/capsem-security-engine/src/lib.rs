use capsem_proto::{
    BodyPolicyContext, BodyState, CommonPolicyContext, ConversationActivityPolicyContext,
    ConversationPolicyContext, CredentialActivityPolicyContext, CredentialPolicyContext,
    DnsPolicyContext, DnsRequestPolicyContext, FileActivityPolicyContext, FilePolicyContext,
    HttpPolicyContext, HttpRequestPolicyContext, HttpResponsePolicyContext, McpPolicyContext,
    McpRequestPolicyContext, ModelEvidencePolicyContext, ModelPolicyContext,
    ModelRequestPolicyContext, ModelToolCallPolicyContext, ModelToolResultPolicyContext,
    PolicyContext, ProcessActivityPolicyContext, ProcessIdentityPolicyContext,
    ProcessPolicyContext, ProfileActivityPolicyContext, ProfilePolicyContext,
    SnapshotActivityPolicyContext, SnapshotPolicyContext, VmActivityPolicyContext, VmPolicyContext,
};
use cel::extractors::This;
use cel::objects::{Key, OptionalValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

pub mod detection_ir;

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

impl EventFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Http => "http",
            Self::Mcp => "mcp",
            Self::Model => "model",
            Self::File => "file",
            Self::Process => "process",
            Self::Credential => "credential",
            Self::Vm => "vm",
            Self::Profile => "profile",
            Self::Conversation => "conversation",
            Self::Snapshot => "snapshot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEventTypeParseError {
    value: String,
}

impl fmt::Display for SecurityEventTypeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown security event type '{}'", self.value)
    }
}

impl std::error::Error for SecurityEventTypeParseError {}

/// Closed security-event identity contract.
///
/// Variants with `Future` in their Rust name are intentionally reserved
/// contract points. They may be used by rule authoring and schema validation,
/// but producers must not emit them until a producing engine lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityEventType {
    DnsRequest,
    HttpRequest,
    HttpResponse,
    McpRequest,
    McpResponse,
    ModelRequest,
    ModelResponse,
    ModelToolCallFuture,
    ModelToolResponseFuture,
    FileActivity,
    FileRead,
    FileWrite,
    ProcessExec,
    CredentialRequest,
    CredentialActivity,
    VmCreate,
    VmStart,
    ProfileUpdate,
    ConversationMessage,
    SnapshotCreate,
}

impl SecurityEventType {
    pub const ALL: &'static [Self] = &[
        Self::DnsRequest,
        Self::HttpRequest,
        Self::HttpResponse,
        Self::McpRequest,
        Self::McpResponse,
        Self::ModelRequest,
        Self::ModelResponse,
        Self::ModelToolCallFuture,
        Self::ModelToolResponseFuture,
        Self::FileActivity,
        Self::FileRead,
        Self::FileWrite,
        Self::ProcessExec,
        Self::CredentialRequest,
        Self::CredentialActivity,
        Self::VmCreate,
        Self::VmStart,
        Self::ProfileUpdate,
        Self::ConversationMessage,
        Self::SnapshotCreate,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DnsRequest => "dns.request",
            Self::HttpRequest => "http.request",
            Self::HttpResponse => "http.response",
            Self::McpRequest => "mcp.request",
            Self::McpResponse => "mcp.response",
            Self::ModelRequest => "model.request",
            Self::ModelResponse => "model.response",
            Self::ModelToolCallFuture => "model.tool_call",
            Self::ModelToolResponseFuture => "model.tool_response",
            Self::FileActivity => "file.activity",
            Self::FileRead => "file.read",
            Self::FileWrite => "file.write",
            Self::ProcessExec => "process.exec",
            Self::CredentialRequest => "credential.request",
            Self::CredentialActivity => "credential.activity",
            Self::VmCreate => "vm.create",
            Self::VmStart => "vm.start",
            Self::ProfileUpdate => "profile.update",
            Self::ConversationMessage => "conversation.message",
            Self::SnapshotCreate => "snapshot.create",
        }
    }

    pub const fn family(self) -> EventFamily {
        match self {
            Self::DnsRequest => EventFamily::Dns,
            Self::HttpRequest | Self::HttpResponse => EventFamily::Http,
            Self::McpRequest | Self::McpResponse => EventFamily::Mcp,
            Self::ModelRequest
            | Self::ModelResponse
            | Self::ModelToolCallFuture
            | Self::ModelToolResponseFuture => EventFamily::Model,
            Self::FileActivity | Self::FileRead | Self::FileWrite => EventFamily::File,
            Self::ProcessExec => EventFamily::Process,
            Self::CredentialRequest | Self::CredentialActivity => EventFamily::Credential,
            Self::VmCreate | Self::VmStart => EventFamily::Vm,
            Self::ProfileUpdate => EventFamily::Profile,
            Self::ConversationMessage => EventFamily::Conversation,
            Self::SnapshotCreate => EventFamily::Snapshot,
        }
    }

    pub const fn is_future_marker(self) -> bool {
        matches!(
            self,
            Self::ModelToolCallFuture | Self::ModelToolResponseFuture
        )
    }

    pub fn parse(value: &str) -> Result<Self, SecurityEventTypeParseError> {
        Self::try_from(value)
    }

    pub fn callback_guard(callback: &str) -> Result<String, SecurityEventTypeParseError> {
        let event_type = Self::parse(callback)?;
        Ok(format!("common.event_type == '{}'", event_type.as_str()))
    }
}

impl fmt::Display for SecurityEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for SecurityEventType {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<SecurityEventType> for &str {
    fn eq(&self, other: &SecurityEventType) -> bool {
        *self == other.as_str()
    }
}

impl TryFrom<&str> for SecurityEventType {
    type Error = SecurityEventTypeParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let event_type = match value {
            "dns.request" => Self::DnsRequest,
            "http.request" => Self::HttpRequest,
            "http.response" => Self::HttpResponse,
            "mcp.request" => Self::McpRequest,
            "mcp.response" => Self::McpResponse,
            "model.request" => Self::ModelRequest,
            "model.response" => Self::ModelResponse,
            "model.tool_call" => Self::ModelToolCallFuture,
            "model.tool_response" => Self::ModelToolResponseFuture,
            "file.activity" => Self::FileActivity,
            "file.read" => Self::FileRead,
            "file.write" => Self::FileWrite,
            "process.exec" => Self::ProcessExec,
            "credential.request" => Self::CredentialRequest,
            "credential.activity" => Self::CredentialActivity,
            "vm.create" => Self::VmCreate,
            "vm.start" => Self::VmStart,
            "profile.update" => Self::ProfileUpdate,
            "conversation.message" => Self::ConversationMessage,
            "snapshot.create" => Self::SnapshotCreate,
            _ => {
                return Err(SecurityEventTypeParseError {
                    value: value.to_owned(),
                })
            }
        };
        Ok(event_type)
    }
}

impl FromStr for SecurityEventType {
    type Err = SecurityEventTypeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl Serialize for SecurityEventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecurityEventType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
    }
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
    pub event_type: SecurityEventType,
    #[serde(default)]
    pub redaction_state: RedactionState,
}

impl SecurityEventCommon {
    pub fn assert_family(&self, expected: EventFamily) {
        assert_eq!(
            self.event_type.family(),
            expected,
            "security event type '{}' belongs to family '{}', not '{}'",
            self.event_type.as_str(),
            self.event_type.family().as_str(),
            expected.as_str()
        );
    }
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
        common.assert_family(EventFamily::Dns);
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
        common.assert_family(EventFamily::Http);
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Http(Box::new(subject)),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn mcp(common: SecurityEventCommon, subject: McpSecuritySubject) -> Self {
        common.assert_family(EventFamily::Mcp);
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
        common.assert_family(EventFamily::Model);
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
        common.assert_family(EventFamily::File);
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
        common.assert_family(EventFamily::Process);
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

    pub fn credential(common: SecurityEventCommon, subject: CredentialSecuritySubject) -> Self {
        common.assert_family(EventFamily::Credential);
        Self {
            schema_version: SECURITY_EVENT_SCHEMA_VERSION,
            common,
            subject: SecurityEventSubject::Credential(subject),
            context: EventContext::default(),
            trace: TraceSnapshot::default(),
            labels: Vec::new(),
            findings: Vec::new(),
            decision: None,
            mutations: Vec::new(),
        }
    }

    pub fn conversation(common: SecurityEventCommon, subject: ConversationSecuritySubject) -> Self {
        common.assert_family(EventFamily::Conversation);
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
        common.assert_family(EventFamily::Snapshot);
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
        common.assert_family(EventFamily::Vm);
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
        common.assert_family(EventFamily::Profile);
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
            event_type: self.common.event_type.as_str().to_owned(),
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
            ..QuotaDimensions::default_for(
                self.event_family(),
                self.common.event_type.as_str().to_owned(),
            )
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<EventMutation>,
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
    Http(Box<HttpSecuritySubject>),
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSecuritySubject {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub path_class: String,
    pub request_bytes: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub request_headers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<HttpBodySecuritySubject>,
    #[serde(default)]
    pub response_status: Option<u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub response_headers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub response_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body: Option<HttpBodySecuritySubject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpBodySecuritySubject {
    pub state: HttpBodySecurityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction_reason: Option<String>,
}

impl HttpBodySecuritySubject {
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            state: HttpBodySecurityState::Text,
            size: Some(text.len() as u64),
            text: Some(text),
            content_type: None,
            truncated: false,
            redaction_reason: None,
        }
    }

    pub fn redacted(reason: impl Into<String>) -> Self {
        Self {
            state: HttpBodySecurityState::Redacted,
            text: None,
            content_type: None,
            size: None,
            truncated: false,
            redaction_reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpBodySecurityState {
    Missing,
    Text,
    Binary,
    Redacted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSecuritySubject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
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
            method: Some("tools/call".into()),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub path_class: String,
    #[serde(default)]
    pub byte_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<HttpBodySecuritySubject>,
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

pub trait SecurityEventProcessor: Send {
    fn name(&self) -> &str;
    fn process(&mut self, event: SecurityEvent) -> Result<SecurityEvent, SecurityEngineError>;
}

pub trait EnforcementEvaluator: Send {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Option<SecurityDecision>, SecurityEngineError>;
}

pub trait ConfirmResolver: Send {
    fn resolve(
        &mut self,
        event: &SecurityEvent,
        decision: &SecurityDecision,
    ) -> Result<SecurityDecision, SecurityEngineError>;
}

pub trait DetectionEvaluator: Send {
    fn evaluate(
        &mut self,
        event: &SecurityEvent,
    ) -> Result<Vec<DetectionFinding>, SecurityEngineError>;
}

pub trait RuleMatchRecorder: Send {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), SecurityEngineError>;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<EventMutation>,
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
            let program = compile_policy_cel(&rule.id, &rule.condition)?;
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
                    mutations: compiled.rule.mutations.clone(),
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
            let program = compile_policy_cel(&rule.id, &rule.condition)?;
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
    evaluate_policy_cel_bool(rule_id, program, &policy_context_from_event(event))
}

fn evaluate_policy_cel_bool(
    rule_id: &str,
    program: &cel::Program,
    policy_context: &PolicyContext,
) -> Result<bool, SecurityEngineError> {
    let mut context = cel::Context::default();
    add_policy_context_roots(&mut context, rule_id, policy_context)?;
    context.add_function("contains", policy_contains);
    context.add_function("match", policy_match);
    context.add_function("matches", policy_match);
    context.add_function("header", policy_header);
    context.add_function("exists", policy_exists);

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

fn compile_policy_cel(rule_id: &str, condition: &str) -> Result<cel::Program, SecurityEngineError> {
    let program = cel::Program::compile(condition).map_err(|error| {
        SecurityEngineError::CelCompileFailed {
            rule_id: rule_id.to_owned(),
            message: error.to_string(),
        }
    })?;
    validate_policy_cel_references(rule_id, &program)?;
    Ok(program)
}

fn validate_policy_cel_references(
    rule_id: &str,
    program: &cel::Program,
) -> Result<(), SecurityEngineError> {
    let allowed_roots = [
        "common",
        "http",
        "dns",
        "mcp",
        "model",
        "file",
        "process",
        "credential",
        "vm",
        "profile",
        "conversation",
        "snapshot",
    ];
    let references = program.references();
    for variable in references.variables() {
        if variable == "event" {
            return Err(SecurityEngineError::CelCompileFailed {
                rule_id: rule_id.to_owned(),
                message: "internal event.* paths are not part of the policy CEL ABI".into(),
            });
        }
        if !allowed_roots.contains(&variable) {
            return Err(SecurityEngineError::CelCompileFailed {
                rule_id: rule_id.to_owned(),
                message: format!("unknown policy CEL root {variable:?}"),
            });
        }
    }
    Ok(())
}

fn add_policy_context_roots(
    context: &mut cel::Context,
    rule_id: &str,
    policy_context: &PolicyContext,
) -> Result<(), SecurityEngineError> {
    add_policy_context_root(context, rule_id, "common", &policy_context.common)?;
    add_policy_context_root(context, rule_id, "http", &policy_context.http)?;
    add_policy_context_root(context, rule_id, "dns", &policy_context.dns)?;
    add_policy_context_root(context, rule_id, "mcp", &policy_context.mcp)?;
    add_policy_context_root(context, rule_id, "model", &policy_context.model)?;
    add_policy_context_root(context, rule_id, "file", &policy_context.file)?;
    add_policy_context_root(context, rule_id, "process", &policy_context.process)?;
    add_policy_context_root(context, rule_id, "credential", &policy_context.credential)?;
    add_policy_context_root(context, rule_id, "vm", &policy_context.vm)?;
    add_policy_context_root(context, rule_id, "profile", &policy_context.profile)?;
    add_policy_context_root(
        context,
        rule_id,
        "conversation",
        &policy_context.conversation,
    )?;
    add_policy_context_root(context, rule_id, "snapshot", &policy_context.snapshot)?;
    Ok(())
}

fn add_policy_context_root<T>(
    context: &mut cel::Context,
    rule_id: &str,
    name: &str,
    value: &T,
) -> Result<(), SecurityEngineError>
where
    T: Serialize,
{
    let value = cel::to_value(value).map_err(|error| SecurityEngineError::CelEvaluationFailed {
        rule_id: rule_id.to_owned(),
        message: error.to_string(),
    })?;
    context
        .add_variable(name, value)
        .map_err(|error| SecurityEngineError::CelEvaluationFailed {
            rule_id: rule_id.to_owned(),
            message: error.to_string(),
        })
}

fn policy_contains(
    This(this): This<cel::Value>,
    needle: cel::Value,
) -> Result<cel::Value, cel::ExecutionError> {
    Ok(value_contains(&this, &needle).into())
}

fn policy_match(
    ftx: &cel::FunctionContext,
    This(this): This<cel::Value>,
    pattern: Arc<String>,
) -> Result<bool, cel::ExecutionError> {
    let regex = regex::Regex::new(&pattern)
        .map_err(|error| ftx.error(format!("invalid regex pattern: {error}")))?;
    Ok(value_matches(&this, &regex))
}

fn value_contains(value: &cel::Value, needle: &cel::Value) -> bool {
    match value {
        cel::Value::String(text) => value_search_needle(needle)
            .as_deref()
            .is_some_and(|needle| text.contains(needle)),
        cel::Value::Bytes(bytes) => match needle {
            cel::Value::Bytes(needle) => {
                needle.is_empty()
                    || bytes
                        .windows(needle.len())
                        .any(|window| window == needle.as_slice())
            }
            _ => value_search_needle(needle)
                .and_then(|needle| {
                    String::from_utf8(bytes.to_vec())
                        .ok()
                        .map(|text| (text, needle))
                })
                .is_some_and(|(text, needle)| text.contains(&needle)),
        },
        cel::Value::List(values) => values
            .iter()
            .any(|value| value == needle || value_contains(value, needle)),
        cel::Value::Map(map) => {
            map.map.keys().any(|key| key_equals_value(key, needle))
                || map.map.iter().any(|(key, value)| {
                    key_contains(key, needle) || value == needle || value_contains(value, needle)
                })
        }
        cel::Value::Int(_)
        | cel::Value::UInt(_)
        | cel::Value::Float(_)
        | cel::Value::Bool(_)
        | cel::Value::Null => value_search_needle(needle)
            .is_some_and(|needle| scalar_text(value).is_some_and(|text| text.contains(&needle))),
        cel::Value::Duration(_) | cel::Value::Timestamp(_) => false,
        cel::Value::Function(_, _) | cel::Value::Opaque(_) => false,
    }
}

fn value_matches(value: &cel::Value, regex: &regex::Regex) -> bool {
    match value {
        cel::Value::String(text) => regex.is_match(text),
        cel::Value::Bytes(bytes) => String::from_utf8(bytes.to_vec())
            .ok()
            .is_some_and(|text| regex.is_match(&text)),
        cel::Value::List(values) => values.iter().any(|value| value_matches(value, regex)),
        cel::Value::Map(map) => map
            .map
            .iter()
            .any(|(key, value)| regex.is_match(&key_text(key)) || value_matches(value, regex)),
        cel::Value::Int(_)
        | cel::Value::UInt(_)
        | cel::Value::Float(_)
        | cel::Value::Bool(_)
        | cel::Value::Null => scalar_text(value).is_some_and(|text| regex.is_match(&text)),
        cel::Value::Duration(_) | cel::Value::Timestamp(_) => false,
        cel::Value::Function(_, _) | cel::Value::Opaque(_) => false,
    }
}

fn key_equals_value(key: &Key, value: &cel::Value) -> bool {
    match (key, value) {
        (Key::Int(left), cel::Value::Int(right)) => left == right,
        (Key::Uint(left), cel::Value::UInt(right)) => left == right,
        (Key::Bool(left), cel::Value::Bool(right)) => left == right,
        (Key::String(left), cel::Value::String(right)) => left == right,
        _ => false,
    }
}

fn key_contains(key: &Key, needle: &cel::Value) -> bool {
    value_search_needle(needle).is_some_and(|needle| key_text(key).contains(&needle))
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Int(value) => value.to_string(),
        Key::Uint(value) => value.to_string(),
        Key::Bool(value) => value.to_string(),
        Key::String(value) => value.to_string(),
    }
}

fn value_search_needle(value: &cel::Value) -> Option<String> {
    match value {
        cel::Value::String(text) => Some(text.to_string()),
        cel::Value::Bytes(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        cel::Value::Int(_)
        | cel::Value::UInt(_)
        | cel::Value::Float(_)
        | cel::Value::Bool(_)
        | cel::Value::Null => scalar_text(value),
        _ => None,
    }
}

fn scalar_text(value: &cel::Value) -> Option<String> {
    match value {
        cel::Value::Int(value) => Some(value.to_string()),
        cel::Value::UInt(value) => Some(value.to_string()),
        cel::Value::Float(value) => Some(value.to_string()),
        cel::Value::Bool(value) => Some(value.to_string()),
        cel::Value::Null => Some("null".into()),
        _ => None,
    }
}

fn policy_header(
    ftx: &cel::FunctionContext,
    This(this): This<cel::Value>,
    name: Arc<String>,
) -> Result<cel::Value, cel::ExecutionError> {
    let Some(headers) = policy_map_field(&this, "headers") else {
        return Ok(optional_none());
    };
    let Some(value) = headers.map.iter().find_map(|(key, value)| match key {
        cel::objects::Key::String(header_name) if header_name.eq_ignore_ascii_case(&name) => {
            Some(value)
        }
        _ => None,
    }) else {
        return Ok(optional_none());
    };

    let first = match value {
        cel::Value::List(values) => values.first().cloned(),
        cel::Value::String(_) => Some(value.clone()),
        other => return Err(ftx.error(format!("unsupported header value shape: {other:?}"))),
    };

    Ok(first.map(optional_of).unwrap_or_else(optional_none))
}

fn policy_exists(This(this): This<cel::Value>) -> Result<bool, cel::ExecutionError> {
    Ok(<&OptionalValue>::try_from(&this)?.value().is_some())
}

fn policy_map_field<'a>(value: &'a cel::Value, field: &str) -> Option<&'a cel::objects::Map> {
    let cel::Value::Map(map) = value else {
        return None;
    };
    match map.get(&cel::objects::KeyRef::String(field)) {
        Some(cel::Value::Map(map)) => Some(map),
        _ => None,
    }
}

fn optional_of(value: cel::Value) -> cel::Value {
    cel::Value::Opaque(Arc::new(OptionalValue::of(value)))
}

fn optional_none() -> cel::Value {
    cel::Value::Opaque(Arc::new(OptionalValue::none()))
}

pub fn policy_context_from_event(event: &SecurityEvent) -> PolicyContext {
    let mut context = PolicyContext::new();
    context.common =
        CommonPolicyContext {
            session_id: event.common.session_id.clone(),
            vm_id: event.common.vm_id.clone(),
            profile_id: event.common.profile_id.clone(),
            profile_revision: event.common.profile_revision.clone(),
            user_id: event.common.user_id.clone(),
            event_type: Some(event.common.event_type.as_str().to_owned()),
            enforceability: Some(
                match event.common.enforceability {
                    Enforceability::InlineBlockable => "inline_blockable",
                    Enforceability::ObserveOnly => "observe_only",
                    Enforceability::RemediationOnly => "remediation_only",
                }
                .into(),
            ),
            actor: event.common.accounting_owner.clone(),
            process: event.common.process_id.as_ref().map(|process_id| {
                ProcessIdentityPolicyContext {
                    pid: process_id.parse::<u32>().ok(),
                    ppid: event
                        .common
                        .parent_process_id
                        .as_deref()
                        .and_then(|pid| pid.parse::<u32>().ok()),
                    executable: None,
                    command: None,
                    cwd: None,
                }
            }),
            labels: event
                .labels
                .iter()
                .map(|label| (label.clone(), "true".to_owned()))
                .collect(),
        };

    match &event.subject {
        SecurityEventSubject::Dns(subject) => {
            context.dns = DnsPolicyContext {
                request: Some(DnsRequestPolicyContext {
                    qname: Some(subject.qname.clone()),
                    qtype: None,
                    domain_class: Some(subject.domain_class.clone()),
                    transport: None,
                }),
            };
        }
        SecurityEventSubject::Http(subject) => {
            context.http = HttpPolicyContext {
                request: Some(HttpRequestPolicyContext {
                    method: Some(subject.method.clone()),
                    scheme: subject.scheme.clone(),
                    host: Some(subject.host.clone()),
                    port: subject.port,
                    path: subject.path.clone(),
                    query: subject.query.clone(),
                    url: subject.url.clone(),
                    path_class: Some(subject.path_class.clone()),
                    bytes: Some(subject.request_bytes),
                    headers: subject.request_headers.clone(),
                    body: subject
                        .request_body
                        .as_ref()
                        .map(http_body_policy_context)
                        .unwrap_or_else(BodyPolicyContext::missing),
                }),
                response: http_response_policy_context(subject),
            };
        }
        SecurityEventSubject::Mcp(subject) => {
            context.mcp = McpPolicyContext {
                request: Some(McpRequestPolicyContext {
                    method: subject.method.clone(),
                    server_id: Some(subject.server_id.clone()),
                    tool_name: Some(subject.tool_name.clone()),
                    server_name: None,
                    arguments_status: subject.evidence.as_deref().map(|evidence| {
                        if evidence.request_arguments_json.is_some() {
                            "valid_json".to_owned()
                        } else if evidence.request_arguments_raw.is_some() {
                            "not_json".to_owned()
                        } else {
                            "absent".to_owned()
                        }
                    }),
                    arguments: subject
                        .evidence
                        .as_deref()
                        .map(mcp_arguments_policy_context)
                        .unwrap_or_else(BodyPolicyContext::missing),
                }),
                response: subject.evidence.as_deref().map(|evidence| {
                    capsem_proto::McpResponsePolicyContext {
                        method: subject.method.clone(),
                        server_id: Some(evidence.server_id.clone()),
                        tool_name: Some(evidence.tool_name.clone()),
                        is_error: Some(evidence.is_error),
                        result_status: Some(if evidence.is_error { "error" } else { "ok" }.into()),
                        result: mcp_result_policy_context(evidence),
                    }
                }),
            };
        }
        SecurityEventSubject::Model(subject) => {
            context.model = ModelPolicyContext {
                request: Some(ModelRequestPolicyContext {
                    provider: Some(subject.provider.clone()),
                    api_family: subject
                        .evidence
                        .as_deref()
                        .and_then(|evidence| serialized_enum_string(evidence.api_family)),
                    model: Some(subject.model.clone()),
                    stream: subject
                        .evidence
                        .as_deref()
                        .map(|evidence| evidence.request.stream),
                    operation: None,
                    estimated_input_tokens: subject.estimated_input_tokens,
                    estimated_output_tokens: subject.estimated_output_tokens,
                    estimated_cost_micros: subject.estimated_cost_micros,
                    body: BodyPolicyContext::missing(),
                    tool_calls: subject
                        .evidence
                        .as_deref()
                        .map(model_tool_call_policy_contexts)
                        .unwrap_or_default(),
                }),
                response: Some(capsem_proto::ModelResponsePolicyContext {
                    provider: Some(subject.provider.clone()),
                    api_family: subject
                        .evidence
                        .as_deref()
                        .and_then(|evidence| serialized_enum_string(evidence.api_family)),
                    model: Some(subject.model.clone()),
                    status: None,
                    stop_reason: subject
                        .evidence
                        .as_deref()
                        .and_then(|evidence| evidence.response.as_ref())
                        .and_then(|response| response.stop_reason.clone()),
                    estimated_output_tokens: subject.estimated_output_tokens,
                    body: subject
                        .evidence
                        .as_deref()
                        .and_then(|evidence| evidence.response.as_ref())
                        .and_then(|response| response.text_preview.clone())
                        .map(BodyPolicyContext::text)
                        .unwrap_or_else(BodyPolicyContext::missing),
                    tool_results: subject
                        .evidence
                        .as_deref()
                        .map(model_tool_result_policy_contexts)
                        .unwrap_or_default(),
                }),
                evidence: subject
                    .evidence
                    .as_deref()
                    .map(|evidence| ModelEvidencePolicyContext {
                        parse_status: serialized_enum_string(evidence.parse_status),
                        status: serialized_enum_string(evidence.evidence_status),
                    }),
            };
        }
        SecurityEventSubject::File(subject) => {
            context.file = FilePolicyContext {
                activity: Some(FileActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    path: subject.path.clone(),
                    path_class: Some(subject.path_class.clone()),
                    byte_count: subject.byte_count,
                    content: subject
                        .content
                        .as_ref()
                        .map(http_body_policy_context)
                        .unwrap_or_else(BodyPolicyContext::missing),
                }),
            };
        }
        SecurityEventSubject::Process(subject) => {
            context.process = ProcessPolicyContext {
                activity: Some(ProcessActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    executable: None,
                    command: None,
                    command_class: subject.command_class.clone(),
                    argv: Vec::new(),
                    cwd: None,
                }),
            };
        }
        SecurityEventSubject::Credential(subject) => {
            context.credential = CredentialPolicyContext {
                activity: Some(CredentialActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    credential_id: Some(subject.credential_id.clone()),
                }),
            };
        }
        SecurityEventSubject::VmLifecycle(subject) => {
            context.vm = VmPolicyContext {
                activity: Some(VmActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                }),
            };
        }
        SecurityEventSubject::Profile(subject) => {
            context.profile = ProfilePolicyContext {
                activity: Some(ProfileActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    profile_id: Some(subject.profile_id.clone()),
                    profile_revision: Some(subject.profile_revision.clone()),
                    profile_name: None,
                }),
            };
        }
        SecurityEventSubject::Conversation(subject) => {
            context.conversation = ConversationPolicyContext {
                activity: Some(ConversationActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    conversation_id: subject.conversation_id.clone(),
                }),
            };
        }
        SecurityEventSubject::Snapshot(subject) => {
            context.snapshot = SnapshotPolicyContext {
                activity: Some(SnapshotActivityPolicyContext {
                    operation: Some(subject.operation.clone()),
                    snapshot_id: Some(subject.snapshot_id.clone()),
                }),
            };
        }
    }
    context
}

fn serialized_enum_string<T: Serialize>(value: T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn model_tool_call_policy_contexts(
    evidence: &ModelInteractionEvidence,
) -> Vec<ModelToolCallPolicyContext> {
    evidence
        .tool_calls
        .iter()
        .map(|tool_call| ModelToolCallPolicyContext {
            tool_call_id: Some(tool_call.tool_call_id.clone()),
            provider_call_id: tool_call.provider_call_id.clone(),
            raw_name: Some(tool_call.raw_name.clone()),
            name: Some(tool_call.normalized_name.clone()),
            origin: serialized_enum_string(tool_call.origin),
            arguments_status: serialized_enum_string(tool_call.arguments_status),
            status: serialized_enum_string(tool_call.status),
            linked_mcp_call_id: tool_call.linked_mcp_call_id.clone(),
            parse_confidence: serialized_enum_string(tool_call.parse_confidence),
            arguments: model_tool_call_arguments_policy_context(tool_call),
        })
        .collect()
}

fn mcp_arguments_policy_context(evidence: &McpToolExecutionEvidence) -> BodyPolicyContext {
    evidence
        .request_arguments_json
        .as_deref()
        .map(|arguments| text_body_policy_context(arguments, Some("application/json")))
        .or_else(|| {
            evidence
                .request_arguments_raw
                .as_deref()
                .map(|arguments| text_body_policy_context(arguments, None))
        })
        .unwrap_or_else(BodyPolicyContext::missing)
}

fn mcp_result_policy_context(evidence: &McpToolExecutionEvidence) -> BodyPolicyContext {
    evidence
        .result_json
        .as_deref()
        .map(|result| text_body_policy_context(result, Some("application/json")))
        .or_else(|| {
            evidence
                .result_preview
                .as_deref()
                .map(|result| text_body_policy_context(result, None))
        })
        .unwrap_or_else(BodyPolicyContext::missing)
}

fn model_tool_call_arguments_policy_context(
    tool_call: &ModelToolCallEvidence,
) -> BodyPolicyContext {
    tool_call
        .arguments_json
        .as_deref()
        .map(|arguments| text_body_policy_context(arguments, Some("application/json")))
        .or_else(|| {
            tool_call
                .arguments_raw
                .as_deref()
                .map(|arguments| text_body_policy_context(arguments, None))
        })
        .unwrap_or_else(BodyPolicyContext::missing)
}

fn text_body_policy_context(text: &str, content_type: Option<&str>) -> BodyPolicyContext {
    BodyPolicyContext {
        state: BodyState::Text,
        text: Some(text.to_owned()),
        content_type: content_type.map(str::to_owned),
        size: Some(text.len() as u64),
        truncated: false,
        redaction_reason: None,
    }
}

fn model_tool_result_policy_contexts(
    evidence: &ModelInteractionEvidence,
) -> Vec<ModelToolResultPolicyContext> {
    evidence
        .tool_results
        .iter()
        .map(|tool_result| ModelToolResultPolicyContext {
            tool_call_id: Some(tool_result.tool_call_id.clone()),
            linked_mcp_call_id: tool_result.linked_mcp_call_id.clone(),
            content_kind: serialized_enum_string(tool_result.content_kind),
            content_preview: tool_result.content_preview.clone(),
            content_json: tool_result.content_json.clone(),
            is_error: Some(tool_result.is_error),
            result_status: serialized_enum_string(tool_result.result_status),
            returned_to_model: Some(tool_result.returned_to_model),
            parse_confidence: serialized_enum_string(tool_result.parse_confidence),
        })
        .collect()
}

fn http_body_policy_context(body: &HttpBodySecuritySubject) -> BodyPolicyContext {
    BodyPolicyContext {
        state: match body.state {
            HttpBodySecurityState::Missing => BodyState::Missing,
            HttpBodySecurityState::Text => BodyState::Text,
            HttpBodySecurityState::Binary => BodyState::Binary,
            HttpBodySecurityState::Redacted => BodyState::Redacted,
        },
        text: body.text.clone(),
        content_type: body.content_type.clone(),
        size: body.size,
        truncated: body.truncated,
        redaction_reason: body.redaction_reason.clone(),
    }
}

fn http_response_policy_context(
    subject: &HttpSecuritySubject,
) -> Option<HttpResponsePolicyContext> {
    if subject.response_status.is_none()
        && subject.response_bytes.is_none()
        && subject.response_headers.is_empty()
        && subject.response_body.is_none()
    {
        return None;
    }

    Some(HttpResponsePolicyContext {
        status: subject.response_status,
        bytes: subject.response_bytes,
        headers: subject.response_headers.clone(),
        body: subject
            .response_body
            .as_ref()
            .map(http_body_policy_context)
            .unwrap_or_else(BodyPolicyContext::missing),
    })
}

#[derive(Default)]
pub struct SecurityEngine {
    preprocessors: Vec<Box<dyn SecurityEventProcessor>>,
    enforcement: Option<Box<dyn EnforcementEvaluator>>,
    confirm: Option<Box<dyn ConfirmResolver>>,
    detection: Option<Box<dyn DetectionEvaluator>>,
    match_recorder: Option<Box<dyn RuleMatchRecorder>>,
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

    pub fn set_match_recorder(&mut self, recorder: Box<dyn RuleMatchRecorder>) {
        self.match_recorder = Some(recorder);
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
                    if let Some(rule_id) = decision.rule.as_deref() {
                        record_rule_match(
                            &mut self.match_recorder,
                            rule_id,
                            &event.common.event_id,
                            event.common.timestamp_unix_ms,
                        )?;
                    }
                    steps.push(phase_step(
                        SecurityEnginePhase::Enforcement,
                        StepStatus::Matched,
                        decision.rule.clone(),
                        decision.pack_id.clone(),
                        decision.reason.clone(),
                    ));
                    event.mutations.extend(decision.mutations.clone());
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
                let ask_decision = event.decision.clone().expect("decision checked above");
                let resolved_decision = default_deny_confirm_decision(&ask_decision);
                steps.push(phase_step(
                    SecurityEnginePhase::Confirm,
                    StepStatus::Applied,
                    resolved_decision.rule.clone(),
                    resolved_decision.pack_id.clone(),
                    resolved_decision.reason.clone(),
                ));
                event.decision = Some(resolved_decision);
            }
        }

        let mut detection_findings = Vec::new();
        if let Some(detection) = &mut self.detection {
            match detection.evaluate(&event) {
                Ok(findings) => {
                    for finding in &findings {
                        record_rule_match(
                            &mut self.match_recorder,
                            &finding.rule_id,
                            &event.common.event_id,
                            event.common.timestamp_unix_ms,
                        )?;
                    }
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

fn record_rule_match(
    recorder: &mut Option<Box<dyn RuleMatchRecorder>>,
    rule_id: &str,
    event_id: &str,
    timestamp_unix_ms: u64,
) -> Result<(), SecurityEngineError> {
    if let Some(recorder) = recorder {
        recorder.record_rule_match(rule_id, event_id, timestamp_unix_ms)?;
    }
    Ok(())
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

fn default_deny_confirm_decision(decision: &SecurityDecision) -> SecurityDecision {
    let reason = decision
        .reason
        .as_deref()
        .map(|reason| format!("{reason}; default denied because no confirm resolver is configured"))
        .unwrap_or_else(|| "default denied because no confirm resolver is configured".into());
    SecurityDecision {
        action: SecurityDecisionAction::Block,
        rule: decision.rule.clone(),
        pack_id: decision.pack_id.clone(),
        reason: Some(reason),
        terminal: true,
        mutations: Vec::new(),
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
        if !mutation_target_allowed(event.common.event_type, path) {
            return Err(PluginValidationError::MutationTargetNotAllowed {
                event_type: event.common.event_type.as_str().to_owned(),
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

fn mutation_target_allowed(event_type: SecurityEventType, path: &str) -> bool {
    match event_type {
        SecurityEventType::HttpRequest => {
            path.starts_with("subject.headers.")
                || path == "subject.url"
                || path == "subject.body.text"
        }
        SecurityEventType::HttpResponse => {
            path.starts_with("subject.headers.") || path == "subject.body.text"
        }
        SecurityEventType::ModelRequest => {
            path == "subject.messages[*].content" || path == "subject.tool_results[*].content"
        }
        SecurityEventType::ModelResponse => {
            path == "subject.output_text" || path == "subject.tool_calls[*].arguments"
        }
        SecurityEventType::McpRequest => path == "subject.params.arguments",
        SecurityEventType::McpResponse => path == "subject.result.content",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestEventInput {
    pub event_ref: Option<BacktestEventRef>,
    pub event: SecurityEvent,
    pub expected: Option<String>,
}

pub fn runtime_rule_plan_id(condition: &str) -> String {
    format!("cel:{}", blake3::hash(condition.as_bytes()).to_hex())
}

pub fn validate_runtime_enforcement_decision_supported(
    decision: SecurityDecisionAction,
) -> Result<(), String> {
    if decision == SecurityDecisionAction::Ask {
        return Err(
            "ask decisions require S15-confirm-ux; runtime ask overlays are disabled until the confirm resolver is wired"
                .into(),
        );
    }
    Ok(())
}

pub fn compile_runtime_enforcement_rule(
    rule: CelEnforcementRule,
) -> Result<String, SecurityEngineError> {
    validate_runtime_enforcement_decision_supported(rule.decision).map_err(|message| {
        SecurityEngineError::CelCompileFailed {
            rule_id: rule.id.clone(),
            message,
        }
    })?;
    let plan_id = runtime_rule_plan_id(&rule.condition);
    CelEnforcementEvaluator::compile(vec![rule])?;
    Ok(plan_id)
}

pub fn compile_runtime_detection_rule(
    rule: CelDetectionRule,
) -> Result<String, SecurityEngineError> {
    let plan_id = runtime_rule_plan_id(&rule.condition);
    CelDetectionEvaluator::compile(vec![rule])?;
    Ok(plan_id)
}

pub fn compile_runtime_rule_record(
    record: &RuntimeRuleRecord,
) -> Result<String, SecurityEngineError> {
    match &record.definition {
        RuntimeRuleDefinition::Enforcement { decision, reason } => {
            compile_runtime_enforcement_rule(CelEnforcementRule {
                id: record.metadata.id.clone(),
                pack_id: record.metadata.pack_id.clone(),
                condition: record.source.clone(),
                decision: *decision,
                reason: reason.clone(),
                mutations: Vec::new(),
            })
        }
        RuntimeRuleDefinition::Detection {
            sigma_id,
            title,
            severity,
            confidence,
            tags,
        } => compile_runtime_detection_rule(CelDetectionRule {
            id: record.metadata.id.clone(),
            pack_id: record
                .metadata
                .pack_id
                .clone()
                .unwrap_or_else(|| "runtime".into()),
            sigma_id: sigma_id.clone(),
            title: title.clone(),
            condition: record.source.clone(),
            severity: *severity,
            confidence: *confidence,
            tags: tags.clone(),
        }),
    }
}

pub fn runtime_backtest_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_BACKTEST_MATCH_LIMIT)
}

pub fn inline_backtest_event_ref(input: &BacktestEventInput) -> BacktestEventRef {
    input.event_ref.clone().unwrap_or_else(|| BacktestEventRef {
        corpus: "inline".into(),
        session_id: input.event.common.session_id.clone(),
        event_id: input.event.common.event_id.clone(),
        sequence_no: input.event.common.sequence_no,
        timestamp_unix_ms: input.event.common.timestamp_unix_ms,
    })
}

pub fn backtest_evidence_signature(event: &SecurityEvent) -> Result<String, SecurityEngineError> {
    let evidence = serde_json::json!({
        "event_type": &event.common.event_type,
        "subject": &event.subject,
    });
    let evidence =
        serde_json::to_vec(&evidence).map_err(|error| SecurityEngineError::PhaseFailed {
            phase: SecurityEnginePhase::Detection,
            message: format!("serialize backtest evidence: {error}"),
        })?;
    Ok(blake3::hash(&evidence).to_hex().to_string())
}

pub fn backtest_outcome(expected: Option<&str>, actual: &str) -> BacktestOutcome {
    match expected {
        Some(expected) if expected != actual => BacktestOutcome::Mismatch {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        },
        _ => BacktestOutcome::Matched,
    }
}

pub fn security_decision_action_text(
    action: SecurityDecisionAction,
) -> Result<String, SecurityEngineError> {
    serde_json::to_value(action)
        .map_err(|error| SecurityEngineError::PhaseFailed {
            phase: SecurityEnginePhase::Enforcement,
            message: format!("serialize security decision action: {error}"),
        })?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| SecurityEngineError::PhaseFailed {
            phase: SecurityEnginePhase::Enforcement,
            message: "security decision action did not serialize as a string".into(),
        })
}

pub fn run_enforcement_backtest(
    rule: CelEnforcementRule,
    events: &[BacktestEventInput],
    limit: Option<usize>,
) -> Result<BacktestResult, SecurityEngineError> {
    let fallback_rule_id = rule.id.clone();
    let fallback_pack_id = rule.pack_id.clone();
    let mut evaluator = CelEnforcementEvaluator::compile(vec![rule])?;
    let mut rows = Vec::new();
    for input in events {
        if let Some(decision) = EnforcementEvaluator::evaluate(&mut evaluator, &input.event)? {
            let actual = security_decision_action_text(decision.action)?;
            rows.push(BacktestMatchRow {
                event_ref: inline_backtest_event_ref(input),
                rule_id: decision.rule.unwrap_or_else(|| fallback_rule_id.clone()),
                pack_id: decision
                    .pack_id
                    .or_else(|| fallback_pack_id.clone())
                    .unwrap_or_else(|| "runtime".into()),
                evidence_signature: backtest_evidence_signature(&input.event)?,
                matched_fields: backtest_matched_fields(&input.event)?,
                outcome: backtest_outcome(input.expected.as_deref(), &actual),
            });
        }
    }
    Ok(dedupe_backtest_matches(rows, runtime_backtest_limit(limit)))
}

pub fn run_detection_backtest(
    rule: CelDetectionRule,
    events: &[BacktestEventInput],
    limit: Option<usize>,
) -> Result<BacktestResult, SecurityEngineError> {
    run_detection_hunt(vec![rule], events, limit)
}

pub fn run_detection_hunt(
    rules: Vec<CelDetectionRule>,
    events: &[BacktestEventInput],
    limit: Option<usize>,
) -> Result<BacktestResult, SecurityEngineError> {
    if rules.is_empty() {
        return Err(SecurityEngineError::PhaseFailed {
            phase: SecurityEnginePhase::Detection,
            message: "detection hunt requires at least one rule".into(),
        });
    }
    let mut evaluator = CelDetectionEvaluator::compile(rules)?;
    let mut rows = Vec::new();
    for input in events {
        let findings = DetectionEvaluator::evaluate(&mut evaluator, &input.event)?;
        for finding in findings {
            rows.push(BacktestMatchRow {
                event_ref: inline_backtest_event_ref(input),
                rule_id: finding.rule_id,
                pack_id: finding.pack_id,
                evidence_signature: backtest_evidence_signature(&input.event)?,
                matched_fields: backtest_matched_fields(&input.event)?,
                outcome: backtest_outcome(input.expected.as_deref(), "finding"),
            });
        }
    }
    Ok(dedupe_backtest_matches(rows, runtime_backtest_limit(limit)))
}

pub fn backtest_matched_fields(
    event: &SecurityEvent,
) -> Result<Vec<MatchedField>, SecurityEngineError> {
    let mut fields = Vec::new();
    push_common_matched_fields(&mut fields, event)?;
    match &event.subject {
        SecurityEventSubject::Http(subject) => {
            push_matched_field(&mut fields, "http.request.method", &subject.method)?;
            push_matched_field(&mut fields, "http.request.host", &subject.host)?;
            push_matched_field(&mut fields, "http.request.path_class", &subject.path_class)?;
            push_matched_field(&mut fields, "http.request.bytes", subject.request_bytes)?;
            for (name, values) in &subject.request_headers {
                push_matched_field(&mut fields, &format!("http.request.headers.{name}"), values)?;
            }
            if let Some(body) = &subject.request_body {
                push_http_body_matched_fields(&mut fields, "http.request.body", body)?;
            }
            if let Some(value) = &subject.scheme {
                push_matched_field(&mut fields, "http.request.scheme", value)?;
            }
            if let Some(value) = subject.port {
                push_matched_field(&mut fields, "http.request.port", value)?;
            }
            if let Some(value) = &subject.path {
                push_matched_field(&mut fields, "http.request.path", value)?;
            }
            if let Some(value) = &subject.query {
                push_matched_field(&mut fields, "http.request.query", value)?;
            }
            if let Some(value) = &subject.url {
                push_matched_field(&mut fields, "http.request.url", value)?;
            }
            if let Some(value) = subject.response_status {
                push_matched_field(&mut fields, "http.response.status", value)?;
            }
            if let Some(value) = subject.response_bytes {
                push_matched_field(&mut fields, "http.response.bytes", value)?;
            }
            for (name, values) in &subject.response_headers {
                push_matched_field(
                    &mut fields,
                    &format!("http.response.headers.{name}"),
                    values,
                )?;
            }
            if let Some(body) = &subject.response_body {
                push_http_body_matched_fields(&mut fields, "http.response.body", body)?;
            }
        }
        SecurityEventSubject::Dns(subject) => {
            push_matched_field(&mut fields, "dns.request.qname", &subject.qname)?;
            push_matched_field(
                &mut fields,
                "dns.request.domain_class",
                &subject.domain_class,
            )?;
        }
        SecurityEventSubject::Mcp(subject) => {
            push_matched_field(&mut fields, "mcp.request.server_id", &subject.server_id)?;
            push_matched_field(&mut fields, "mcp.request.tool_name", &subject.tool_name)?;
            if let Some(evidence) = &subject.evidence {
                push_matched_field(
                    &mut fields,
                    "mcp.request.arguments_status",
                    mcp_arguments_status(evidence),
                )?;
                push_matched_field(
                    &mut fields,
                    "mcp.request.namespaced_tool_name",
                    &evidence.namespaced_tool_name,
                )?;
                push_matched_field(&mut fields, "mcp.request.transport", &evidence.transport)?;
                if let Some(value) = &evidence.request_arguments_raw {
                    push_matched_field(&mut fields, "mcp.request.arguments_raw", value)?;
                }
                if let Some(value) = &evidence.request_arguments_json {
                    push_matched_field(&mut fields, "mcp.request.arguments_json", value)?;
                }
                push_matched_field(&mut fields, "mcp.response.is_error", evidence.is_error)?;
                push_matched_field(
                    &mut fields,
                    "mcp.response.result_status",
                    if evidence.is_error { "error" } else { "ok" },
                )?;
                push_matched_field(
                    &mut fields,
                    "mcp.response.result_kind",
                    evidence.result_kind,
                )?;
                if let Some(value) = &evidence.result_preview {
                    push_matched_field(&mut fields, "mcp.response.result_preview", value)?;
                }
                if let Some(value) = &evidence.result_json {
                    push_matched_field(&mut fields, "mcp.response.result_json", value)?;
                }
                push_matched_field(&mut fields, "mcp.response.latency_ms", evidence.latency_ms)?;
                push_matched_field(&mut fields, "mcp.link.status", evidence.link_status)?;
                if let Some(value) = &evidence.linked_model_interaction_id {
                    push_matched_field(&mut fields, "mcp.link.model_interaction_id", value)?;
                }
                if let Some(value) = &evidence.linked_model_tool_call_id {
                    push_matched_field(&mut fields, "mcp.link.model_tool_call_id", value)?;
                }
            }
        }
        SecurityEventSubject::Model(subject) => {
            push_matched_field(&mut fields, "model.request.provider", &subject.provider)?;
            push_matched_field(&mut fields, "model.request.model", &subject.model)?;
            if let Some(value) = subject.estimated_input_tokens {
                push_matched_field(&mut fields, "model.usage.input_tokens", value)?;
            }
            if let Some(value) = subject.estimated_output_tokens {
                push_matched_field(&mut fields, "model.usage.output_tokens", value)?;
            }
            if let Some(value) = subject.estimated_cost_micros {
                push_matched_field(&mut fields, "model.usage.estimated_cost_micros", value)?;
            }
            if let Some(evidence) = &subject.evidence {
                push_matched_field(&mut fields, "model.request.api_family", evidence.api_family)?;
                push_matched_field(&mut fields, "model.request.stream", evidence.request.stream)?;
                push_matched_field(
                    &mut fields,
                    "model.request.message_count",
                    evidence.request.message_count,
                )?;
                push_matched_field(
                    &mut fields,
                    "model.request.tools_declared_count",
                    evidence.request.tools_declared_count,
                )?;
                push_matched_field(
                    &mut fields,
                    "model.request.unknown_fields_present",
                    evidence.request.unknown_fields_present,
                )?;
                push_matched_field(
                    &mut fields,
                    "model.evidence.parse_status",
                    evidence.parse_status,
                )?;
                push_matched_field(
                    &mut fields,
                    "model.evidence.status",
                    evidence.evidence_status,
                )?;
                for (index, tool_call) in evidence.tool_calls.iter().enumerate() {
                    let prefix = format!("model.request.tool_calls[{index}]");
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.tool_call_id"),
                        &tool_call.tool_call_id,
                    )?;
                    if let Some(value) = &tool_call.provider_call_id {
                        push_matched_field(
                            &mut fields,
                            &format!("{prefix}.provider_call_id"),
                            value,
                        )?;
                    }
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.raw_name"),
                        &tool_call.raw_name,
                    )?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.name"),
                        &tool_call.normalized_name,
                    )?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.arguments_status"),
                        tool_call.arguments_status,
                    )?;
                    push_matched_field(&mut fields, &format!("{prefix}.origin"), tool_call.origin)?;
                    push_matched_field(&mut fields, &format!("{prefix}.status"), tool_call.status)?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.parse_confidence"),
                        tool_call.parse_confidence,
                    )?;
                    if let Some(value) = &tool_call.linked_mcp_call_id {
                        push_matched_field(
                            &mut fields,
                            &format!("{prefix}.linked_mcp_call_id"),
                            value,
                        )?;
                    }
                    if let Some(value) = &tool_call.arguments_raw {
                        push_matched_field(&mut fields, &format!("{prefix}.arguments_raw"), value)?;
                    }
                    if let Some(value) = &tool_call.arguments_json {
                        push_matched_field(
                            &mut fields,
                            &format!("{prefix}.arguments_json"),
                            value,
                        )?;
                    }
                }
                if let Some(response) = &evidence.response {
                    if let Some(value) = &response.stop_reason {
                        push_matched_field(&mut fields, "model.response.stop_reason", value)?;
                    }
                    if let Some(value) = &response.provider_response_id {
                        push_matched_field(
                            &mut fields,
                            "model.response.provider_response_id",
                            value,
                        )?;
                    }
                }
                for (index, tool_result) in evidence.tool_results.iter().enumerate() {
                    let prefix = format!("model.response.tool_results[{index}]");
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.tool_call_id"),
                        &tool_result.tool_call_id,
                    )?;
                    if let Some(value) = &tool_result.linked_mcp_call_id {
                        push_matched_field(
                            &mut fields,
                            &format!("{prefix}.linked_mcp_call_id"),
                            value,
                        )?;
                    }
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.content_kind"),
                        tool_result.content_kind,
                    )?;
                    if let Some(value) = &tool_result.content_preview {
                        push_matched_field(
                            &mut fields,
                            &format!("{prefix}.content_preview"),
                            value,
                        )?;
                    }
                    if let Some(value) = &tool_result.content_json {
                        push_matched_field(&mut fields, &format!("{prefix}.content_json"), value)?;
                    }
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.is_error"),
                        tool_result.is_error,
                    )?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.result_status"),
                        tool_result.result_status,
                    )?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.returned_to_model"),
                        tool_result.returned_to_model,
                    )?;
                    push_matched_field(
                        &mut fields,
                        &format!("{prefix}.parse_confidence"),
                        tool_result.parse_confidence,
                    )?;
                }
            }
        }
        SecurityEventSubject::File(subject) => {
            push_matched_field(&mut fields, "file.activity.operation", &subject.operation)?;
            push_matched_field(&mut fields, "file.activity.path_class", &subject.path_class)?;
            if let Some(value) = &subject.path {
                push_matched_field(&mut fields, "file.activity.path", value)?;
            }
            if let Some(value) = subject.byte_count {
                push_matched_field(&mut fields, "file.activity.byte_count", value)?;
            }
        }
        SecurityEventSubject::Process(subject) => {
            push_matched_field(
                &mut fields,
                "process.activity.operation",
                &subject.operation,
            )?;
            if let Some(value) = &subject.command_class {
                push_matched_field(&mut fields, "process.activity.command_class", value)?;
            }
        }
        SecurityEventSubject::Credential(subject) => {
            push_matched_field(
                &mut fields,
                "credential.activity.operation",
                &subject.operation,
            )?;
            push_matched_field(
                &mut fields,
                "credential.activity.credential_id",
                &subject.credential_id,
            )?;
        }
        SecurityEventSubject::VmLifecycle(subject) => {
            push_matched_field(&mut fields, "vm.activity.operation", &subject.operation)?;
        }
        SecurityEventSubject::Profile(subject) => {
            push_matched_field(
                &mut fields,
                "profile.activity.operation",
                &subject.operation,
            )?;
            push_matched_field(
                &mut fields,
                "profile.activity.profile_id",
                &subject.profile_id,
            )?;
            push_matched_field(
                &mut fields,
                "profile.activity.profile_revision",
                &subject.profile_revision,
            )?;
            push_matched_field(&mut fields, "profile.id", &subject.profile_id)?;
            push_matched_field(&mut fields, "profile.revision", &subject.profile_revision)?;
        }
        SecurityEventSubject::Conversation(subject) => {
            push_matched_field(
                &mut fields,
                "conversation.activity.operation",
                &subject.operation,
            )?;
            if let Some(value) = &subject.conversation_id {
                push_matched_field(&mut fields, "conversation.id", value)?;
            }
        }
        SecurityEventSubject::Snapshot(subject) => {
            push_matched_field(
                &mut fields,
                "snapshot.activity.operation",
                &subject.operation,
            )?;
            push_matched_field(&mut fields, "snapshot.id", &subject.snapshot_id)?;
        }
    }
    Ok(fields)
}

fn push_common_matched_fields(
    fields: &mut Vec<MatchedField>,
    event: &SecurityEvent,
) -> Result<(), SecurityEngineError> {
    push_matched_field(fields, "common.event_id", &event.common.event_id)?;
    push_matched_field(fields, "common.event_type", &event.common.event_type)?;
    push_matched_field(fields, "common.source_engine", event.common.source_engine)?;
    push_matched_field(fields, "common.enforceability", event.common.enforceability)?;
    push_matched_field(
        fields,
        "common.attribution_scope",
        event.common.attribution_scope,
    )?;
    push_matched_field(fields, "common.origin_kind", event.common.origin_kind)?;
    push_matched_field(
        fields,
        "common.timestamp_unix_ms",
        event.common.timestamp_unix_ms,
    )?;
    if let Some(value) = &event.common.vm_id {
        push_matched_field(fields, "common.vm_id", value)?;
    }
    if let Some(value) = &event.common.session_id {
        push_matched_field(fields, "common.session_id", value)?;
    }
    if let Some(value) = &event.common.profile_id {
        push_matched_field(fields, "common.profile_id", value)?;
    }
    if let Some(value) = &event.common.user_id {
        push_matched_field(fields, "common.user_id", value)?;
    }
    if let Some(value) = &event.common.process_id {
        push_matched_field(fields, "common.process_id", value)?;
    }
    if let Some(value) = &event.common.exec_id {
        push_matched_field(fields, "common.exec_id", value)?;
    }
    if let Some(value) = &event.common.turn_id {
        push_matched_field(fields, "common.turn_id", value)?;
    }
    if let Some(value) = &event.common.message_id {
        push_matched_field(fields, "common.message_id", value)?;
    }
    if let Some(value) = &event.common.tool_call_id {
        push_matched_field(fields, "common.tool_call_id", value)?;
    }
    if let Some(value) = &event.common.mcp_call_id {
        push_matched_field(fields, "common.mcp_call_id", value)?;
    }
    if let Some(value) = &event.common.accounting_owner {
        push_matched_field(fields, "common.accounting_owner", value)?;
    }
    Ok(())
}

fn push_http_body_matched_fields(
    fields: &mut Vec<MatchedField>,
    prefix: &str,
    body: &HttpBodySecuritySubject,
) -> Result<(), SecurityEngineError> {
    push_matched_field(fields, &format!("{prefix}.state"), body.state)?;
    if let Some(value) = &body.text {
        push_matched_field(fields, &format!("{prefix}.text"), value)?;
    }
    if let Some(value) = &body.content_type {
        push_matched_field(fields, &format!("{prefix}.content_type"), value)?;
    }
    if let Some(value) = body.size {
        push_matched_field(fields, &format!("{prefix}.size"), value)?;
    }
    push_matched_field(fields, &format!("{prefix}.truncated"), body.truncated)?;
    if let Some(value) = &body.redaction_reason {
        push_matched_field(fields, &format!("{prefix}.redaction_reason"), value)?;
    }
    Ok(())
}

fn mcp_arguments_status(evidence: &McpToolExecutionEvidence) -> &'static str {
    if evidence.request_arguments_json.is_some() {
        "valid_json"
    } else if evidence.request_arguments_raw.is_some() {
        "not_json"
    } else {
        "absent"
    }
}

fn push_matched_field(
    fields: &mut Vec<MatchedField>,
    path: &str,
    value: impl Serialize,
) -> Result<(), SecurityEngineError> {
    fields.push(MatchedField {
        path: path.to_owned(),
        value: serde_json::to_value(value).map_err(|error| SecurityEngineError::PhaseFailed {
            phase: SecurityEnginePhase::Detection,
            message: format!("serialize backtest matched field {path}: {error}"),
        })?,
    });
    Ok(())
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
    #[serde(default = "default_runtime_rule_priority")]
    pub priority: i32,
}

pub const DEFAULT_RUNTIME_RULE_PRIORITY: i32 = 100;

pub fn default_runtime_rule_priority() -> i32 {
    DEFAULT_RUNTIME_RULE_PRIORITY
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeRuleDefinition {
    Enforcement {
        decision: SecurityDecisionAction,
        #[serde(default)]
        reason: Option<String>,
    },
    Detection {
        #[serde(default)]
        sigma_id: Option<String>,
        title: String,
        severity: Severity,
        confidence: Confidence,
        #[serde(default)]
        tags: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRuleRecord {
    pub metadata: RuntimeRuleMetadata,
    pub definition: RuntimeRuleDefinition,
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
    pub definition: RuntimeRuleDefinition,
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
                definition: record.definition,
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

    pub fn enabled_enforcement_rules(&self) -> Vec<CelEnforcementRule> {
        self.enabled_rules_by_priority()
            .into_iter()
            .filter_map(|entry| match &entry.definition {
                RuntimeRuleDefinition::Enforcement { decision, reason } => {
                    Some(CelEnforcementRule {
                        id: entry.metadata.id.clone(),
                        pack_id: entry.metadata.pack_id.clone(),
                        condition: entry.source.clone(),
                        decision: *decision,
                        reason: reason.clone(),
                        mutations: Vec::new(),
                    })
                }
                RuntimeRuleDefinition::Detection { .. } => None,
            })
            .collect()
    }

    pub fn enabled_detection_rules(&self) -> Vec<CelDetectionRule> {
        self.enabled_rules_by_priority()
            .into_iter()
            .filter_map(|entry| match &entry.definition {
                RuntimeRuleDefinition::Detection {
                    sigma_id,
                    title,
                    severity,
                    confidence,
                    tags,
                } => Some(CelDetectionRule {
                    id: entry.metadata.id.clone(),
                    pack_id: entry
                        .metadata
                        .pack_id
                        .clone()
                        .unwrap_or_else(|| "runtime".into()),
                    sigma_id: sigma_id.clone(),
                    title: title.clone(),
                    condition: entry.source.clone(),
                    severity: *severity,
                    confidence: *confidence,
                    tags: tags.clone(),
                }),
                RuntimeRuleDefinition::Enforcement { .. } => None,
            })
            .collect()
    }

    fn enabled_rules_by_priority(&self) -> Vec<&RuntimeRuleEntry> {
        let mut entries = self
            .rules
            .values()
            .filter(|entry| entry.enabled)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.metadata
                .priority
                .cmp(&right.metadata.priority)
                .then_with(|| left.metadata.id.cmp(&right.metadata.id))
        });
        entries
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

impl RuleMatchRecorder for RuntimeRuleRegistry {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), SecurityEngineError> {
        self.record_match(rule_id, event_id, timestamp_unix_ms)
            .map_err(|error| SecurityEngineError::PhaseFailed {
                phase: SecurityEnginePhase::Detection,
                message: error.to_string(),
            })
    }
}

impl RuleMatchRecorder for std::sync::Arc<std::sync::Mutex<RuntimeRuleRegistry>> {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), SecurityEngineError> {
        let mut registry = self
            .lock()
            .map_err(|error| SecurityEngineError::PhaseFailed {
                phase: SecurityEnginePhase::Detection,
                message: format!("runtime rule registry lock poisoned: {error}"),
            })?;
        registry.record_rule_match(rule_id, event_id, timestamp_unix_ms)
    }
}

#[cfg(test)]
mod tests;
