use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEventCommon {
    pub event_id: String,
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
    pub common: SecurityEventCommon,
    pub subject: SecurityEventSubject,
}

impl SecurityEvent {
    pub fn dns(common: SecurityEventCommon, subject: DnsSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Dns(subject),
        }
    }

    pub fn http(common: SecurityEventCommon, subject: HttpSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Http(subject),
        }
    }

    pub fn mcp(common: SecurityEventCommon, subject: McpSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Mcp(subject),
        }
    }

    pub fn model(common: SecurityEventCommon, subject: ModelSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Model(subject),
        }
    }

    pub fn file(common: SecurityEventCommon, subject: FileSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::File(subject),
        }
    }

    pub fn process(common: SecurityEventCommon, subject: ProcessSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Process(subject),
        }
    }

    pub fn conversation(common: SecurityEventCommon, subject: ConversationSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Conversation(subject),
        }
    }

    pub fn snapshot(common: SecurityEventCommon, subject: SnapshotSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Snapshot(subject),
        }
    }

    pub fn vm_lifecycle(common: SecurityEventCommon, subject: VmLifecycleSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::VmLifecycle(subject),
        }
    }

    pub fn profile(common: SecurityEventCommon, subject: ProfileSecuritySubject) -> Self {
        Self {
            common,
            subject: SecurityEventSubject::Profile(subject),
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
            event_family: self.event_family(),
            event_type: self.common.event_type.clone(),
            correlation_ids: CorrelationIds {
                trace_id: self.common.trace_id.clone(),
                span_id: self.common.span_id.clone(),
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
            }
            Self::Model(subject) => {
                dimensions.provider = Some(subject.provider.clone());
                dimensions.model = Some(subject.model.clone());
                dimensions.estimated_input_tokens = subject.estimated_input_tokens;
                dimensions.estimated_output_tokens = subject.estimated_output_tokens;
                dimensions.estimated_cost_micros = subject.estimated_cost_micros;
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationIds {
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
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
    pub event_family: EventFamily,
    pub event_type: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mcp_server: Option<String>,
    #[serde(default)]
    pub mcp_tool: Option<String>,
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
            event_family,
            event_type,
            provider: None,
            model: None,
            mcp_server: None,
            mcp_tool: None,
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
    pub event: SecurityEvent,
    #[serde(default)]
    pub steps: Vec<ResolvedEventStep>,
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

#[cfg(test)]
mod tests;
