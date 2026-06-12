use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use capsem_logger::{
    AuditEvent, DbWriter, ExecEvent, ExecEventComplete, FileAction, FileEvent, SecurityAskEvent,
    SecurityAskPending, SecurityAskStatus, SecurityDecision as LoggedSecurityDecision,
    SecurityDecisionEvent, SecurityDecisionStage as LoggedSecurityDecisionStage,
    SecurityDetectionLevel as LoggedDetectionLevel, SecurityRuleAction as LoggedRuleAction,
    SecurityRuleEvent, SubstitutionEvent, WriteOp,
};
use serde::Serialize;
use serde_json::json;
use tracing::Instrument;
use uuid::Uuid;

use crate::credential_broker::{BrokeredUpstreamCredentials, CredentialObservation};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::{
    CompiledSecurityRule, DetectionLevel, PolicyActionId, PolicySubject, PolicySubjectValue,
    SecurityPluginConfig, SecurityPluginMode, SecurityRuleAction, SecurityRuleSet,
};

pub const SECURITY_EVENT_EMIT_SPAN: &str = "capsem.security_event.emit";
pub const SECURITY_EVENT_EMIT_TOTAL: &str = "security_event.emit_total";
pub const SECURITY_EVENT_EMIT_DURATION_MS: &str = "security_event.emit_duration_ms";
pub const DUMMY_EICAR_TEST_STRING: &str =
    r#"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeSecurityEventFamily {
    Http,
    Model,
    Mcp,
    Dns,
    File,
    Process,
    Credential,
    Security,
}

impl RuntimeSecurityEventFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            RuntimeSecurityEventFamily::Http => "http",
            RuntimeSecurityEventFamily::Model => "model",
            RuntimeSecurityEventFamily::Mcp => "mcp",
            RuntimeSecurityEventFamily::Dns => "dns",
            RuntimeSecurityEventFamily::File => "file",
            RuntimeSecurityEventFamily::Process => "process",
            RuntimeSecurityEventFamily::Credential => "credential",
            RuntimeSecurityEventFamily::Security => "security",
        }
    }

    pub const fn is_first_party_cel_root(self) -> bool {
        matches!(
            self,
            RuntimeSecurityEventFamily::Http
                | RuntimeSecurityEventFamily::Model
                | RuntimeSecurityEventFamily::Mcp
                | RuntimeSecurityEventFamily::Dns
                | RuntimeSecurityEventFamily::File
                | RuntimeSecurityEventFamily::Process
        )
    }

    pub const fn is_ledger_only(self) -> bool {
        matches!(self, RuntimeSecurityEventFamily::Credential)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeSecurityEventType {
    HttpRequest,
    ModelCall,
    McpToolCall,
    McpToolList,
    /// Intentionally supported for MCP methods that are neither tool calls nor
    /// tool listing, including resource and future MCP control messages.
    McpEvent,
    DnsQuery,
    FileEvent,
    FileImport,
    FileExport,
    ProcessExec,
    ProcessExecComplete,
    ProcessAudit,
    CredentialSubstitution,
    SecurityRule,
    SecurityAsk,
}

impl RuntimeSecurityEventType {
    pub const ALL: &'static [Self] = &[
        Self::HttpRequest,
        Self::ModelCall,
        Self::McpToolCall,
        Self::McpToolList,
        Self::McpEvent,
        Self::DnsQuery,
        Self::FileEvent,
        Self::FileImport,
        Self::FileExport,
        Self::ProcessExec,
        Self::ProcessExecComplete,
        Self::ProcessAudit,
        Self::CredentialSubstitution,
        Self::SecurityRule,
        Self::SecurityAsk,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            RuntimeSecurityEventType::HttpRequest => "http.request",
            RuntimeSecurityEventType::ModelCall => "model.call",
            RuntimeSecurityEventType::McpToolCall => "mcp.tool_call",
            RuntimeSecurityEventType::McpToolList => "mcp.tool_list",
            RuntimeSecurityEventType::McpEvent => "mcp.event",
            RuntimeSecurityEventType::DnsQuery => "dns.query",
            RuntimeSecurityEventType::FileEvent => "file.event",
            RuntimeSecurityEventType::FileImport => "file.import",
            RuntimeSecurityEventType::FileExport => "file.export",
            RuntimeSecurityEventType::ProcessExec => "process.exec",
            RuntimeSecurityEventType::ProcessExecComplete => "process.exec_complete",
            RuntimeSecurityEventType::ProcessAudit => "process.audit",
            RuntimeSecurityEventType::CredentialSubstitution => "credential.substitution",
            RuntimeSecurityEventType::SecurityRule => "security.rule",
            RuntimeSecurityEventType::SecurityAsk => "security.ask",
        }
    }

    pub const fn family(self) -> RuntimeSecurityEventFamily {
        match self {
            RuntimeSecurityEventType::HttpRequest => RuntimeSecurityEventFamily::Http,
            RuntimeSecurityEventType::ModelCall => RuntimeSecurityEventFamily::Model,
            RuntimeSecurityEventType::McpToolCall
            | RuntimeSecurityEventType::McpToolList
            | RuntimeSecurityEventType::McpEvent => RuntimeSecurityEventFamily::Mcp,
            RuntimeSecurityEventType::DnsQuery => RuntimeSecurityEventFamily::Dns,
            RuntimeSecurityEventType::FileEvent
            | RuntimeSecurityEventType::FileImport
            | RuntimeSecurityEventType::FileExport => RuntimeSecurityEventFamily::File,
            RuntimeSecurityEventType::ProcessExec
            | RuntimeSecurityEventType::ProcessExecComplete
            | RuntimeSecurityEventType::ProcessAudit => RuntimeSecurityEventFamily::Process,
            RuntimeSecurityEventType::CredentialSubstitution => {
                RuntimeSecurityEventFamily::Credential
            }
            RuntimeSecurityEventType::SecurityRule => RuntimeSecurityEventFamily::Security,
            RuntimeSecurityEventType::SecurityAsk => RuntimeSecurityEventFamily::Security,
        }
    }

    pub const fn uses_ledger_only_family(self) -> bool {
        self.family().is_ledger_only()
    }

    pub fn parse_str(value: &str) -> Result<Self, SecurityEventTypeParseError> {
        match value {
            "http.request" => Ok(Self::HttpRequest),
            "model.call" => Ok(Self::ModelCall),
            "mcp.tool_call" => Ok(Self::McpToolCall),
            "mcp.tool_list" => Ok(Self::McpToolList),
            "mcp.event" => Ok(Self::McpEvent),
            "dns.query" => Ok(Self::DnsQuery),
            "file.event" => Ok(Self::FileEvent),
            "file.import" => Ok(Self::FileImport),
            "file.export" => Ok(Self::FileExport),
            "process.exec" => Ok(Self::ProcessExec),
            "process.exec_complete" => Ok(Self::ProcessExecComplete),
            "process.audit" => Ok(Self::ProcessAudit),
            "credential.substitution" => Ok(Self::CredentialSubstitution),
            "security.rule" => Ok(Self::SecurityRule),
            "security.ask" => Ok(Self::SecurityAsk),
            other => Err(SecurityEventTypeParseError {
                value: other.to_string(),
            }),
        }
    }

    fn for_write_op(op: &WriteOp) -> Self {
        match op {
            WriteOp::NetEvent(_) => Self::HttpRequest,
            WriteOp::ModelCall(_) => Self::ModelCall,
            WriteOp::McpCall(call) => match call.method.as_str() {
                "tools/call" => Self::McpToolCall,
                "tools/list" => Self::McpToolList,
                _ => Self::McpEvent,
            },
            WriteOp::FileEvent(event) => runtime_file_event_type(event.action),
            WriteOp::ExecEvent(_) => Self::ProcessExec,
            WriteOp::ExecEventComplete(_) => Self::ProcessExecComplete,
            WriteOp::AuditEvent(_) => Self::ProcessAudit,
            WriteOp::DnsEvent(_) => Self::DnsQuery,
            WriteOp::SubstitutionEvent(_) => Self::CredentialSubstitution,
            WriteOp::SecurityRuleEvent(_) => Self::SecurityRule,
            WriteOp::SecurityAskEvent(_) => Self::SecurityAsk,
            WriteOp::SecurityDecisionEvent(_) => Self::SecurityRule,
            WriteOp::ProfileMutationEvent(_) => Self::SecurityRule,
        }
    }
}

impl TryFrom<&str> for RuntimeSecurityEventType {
    type Error = SecurityEventTypeParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEventTypeParseError {
    value: String,
}

impl fmt::Display for SecurityEventTypeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown runtime security event type '{}'", self.value)
    }
}

impl std::error::Error for SecurityEventTypeParseError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecurityEventId(String);

impl SecurityEventId {
    pub fn new_uuid4() -> Self {
        let value = Uuid::new_v4().simple().to_string();
        Self(value[..12].to_string())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() == 12
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err("security event id must be 12 lowercase hex characters".to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSecurityEvent {
    pub event_id: Option<SecurityEventId>,
    pub event_type: RuntimeSecurityEventType,
    pub event_family: RuntimeSecurityEventFamily,
    pub credential_ref: Option<String>,
    pub trace_id: Option<String>,
    logger_write: WriteOp,
}

impl RuntimeSecurityEvent {
    pub fn from_logger_write(mut logger_write: WriteOp) -> Self {
        let event_id = logger_write
            .ensure_event_id()
            .and_then(|value| SecurityEventId::parse(value).ok());
        let event_type = RuntimeSecurityEventType::for_write_op(&logger_write);
        let event_family = event_type.family();
        let credential_ref = logger_write_credential_ref(&logger_write);
        let trace_id = logger_write_trace_id(&logger_write);
        Self {
            event_id,
            event_type,
            event_family,
            credential_ref,
            trace_id,
            logger_write,
        }
    }

    pub fn into_logger_write(self) -> WriteOp {
        self.logger_write
    }
}

pub async fn emit_security_write(db: &DbWriter, op: WriteOp) -> Option<SecurityEventId> {
    let event = RuntimeSecurityEvent::from_logger_write(op);
    let event_type = event.event_type.as_str();
    let event_family = event.event_family.as_str();
    let span = tracing::debug_span!(
        target: "capsem.security_event",
        SECURITY_EVENT_EMIT_SPAN,
        event_type,
        event_family,
        status = tracing::field::Empty,
        queue_result = tracing::field::Empty,
    );
    let started = Instant::now();
    span.in_scope(|| trace_runtime_security_event(&event));
    let event_id = event.event_id.clone();
    db.write(event.into_logger_write())
        .instrument(span.clone())
        .await;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(SECURITY_EVENT_EMIT_TOTAL,
        "event_type" => event_type,
        "event_family" => event_family,
        "status" => "ok",
        "queue_result" => "queued")
    .increment(1);
    ::metrics::histogram!(SECURITY_EVENT_EMIT_DURATION_MS,
        "event_type" => event_type,
        "event_family" => event_family)
    .record(elapsed_ms);
    span.record("status", "ok");
    span.record("queue_result", "queued");
    event_id
}

pub fn emit_security_write_blocking(db: &DbWriter, op: WriteOp) -> Option<SecurityEventId> {
    let event = RuntimeSecurityEvent::from_logger_write(op);
    let event_type = event.event_type.as_str();
    let event_family = event.event_family.as_str();
    let span = tracing::debug_span!(
        target: "capsem.security_event",
        SECURITY_EVENT_EMIT_SPAN,
        event_type,
        event_family,
        status = tracing::field::Empty,
        queue_result = tracing::field::Empty,
    );
    let started = Instant::now();
    span.in_scope(|| trace_runtime_security_event(&event));
    let event_id = event.event_id.clone();
    span.in_scope(|| db.write_blocking(event.into_logger_write()));
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(SECURITY_EVENT_EMIT_TOTAL,
        "event_type" => event_type,
        "event_family" => event_family,
        "status" => "ok",
        "queue_result" => "queued")
    .increment(1);
    ::metrics::histogram!(SECURITY_EVENT_EMIT_DURATION_MS,
        "event_type" => event_type,
        "event_family" => event_family)
    .record(elapsed_ms);
    span.record("status", "ok");
    span.record("queue_result", "queued");
    event_id
}

pub async fn emit_file_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: FileEvent,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_file_event(&event);
    let event_type = runtime_file_event_type(event.action);
    let event_id = emit_security_write(db, WriteOp::FileEvent(event)).await?;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        event_type,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(error = %error, "failed to emit file security rule ledger rows");
    }
    Some(event_id)
}

pub struct ExplicitFileSecurityEvent {
    pub action: FileAction,
    pub path: String,
    pub size: Option<u64>,
    pub content: Option<String>,
    pub mime_type: Option<String>,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
}

pub async fn emit_explicit_file_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: ExplicitFileSecurityEvent,
) -> Option<SecurityEventId> {
    let primary = FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: event.action,
        path: event.path.clone(),
        size: event.size,
        trace_id: event.trace_id.clone(),
        credential_ref: event.credential_ref.clone(),
    };
    let security_event = security_event_from_explicit_file_event(&event);
    let event_type = runtime_file_event_type(event.action);
    let event_id = emit_security_write(db, WriteOp::FileEvent(primary)).await?;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        event_type,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(error = %error, "failed to emit explicit file security rule ledger rows");
    }
    Some(event_id)
}

pub fn emit_file_security_write_and_rules_blocking(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: FileEvent,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_file_event(&event);
    let event_type = runtime_file_event_type(event.action);
    let event_id = emit_security_write_blocking(db, WriteOp::FileEvent(event))?;
    if let Err(error) = emit_matching_security_rules_blocking(
        db,
        event_id.clone(),
        event_type,
        rules,
        &security_event,
        current_unix_ms(),
    ) {
        tracing::warn!(error = %error, "failed to emit file security rule ledger rows");
    }
    Some(event_id)
}

pub const fn runtime_file_event_type(action: FileAction) -> RuntimeSecurityEventType {
    match action {
        FileAction::Imported => RuntimeSecurityEventType::FileImport,
        FileAction::Exported => RuntimeSecurityEventType::FileExport,
        FileAction::Created
        | FileAction::Modified
        | FileAction::Deleted
        | FileAction::Restored
        | FileAction::Read => RuntimeSecurityEventType::FileEvent,
    }
}

pub fn security_event_from_file_event(event: &FileEvent) -> SecurityEvent {
    let mut file = FileSecurityEvent::default();
    let path = Some(event.path.clone());
    let name = file_name(&event.path);
    let ext = file_ext(&event.path);
    match event.action {
        FileAction::Created => {
            file.create_path = path;
            file.create_name = name;
            file.create_ext = ext;
        }
        FileAction::Modified | FileAction::Restored => {
            file.write_path = path;
            file.write_name = name;
            file.write_ext = ext;
        }
        FileAction::Deleted => {
            file.delete_path = path;
            file.delete_name = name;
            file.delete_ext = ext;
        }
        FileAction::Read => {
            file.read_path = path;
            file.read_name = name;
            file.read_ext = ext;
        }
        FileAction::Imported => {
            file.import_path = path;
            file.import_name = name;
            file.import_ext = ext;
        }
        FileAction::Exported => {
            file.export_path = path;
            file.export_name = name;
            file.export_ext = ext;
        }
    }
    let security_event = SecurityEvent::new(runtime_file_event_type(event.action)).with_file(file);
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

pub fn security_event_from_explicit_file_event(event: &ExplicitFileSecurityEvent) -> SecurityEvent {
    let mut file = FileSecurityEvent::default();
    let path = Some(event.path.clone());
    let name = file_name(&event.path);
    let ext = file_ext(&event.path);
    let mime_type = event.mime_type.clone();
    let content = event.content.clone();
    file.content = content.clone();
    match event.action {
        FileAction::Created => {
            file.create_path = path;
            file.create_name = name;
            file.create_ext = ext;
            file.create_mime_type = mime_type;
            file.create_content = content;
        }
        FileAction::Modified | FileAction::Restored => {
            file.write_path = path;
            file.write_name = name;
            file.write_ext = ext;
            file.write_mime_type = mime_type;
            file.write_content = content;
        }
        FileAction::Deleted => {
            file.delete_path = path;
            file.delete_name = name;
            file.delete_ext = ext;
            file.delete_mime_type = mime_type;
            file.delete_content = content;
        }
        FileAction::Read => {
            file.read_path = path;
            file.read_name = name;
            file.read_ext = ext;
            file.read_mime_type = mime_type;
            file.read_content = content;
        }
        FileAction::Imported => {
            file.import_path = path;
            file.import_name = name;
            file.import_ext = ext;
            file.import_mime_type = mime_type;
            file.import_content = content;
        }
        FileAction::Exported => {
            file.export_path = path;
            file.export_name = name;
            file.export_ext = ext;
            file.export_mime_type = mime_type;
            file.export_content = content;
        }
    }
    let security_event = SecurityEvent::new(runtime_file_event_type(event.action)).with_file(file);
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

pub async fn emit_process_exec_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: ExecEvent,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_exec_event(&event);
    let event_id = emit_security_write(db, WriteOp::ExecEvent(event)).await?;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        RuntimeSecurityEventType::ProcessExec,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(error = %error, "failed to emit process exec security rule ledger rows");
    }
    Some(event_id)
}

pub async fn emit_process_complete_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event_id: SecurityEventId,
    event: ExecEventComplete,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_exec_complete_event(&event);
    emit_security_write(db, WriteOp::ExecEventComplete(event)).await;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        RuntimeSecurityEventType::ProcessExecComplete,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to emit process exec-complete security rule ledger rows"
        );
    }
    Some(event_id)
}

pub async fn emit_process_complete_security_write_only(
    db: &DbWriter,
    event: ExecEventComplete,
) -> Option<SecurityEventId> {
    emit_security_write(db, WriteOp::ExecEventComplete(event)).await
}

pub fn emit_process_audit_security_write_and_rules_blocking(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: AuditEvent,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_audit_event(&event);
    let event_id = emit_security_write_blocking(db, WriteOp::AuditEvent(event))?;
    if let Err(error) = emit_matching_security_rules_blocking(
        db,
        event_id.clone(),
        RuntimeSecurityEventType::ProcessAudit,
        rules,
        &security_event,
        current_unix_ms(),
    ) {
        tracing::warn!(error = %error, "failed to emit process audit security rule ledger rows");
    }
    Some(event_id)
}

pub async fn emit_substitution_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: SubstitutionEvent,
) -> Option<SecurityEventId> {
    let security_event = security_event_from_substitution_event(&event);
    let event_id = emit_security_write(db, WriteOp::SubstitutionEvent(event)).await?;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        RuntimeSecurityEventType::CredentialSubstitution,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to emit credential substitution security rule ledger rows"
        );
    }
    Some(event_id)
}

pub fn security_event_from_exec_event(event: &ExecEvent) -> SecurityEvent {
    let security_event = SecurityEvent::new(RuntimeSecurityEventType::ProcessExec).with_process(
        ProcessSecurityEvent {
            exec_id: Some(event.exec_id.to_string()),
            exec_path: None,
            command: Some(event.command.clone()),
            exit_code: None,
            stdout: None,
            stderr: None,
        },
    );
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

pub fn security_event_from_exec_complete_event(event: &ExecEventComplete) -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::ProcessExecComplete).with_process(
        ProcessSecurityEvent {
            exec_id: Some(event.exec_id.to_string()),
            exec_path: None,
            command: None,
            exit_code: Some(event.exit_code.to_string()),
            stdout: event.stdout_preview.clone(),
            stderr: event.stderr_preview.clone(),
        },
    )
}

pub fn security_event_from_audit_event(event: &AuditEvent) -> SecurityEvent {
    let security_event = SecurityEvent::new(RuntimeSecurityEventType::ProcessAudit).with_process(
        ProcessSecurityEvent {
            exec_id: event.audit_id.clone(),
            exec_path: Some(event.exe.clone()),
            command: Some(event.argv.clone()),
            exit_code: None,
            stdout: None,
            stderr: None,
        },
    );
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

pub fn security_event_from_substitution_event(event: &SubstitutionEvent) -> SecurityEvent {
    let security_event = SecurityEvent::new(RuntimeSecurityEventType::CredentialSubstitution)
        .with_credential_ref(event.substitution_ref.clone());
    match event.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

fn file_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn file_ext(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub async fn emit_matching_security_rules(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<usize, String> {
    emit_matching_security_rules_with_decision(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
    )
    .await
    .map(|emission| emission.emitted)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityRuleEmission {
    pub emitted: usize,
    pub enforcement: SecurityEnforcementDecision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityBoundaryEvaluation {
    pub event: SecurityEvent,
    pub enforcement: SecurityEnforcementDecision,
    pub matched_rule_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEnforcementDecision {
    pub action: SecurityEnforcementAction,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub reason: Option<String>,
    pub ask_id: Option<SecurityEventId>,
}

impl SecurityEnforcementDecision {
    pub fn allow() -> Self {
        Self {
            action: SecurityEnforcementAction::Allow,
            rule_id: None,
            rule_name: None,
            reason: None,
            ask_id: None,
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self.action, SecurityEnforcementAction::Allow)
    }

    pub fn with_ask_resolution(
        &self,
        resolution: &SecurityAskEvent,
    ) -> Result<Self, SecurityActionError> {
        if !matches!(self.action, SecurityEnforcementAction::Ask) {
            return Err(SecurityActionError::new(
                "only ask enforcement decisions can consume ask resolutions",
            ));
        }
        if self.ask_id.as_ref().map(SecurityEventId::as_str) != Some(resolution.ask_id.as_str()) {
            return Err(SecurityActionError::new(format!(
                "ask resolution '{}' does not match enforcement ask id",
                resolution.ask_id
            )));
        }
        match resolution.status {
            SecurityAskStatus::Pending => Err(SecurityActionError::new(format!(
                "ask '{}' is still pending",
                resolution.ask_id
            ))),
            SecurityAskStatus::Approved => Ok(Self {
                action: SecurityEnforcementAction::Allow,
                rule_id: self.rule_id.clone(),
                rule_name: self.rule_name.clone(),
                reason: resolution.reason.clone().or_else(|| self.reason.clone()),
                ask_id: self.ask_id.clone(),
            }),
            SecurityAskStatus::Denied => Ok(Self {
                action: SecurityEnforcementAction::Block,
                rule_id: self.rule_id.clone(),
                rule_name: self.rule_name.clone(),
                reason: resolution.reason.clone().or_else(|| self.reason.clone()),
                ask_id: self.ask_id.clone(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEnforcementAction {
    Allow,
    Ask,
    Block,
}

pub async fn emit_matching_security_rules_with_decision(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityRuleEmission, String> {
    let evaluation = rules.evaluate(event)?;
    let selected_rule = selected_enforcement_rule(&evaluation);
    let mut enforcement = security_enforcement_decision(selected_rule);
    let mut emitted = 0;
    let enriched_event = event_with_rule_detections(event, evaluation.detections());
    let mut decision_state = enriched_event.decision.clone();
    for rule in evaluation.matched_rules() {
        emit_security_decision_transition(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            &mut decision_state,
            timestamp_unix_ms,
        )
        .await?;
        emit_security_rule_match(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )
        .await?;
        emitted += 1;
    }
    if matches!(enforcement.action, SecurityEnforcementAction::Ask) {
        let Some(rule) = selected_rule else {
            return Err("ask enforcement decision did not carry a rule".to_string());
        };
        let ask_id = emit_security_ask_pending(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )
        .await?;
        enforcement.ask_id = Some(ask_id);
    }
    Ok(SecurityRuleEmission {
        emitted,
        enforcement,
    })
}

pub fn emit_matching_security_rules_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<usize, String> {
    emit_matching_security_rules_with_decision_blocking(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
    )
    .map(|emission| emission.emitted)
}

pub fn emit_matching_security_rules_with_decision_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityRuleEmission, String> {
    let evaluation = rules.evaluate(event)?;
    let selected_rule = selected_enforcement_rule(&evaluation);
    let mut enforcement = security_enforcement_decision(selected_rule);
    let mut emitted = 0;
    let enriched_event = event_with_rule_detections(event, evaluation.detections());
    let mut decision_state = enriched_event.decision.clone();
    for rule in evaluation.matched_rules() {
        emit_security_decision_transition_blocking(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            &mut decision_state,
            timestamp_unix_ms,
        )?;
        emit_security_rule_match_blocking(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )?;
        emitted += 1;
    }
    if matches!(enforcement.action, SecurityEnforcementAction::Ask) {
        let Some(rule) = selected_rule else {
            return Err("ask enforcement decision did not carry a rule".to_string());
        };
        let ask_id = emit_security_ask_pending_blocking(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )?;
        enforcement.ask_id = Some(ask_id);
    }
    Ok(SecurityRuleEmission {
        emitted,
        enforcement,
    })
}

fn requested_decision_for_rule(action: SecurityRuleAction) -> SecurityDecisionKind {
    match action {
        SecurityRuleAction::Allow
        | SecurityRuleAction::Preprocess
        | SecurityRuleAction::Rewrite
        | SecurityRuleAction::Postprocess => SecurityDecisionKind::Allow,
        SecurityRuleAction::Ask => SecurityDecisionKind::Ask,
        SecurityRuleAction::Block => SecurityDecisionKind::Block,
    }
}

fn decision_stage_for_rule(action: SecurityRuleAction) -> LoggedSecurityDecisionStage {
    match action {
        SecurityRuleAction::Preprocess => LoggedSecurityDecisionStage::Preprocess,
        SecurityRuleAction::Rewrite => LoggedSecurityDecisionStage::Rewrite,
        SecurityRuleAction::Postprocess => LoggedSecurityDecisionStage::Postprocess,
        SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block => {
            LoggedSecurityDecisionStage::Rule
        }
    }
}

fn security_decision_event(
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    decision_state: &mut SecurityDecisionState,
    timestamp_unix_ms: i64,
) -> Result<SecurityDecisionEvent, String> {
    let requested = requested_decision_for_rule(rule.action);
    let (previous, effective) = decision_state.request(requested);
    Ok(SecurityDecisionEvent {
        timestamp_unix_ms,
        event_id: event_id.as_str().to_string(),
        event_type: event_type.as_str().to_string(),
        stage: decision_stage_for_rule(rule.action),
        actor: rule.rule_id.clone(),
        rule_id: Some(rule.rule_id.clone()),
        plugin_id: None,
        previous_decision: previous.into(),
        requested_decision: requested.into(),
        effective_decision: effective.into(),
        reason: rule.reason.clone(),
        event_json: serde_json::to_string(&security_event_forensic_json(event))
            .map_err(|error| format!("serialize security decision event payload: {error}"))?,
        trace_id: event.trace_id(),
    })
}

fn record_rule_detection(event: &mut SecurityEvent, rule: &CompiledSecurityRule) {
    let Some(detection_level) = rule.detection_level else {
        return;
    };
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Rule,
        detection_level,
        rule_id: Some(rule.rule_id.clone()),
        plugin_id: None,
        action: Some(rule.action),
        plugin_mode: None,
        reason: rule.reason.clone(),
    });
}

fn event_with_rule_detections<'a>(
    event: &SecurityEvent,
    rules: impl IntoIterator<Item = &'a CompiledSecurityRule>,
) -> SecurityEvent {
    let mut enriched = event.clone();
    for rule in rules {
        record_rule_detection(&mut enriched, rule);
    }
    enriched
}

pub async fn emit_security_decision_transition(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    decision_state: &mut SecurityDecisionState,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let decision_event = security_decision_event(
        event_id,
        event_type,
        rule,
        event,
        decision_state,
        timestamp_unix_ms,
    )?;
    emit_security_write(db, WriteOp::SecurityDecisionEvent(decision_event)).await;
    Ok(())
}

pub fn emit_security_decision_transition_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    decision_state: &mut SecurityDecisionState,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let decision_event = security_decision_event(
        event_id,
        event_type,
        rule,
        event,
        decision_state,
        timestamp_unix_ms,
    )?;
    emit_security_write_blocking(db, WriteOp::SecurityDecisionEvent(decision_event));
    Ok(())
}

fn selected_enforcement_rule<'a>(
    evaluation: &'a crate::net::policy_config::SecurityRuleEvaluation<'a>,
) -> Option<&'a CompiledSecurityRule> {
    evaluation.enforcement_rules().into_iter().next()
}

fn security_enforcement_decision(
    rule: Option<&CompiledSecurityRule>,
) -> SecurityEnforcementDecision {
    let Some(rule) = rule else {
        return SecurityEnforcementDecision::allow();
    };
    SecurityEnforcementDecision {
        action: match rule.action {
            SecurityRuleAction::Allow => SecurityEnforcementAction::Allow,
            SecurityRuleAction::Ask => SecurityEnforcementAction::Ask,
            SecurityRuleAction::Block => SecurityEnforcementAction::Block,
            SecurityRuleAction::Preprocess
            | SecurityRuleAction::Rewrite
            | SecurityRuleAction::Postprocess => SecurityEnforcementAction::Allow,
        },
        rule_id: Some(rule.rule_id.clone()),
        rule_name: Some(rule.name.clone()),
        reason: rule.reason.clone(),
        ask_id: None,
    }
}

pub fn evaluate_security_boundary(
    rules: &SecurityRuleSet,
    plugin_policy: BTreeMap<String, SecurityPluginConfig>,
    mut event: SecurityEvent,
) -> Result<SecurityBoundaryEvaluation, SecurityActionError> {
    let action_registry =
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(plugin_policy);

    event = action_registry.apply_security_plugins(SecurityPluginStage::PreDecision, event)?;

    let evaluation = rules.evaluate(&event).map_err(SecurityActionError::new)?;
    for rule in evaluation.matched_rules() {
        record_rule_detection(&mut event, rule);
    }

    let selected_rule = selected_enforcement_rule(&evaluation);
    if let Some(rule) = selected_rule {
        event.request_decision(requested_decision_for_rule(rule.action));
    }
    let mut enforcement = security_enforcement_decision(selected_rule);
    if matches!(event.decision.effective, SecurityDecisionKind::Block) {
        enforcement.action = SecurityEnforcementAction::Block;
    } else if matches!(event.decision.effective, SecurityDecisionKind::Ask)
        && matches!(enforcement.action, SecurityEnforcementAction::Allow)
    {
        enforcement.action = SecurityEnforcementAction::Ask;
    }

    event = action_registry.apply_security_plugins(SecurityPluginStage::PostDecision, event)?;
    if matches!(event.decision.effective, SecurityDecisionKind::Block) {
        enforcement.action = SecurityEnforcementAction::Block;
    }

    Ok(SecurityBoundaryEvaluation {
        event,
        enforcement,
        matched_rule_count: evaluation.matched_rules().len(),
    })
}

pub async fn emit_security_rule_match(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let rule_event = security_rule_event(event_id, event_type, rule, event, timestamp_unix_ms)?;
    trace_security_rule_match(&rule_event, rule);
    emit_security_write(db, WriteOp::SecurityRuleEvent(rule_event)).await;
    Ok(())
}

pub fn emit_security_rule_match_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let rule_event = security_rule_event(event_id, event_type, rule, event, timestamp_unix_ms)?;
    trace_security_rule_match(&rule_event, rule);
    emit_security_write_blocking(db, WriteOp::SecurityRuleEvent(rule_event));
    Ok(())
}

pub fn security_rule_event(
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityRuleEvent, String> {
    Ok(SecurityRuleEvent {
        timestamp_unix_ms,
        event_id: event_id.as_str().to_string(),
        event_type: event_type.as_str().to_string(),
        rule_id: rule.rule_id.clone(),
        rule_action: logged_rule_action(rule.action),
        detection_level: logged_detection_level(rule.detection_level),
        rule_json: serde_json::to_string(&compiled_rule_forensic_json(rule))
            .map_err(|error| format!("serialize security rule snapshot: {error}"))?,
        event_json: serde_json::to_string(&security_event_forensic_json(event))
            .map_err(|error| format!("serialize security event payload: {error}"))?,
        trace_id: event.trace_id(),
    })
}

pub async fn emit_security_ask_pending(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityEventId, String> {
    let ask_id = SecurityEventId::new_uuid4();
    let ask_event = security_ask_pending_event(
        ask_id.clone(),
        event_id,
        event_type,
        rule,
        event,
        timestamp_unix_ms,
    )?;
    emit_security_write(db, WriteOp::SecurityAskEvent(ask_event)).await;
    Ok(ask_id)
}

pub fn emit_security_ask_pending_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityEventId, String> {
    let ask_id = SecurityEventId::new_uuid4();
    let ask_event = security_ask_pending_event(
        ask_id.clone(),
        event_id,
        event_type,
        rule,
        event,
        timestamp_unix_ms,
    )?;
    emit_security_write_blocking(db, WriteOp::SecurityAskEvent(ask_event));
    Ok(ask_id)
}

pub fn emit_security_ask_resolution_blocking(
    db: &DbWriter,
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let event =
        security_ask_resolution_event(pending, status, resolver, reason, timestamp_unix_ms)?;
    emit_security_write_blocking(db, WriteOp::SecurityAskEvent(event));
    Ok(())
}

pub async fn emit_security_ask_resolution(
    db: &DbWriter,
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let event =
        security_ask_resolution_event(pending, status, resolver, reason, timestamp_unix_ms)?;
    emit_security_write(db, WriteOp::SecurityAskEvent(event)).await;
    Ok(())
}

fn security_ask_resolution_event(
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<SecurityAskEvent, String> {
    if matches!(status, SecurityAskStatus::Pending) {
        return Err("ask resolution status must be approved or denied".to_string());
    }
    let mut event = SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms,
        ask_id: pending.ask_id.clone(),
        event_id: pending.event_id.clone(),
        event_type: pending.event_type.clone(),
        rule_id: pending.rule_id.clone(),
        rule_name: pending.rule_name.clone(),
        rule_json: pending.rule_json.clone(),
        event_json: pending.event_json.clone(),
    })
    .with_status(status)
    .with_resolver(resolver);
    if let Some(reason) = reason {
        event = event.with_reason(reason);
    }
    if let Some(trace_id) = pending.trace_id.clone() {
        event = event.with_trace_id(trace_id);
    }
    Ok(event)
}

pub fn security_ask_pending_event(
    ask_id: SecurityEventId,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityAskEvent, String> {
    let mut ask = SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms,
        ask_id: ask_id.as_str().to_string(),
        event_id: event_id.as_str().to_string(),
        event_type: event_type.as_str().to_string(),
        rule_id: rule.rule_id.clone(),
        rule_name: rule.name.clone(),
        rule_json: serde_json::to_string(&compiled_rule_forensic_json(rule))
            .map_err(|error| format!("serialize security ask rule snapshot: {error}"))?,
        event_json: serde_json::to_string(&security_event_forensic_json(event))
            .map_err(|error| format!("serialize security ask event payload: {error}"))?,
    });
    if let Some(trace_id) = event.trace_id() {
        ask = ask.with_trace_id(trace_id);
    }
    Ok(ask)
}

fn logged_rule_action(action: SecurityRuleAction) -> LoggedRuleAction {
    match action {
        SecurityRuleAction::Allow => LoggedRuleAction::Allow,
        SecurityRuleAction::Ask => LoggedRuleAction::Ask,
        SecurityRuleAction::Block => LoggedRuleAction::Block,
        SecurityRuleAction::Preprocess => LoggedRuleAction::Preprocess,
        SecurityRuleAction::Rewrite => LoggedRuleAction::Rewrite,
        SecurityRuleAction::Postprocess => LoggedRuleAction::Postprocess,
    }
}

fn logged_detection_level(level: Option<DetectionLevel>) -> LoggedDetectionLevel {
    match level {
        Some(DetectionLevel::Informational) => LoggedDetectionLevel::Informational,
        Some(DetectionLevel::Low) => LoggedDetectionLevel::Low,
        Some(DetectionLevel::Medium) => LoggedDetectionLevel::Medium,
        Some(DetectionLevel::High) => LoggedDetectionLevel::High,
        Some(DetectionLevel::Critical) => LoggedDetectionLevel::Critical,
        None => LoggedDetectionLevel::None,
    }
}

fn compiled_rule_forensic_json(rule: &CompiledSecurityRule) -> serde_json::Value {
    json!({
        "rule_id": rule.rule_id,
        "provider": rule.provider,
        "namespace": rule.namespace,
        "rule_key": rule.rule_key,
        "name": rule.name,
        "rule_action": rule.action.as_str(),
        "match": rule.condition,
        "detection_level": rule
            .detection_level
            .map(|level| level.as_str())
            .unwrap_or("none"),
        "priority": rule.priority,
        "corp_locked": rule.corp_locked,
        "reason": rule.reason,
    })
}

fn security_event_forensic_json(event: &SecurityEvent) -> serde_json::Value {
    json!({
        "event_type": event.event_type.as_str(),
        "credential_ref": event.credential_ref,
        "credential_observations": event.credential_observations.iter().map(|observation| {
            json!({
                "provider": observation.provider.as_str(),
                "source": observation.source,
                "event_type": observation.event_type,
                "confidence": observation.confidence,
                "trace_id": observation.trace_id,
                "context_json": observation.context_json,
                "credential_ref": observation.credential_ref(),
            })
        }).collect::<Vec<_>>(),
        "action_trace": event.action_trace.iter().map(|action| action.as_str()).collect::<Vec<_>>(),
        "decision": event.decision,
        "detections": event.detections,
        "http_request": event.http_request.as_ref().map(http_request_forensic_json),
        "http": event.http,
        "dns": event.dns,
        "mcp": event.mcp,
        "model": event.model,
        "file": event.file,
        "process": event.process,
    })
}

fn http_request_forensic_json(request: &HttpRequestSecurityEvent) -> serde_json::Value {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<non-utf8>").to_string(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    json!({
        "domain": request.domain,
        "ai_provider": request.ai_provider.map(|provider| provider.as_str()),
        "headers": headers,
        "query": request.query,
    })
}

fn trace_runtime_security_event(event: &RuntimeSecurityEvent) {
    tracing::debug!(
        event_type = event.event_type.as_str(),
        event_family = event.event_family.as_str(),
        event_id = event.event_id.as_ref().map(|id| id.as_str()),
        credential_ref = event.credential_ref.as_deref(),
        trace_id = event.trace_id.as_deref(),
        "runtime security event emitted"
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityRuleTraceLabels {
    pub rule_id: String,
    pub rule_name: String,
    pub rule_action: &'static str,
    pub rule_detection_level: &'static str,
    pub provider: String,
}

impl SecurityRuleTraceLabels {
    pub fn from_rule(rule: &CompiledSecurityRule) -> Self {
        Self {
            rule_id: rule.rule_id.clone(),
            rule_name: rule.name.clone(),
            rule_action: rule.action.as_str(),
            rule_detection_level: rule
                .detection_level
                .map(|level| level.as_str())
                .unwrap_or("none"),
            provider: rule.provider.clone(),
        }
    }
}

fn trace_security_rule_match(event: &SecurityRuleEvent, rule: &CompiledSecurityRule) {
    let labels = SecurityRuleTraceLabels::from_rule(rule);
    tracing::debug!(
        event_id = event.event_id.as_str(),
        event_type = event.event_type.as_str(),
        trace_id = event.trace_id.as_deref(),
        rule_id = labels.rule_id.as_str(),
        rule_name = labels.rule_name.as_str(),
        rule_action = labels.rule_action,
        rule_detection_level = labels.rule_detection_level,
        provider = labels.provider.as_str(),
        "security rule matched"
    );
}

fn logger_write_credential_ref(op: &WriteOp) -> Option<String> {
    match op {
        WriteOp::NetEvent(event) => event.credential_ref.clone(),
        WriteOp::ModelCall(event) => event.credential_ref.clone(),
        WriteOp::McpCall(event) => event.credential_ref.clone(),
        WriteOp::FileEvent(event) => event.credential_ref.clone(),
        WriteOp::ExecEvent(event) => event.credential_ref.clone(),
        WriteOp::ExecEventComplete(_) => None,
        WriteOp::AuditEvent(event) => event.credential_ref.clone(),
        WriteOp::DnsEvent(event) => event.credential_ref.clone(),
        WriteOp::SubstitutionEvent(event) => Some(event.substitution_ref.clone()),
        WriteOp::SecurityRuleEvent(_) => None,
        WriteOp::SecurityAskEvent(_) => None,
        WriteOp::SecurityDecisionEvent(_) => None,
        WriteOp::ProfileMutationEvent(_) => None,
    }
}

fn logger_write_trace_id(op: &WriteOp) -> Option<String> {
    match op {
        WriteOp::NetEvent(event) => event.trace_id.clone(),
        WriteOp::ModelCall(event) => event.trace_id.clone(),
        WriteOp::McpCall(event) => event.trace_id.clone(),
        WriteOp::FileEvent(event) => event.trace_id.clone(),
        WriteOp::ExecEvent(event) => event.trace_id.clone(),
        WriteOp::ExecEventComplete(_) => None,
        WriteOp::AuditEvent(event) => event.trace_id.clone(),
        WriteOp::DnsEvent(event) => event.trace_id.clone(),
        WriteOp::SubstitutionEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityRuleEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityAskEvent(event) => event.trace_id.clone(),
        WriteOp::SecurityDecisionEvent(event) => event.trace_id.clone(),
        WriteOp::ProfileMutationEvent(event) => event.trace_id.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityDecisionKind {
    Allow,
    Ask,
    Block,
}

impl SecurityDecisionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Ask => 1,
            Self::Block => 2,
        }
    }

    pub const fn merge(self, requested: Self) -> Self {
        if self.rank() >= requested.rank() {
            self
        } else {
            requested
        }
    }
}

impl From<SecurityDecisionKind> for LoggedSecurityDecision {
    fn from(value: SecurityDecisionKind) -> Self {
        match value {
            SecurityDecisionKind::Allow => LoggedSecurityDecision::Allow,
            SecurityDecisionKind::Ask => LoggedSecurityDecision::Ask,
            SecurityDecisionKind::Block => LoggedSecurityDecision::Block,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityDecisionState {
    pub effective: SecurityDecisionKind,
}

impl Default for SecurityDecisionState {
    fn default() -> Self {
        Self {
            effective: SecurityDecisionKind::Allow,
        }
    }
}

impl SecurityDecisionState {
    pub fn request(
        &mut self,
        requested: SecurityDecisionKind,
    ) -> (SecurityDecisionKind, SecurityDecisionKind) {
        let previous = self.effective;
        self.effective = self.effective.merge(requested);
        (previous, self.effective)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityDetectionEvent {
    pub source: SecurityDetectionSource,
    pub detection_level: DetectionLevel,
    pub rule_id: Option<String>,
    pub plugin_id: Option<String>,
    pub action: Option<SecurityRuleAction>,
    pub plugin_mode: Option<SecurityPluginMode>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityDetectionSource {
    Rule,
    Plugin,
}

/// Canonical security-event envelope used by rule actions and emitters.
///
/// Protocol parsers attach typed context to this object; action plugins return
/// the next object. Persistence, fanout, batching, and future process
/// transport should hang off `SecurityEventEmitter`, not protocol side writes.
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityEvent {
    pub event_type: RuntimeSecurityEventType,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
    pub credential_observations: Vec<CredentialObservation>,
    pub action_trace: Vec<PolicyActionId>,
    pub decision: SecurityDecisionState,
    pub detections: Vec<SecurityDetectionEvent>,
    pub http_request: Option<HttpRequestSecurityEvent>,
    pub http: Option<HttpSecurityEvent>,
    pub dns: Option<DnsSecurityEvent>,
    pub mcp: Option<McpSecurityEvent>,
    pub model: Option<ModelSecurityEvent>,
    pub file: Option<FileSecurityEvent>,
    pub process: Option<ProcessSecurityEvent>,
    pub ip: Option<IpSecurityEvent>,
    pub tcp: Option<TcpSecurityEvent>,
    pub udp: Option<UdpSecurityEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SerializableSecurityEvent {
    pub event_type: String,
    pub trace_id: Option<String>,
    pub credential_ref: Option<String>,
    pub action_trace: Vec<String>,
    pub decision: SecurityDecisionState,
    pub detections: Vec<SecurityDetectionEvent>,
    pub http: Option<HttpSecurityEvent>,
    pub dns: Option<DnsSecurityEvent>,
    pub mcp: Option<McpSecurityEvent>,
    pub model: Option<ModelSecurityEvent>,
    pub file: Option<FileSecurityEvent>,
    pub process: Option<ProcessSecurityEvent>,
    pub ip: Option<IpSecurityEvent>,
    pub tcp: Option<TcpSecurityEvent>,
    pub udp: Option<UdpSecurityEvent>,
}

impl From<&SecurityEvent> for SerializableSecurityEvent {
    fn from(event: &SecurityEvent) -> Self {
        Self {
            event_type: event.event_type.as_str().to_string(),
            trace_id: event.trace_id.clone(),
            credential_ref: event.credential_ref.clone(),
            action_trace: event
                .action_trace
                .iter()
                .map(|action| action.as_str().to_string())
                .collect(),
            decision: event.decision.clone(),
            detections: event.detections.clone(),
            http: event.http.clone(),
            dns: event.dns.clone(),
            mcp: event.mcp.clone(),
            model: event.model.clone(),
            file: event.file.clone(),
            process: event.process.clone(),
            ip: event.ip.clone(),
            tcp: event.tcp.clone(),
            udp: event.udp.clone(),
        }
    }
}

impl SecurityEvent {
    pub fn new(event_type: RuntimeSecurityEventType) -> Self {
        Self {
            event_type,
            trace_id: None,
            credential_ref: None,
            credential_observations: Vec::new(),
            action_trace: Vec::new(),
            decision: SecurityDecisionState::default(),
            detections: Vec::new(),
            http_request: None,
            http: None,
            dns: None,
            mcp: None,
            model: None,
            file: None,
            process: None,
            ip: None,
            tcp: None,
            udp: None,
        }
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    pub fn with_credential_ref(mut self, credential_ref: impl Into<String>) -> Self {
        self.credential_ref = Some(credential_ref.into());
        self
    }

    pub fn with_http_request(mut self, request: HttpRequestSecurityEvent) -> Self {
        self.http_request = Some(request);
        self
    }

    pub fn with_credential_observations(
        mut self,
        observations: Vec<CredentialObservation>,
    ) -> Self {
        self.credential_observations = observations;
        self
    }

    pub fn with_http(mut self, http: HttpSecurityEvent) -> Self {
        self.http = Some(http);
        self
    }

    pub fn with_dns(mut self, dns: DnsSecurityEvent) -> Self {
        self.dns = Some(dns);
        self
    }

    pub fn with_mcp(mut self, mcp: McpSecurityEvent) -> Self {
        self.mcp = Some(mcp);
        self
    }

    pub fn with_model(mut self, model: ModelSecurityEvent) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_file(mut self, file: FileSecurityEvent) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_process(mut self, process: ProcessSecurityEvent) -> Self {
        self.process = Some(process);
        self
    }

    pub fn with_ip(mut self, ip: IpSecurityEvent) -> Self {
        self.ip = Some(ip);
        self
    }

    pub fn with_tcp(mut self, tcp: TcpSecurityEvent) -> Self {
        self.tcp = Some(tcp);
        self
    }

    pub fn with_udp(mut self, udp: UdpSecurityEvent) -> Self {
        self.udp = Some(udp);
        self
    }

    pub fn trace_id(&self) -> Option<String> {
        self.trace_id.clone().or_else(|| {
            self.credential_observations
                .iter()
                .find_map(|observation| observation.trace_id.clone())
        })
    }

    pub fn request_decision(
        &mut self,
        requested: SecurityDecisionKind,
    ) -> (SecurityDecisionKind, SecurityDecisionKind) {
        self.decision.request(requested)
    }

    pub fn record_detection(&mut self, detection: SecurityDetectionEvent) {
        self.detections.push(detection);
    }

    pub fn serializable(&self) -> SerializableSecurityEvent {
        SerializableSecurityEvent::from(self)
    }
}

impl PolicySubject for SecurityEvent {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        if let Some(rest) = field.strip_prefix("http.") {
            return self.http.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("dns.") {
            return self.dns.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("mcp.") {
            return self.mcp.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("model.") {
            return self.model.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("file.") {
            return self.file.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("process.") {
            return self.process.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("ip.") {
            return self.ip.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("tcp.") {
            return self.tcp.as_ref().and_then(|event| event.get(rest));
        }
        if let Some(rest) = field.strip_prefix("udp.") {
            return self.udp.as_ref().and_then(|event| event.get(rest));
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct HttpSecurityEvent {
    pub host: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<String>,
    pub body: Option<String>,
}

impl HttpSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "host" => borrowed_string(self.host.as_deref()),
            "method" => borrowed_string(self.method.as_deref()),
            "path" => borrowed_string(self.path.as_deref()),
            "status" => borrowed_string(self.status.as_deref()),
            "body" => borrowed_string(self.body.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct DnsSecurityEvent {
    pub qname: Option<String>,
    pub qtype: Option<String>,
}

impl DnsSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "qname" => borrowed_string(self.qname.as_deref()),
            "qtype" => borrowed_string(self.qtype.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct McpSecurityEvent {
    pub method: Option<String>,
    pub server_name: Option<String>,
    pub tool_call_name: Option<String>,
    pub tool_list: Option<String>,
}

impl McpSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "method" => borrowed_string(self.method.as_deref()),
            "server.name" => borrowed_string(self.server_name.as_deref()),
            "server.valid" => Some(PolicySubjectValue::Bool(self.server_name.is_some())),
            "tool_call.valid" => Some(PolicySubjectValue::Bool(self.tool_call_name.is_some())),
            "tool_call.name" => borrowed_string(self.tool_call_name.as_deref()),
            "tool_list.valid" => Some(PolicySubjectValue::Bool(self.tool_list.is_some())),
            "tool_list" => borrowed_string(self.tool_list.as_deref()),
            "event.valid" => Some(PolicySubjectValue::Bool(self.method.is_some())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ModelSecurityEvent {
    pub provider: Option<String>,
    pub name: Option<String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub tool_calls: Option<String>,
}

impl ModelSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "provider" => borrowed_string(self.provider.as_deref()),
            "name" => borrowed_string(self.name.as_deref()),
            "request.valid" => Some(PolicySubjectValue::Bool(
                self.request_body.is_some() || self.tool_calls.is_some(),
            )),
            "request.body" => borrowed_string(self.request_body.as_deref()),
            "response.valid" => Some(PolicySubjectValue::Bool(self.response_body.is_some())),
            "response.body" => borrowed_string(self.response_body.as_deref()),
            "tool_call.valid" => Some(PolicySubjectValue::Bool(self.tool_calls.is_some())),
            "request.tool_calls" => borrowed_string(self.tool_calls.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct FileSecurityEvent {
    pub import_path: Option<String>,
    pub import_name: Option<String>,
    pub import_ext: Option<String>,
    pub import_mime_type: Option<String>,
    pub import_content: Option<String>,
    pub export_path: Option<String>,
    pub export_name: Option<String>,
    pub export_ext: Option<String>,
    pub export_mime_type: Option<String>,
    pub export_content: Option<String>,
    pub read_path: Option<String>,
    pub read_name: Option<String>,
    pub read_ext: Option<String>,
    pub read_mime_type: Option<String>,
    pub read_content: Option<String>,
    pub create_path: Option<String>,
    pub create_name: Option<String>,
    pub create_ext: Option<String>,
    pub create_mime_type: Option<String>,
    pub create_content: Option<String>,
    pub write_path: Option<String>,
    pub write_name: Option<String>,
    pub write_ext: Option<String>,
    pub write_mime_type: Option<String>,
    pub write_content: Option<String>,
    pub delete_path: Option<String>,
    pub delete_name: Option<String>,
    pub delete_ext: Option<String>,
    pub delete_mime_type: Option<String>,
    pub delete_content: Option<String>,
    pub content: Option<String>,
}

impl FileSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "import.valid" => Some(PolicySubjectValue::Bool(self.import_path.is_some())),
            "import.path" => borrowed_string(self.import_path.as_deref()),
            "import.name" => borrowed_string(self.import_name.as_deref()),
            "import.ext" => borrowed_string(self.import_ext.as_deref()),
            "import.mime_type" => borrowed_string(self.import_mime_type.as_deref()),
            "import.content" => borrowed_string(self.import_content.as_deref()),
            "export.valid" => Some(PolicySubjectValue::Bool(self.export_path.is_some())),
            "export.path" => borrowed_string(self.export_path.as_deref()),
            "export.name" => borrowed_string(self.export_name.as_deref()),
            "export.ext" => borrowed_string(self.export_ext.as_deref()),
            "export.mime_type" => borrowed_string(self.export_mime_type.as_deref()),
            "export.content" => borrowed_string(self.export_content.as_deref()),
            "read.valid" => Some(PolicySubjectValue::Bool(self.read_path.is_some())),
            "read.path" => borrowed_string(self.read_path.as_deref()),
            "read.name" => borrowed_string(self.read_name.as_deref()),
            "read.ext" => borrowed_string(self.read_ext.as_deref()),
            "read.mime_type" => borrowed_string(self.read_mime_type.as_deref()),
            "read.content" => borrowed_string(self.read_content.as_deref()),
            "create.valid" => Some(PolicySubjectValue::Bool(self.create_path.is_some())),
            "create.path" => borrowed_string(self.create_path.as_deref()),
            "create.name" => borrowed_string(self.create_name.as_deref()),
            "create.ext" => borrowed_string(self.create_ext.as_deref()),
            "create.mime_type" => borrowed_string(self.create_mime_type.as_deref()),
            "create.content" => borrowed_string(self.create_content.as_deref()),
            "write.valid" => Some(PolicySubjectValue::Bool(self.write_path.is_some())),
            "write.path" => borrowed_string(self.write_path.as_deref()),
            "write.name" => borrowed_string(self.write_name.as_deref()),
            "write.ext" => borrowed_string(self.write_ext.as_deref()),
            "write.mime_type" => borrowed_string(self.write_mime_type.as_deref()),
            "write.content" => borrowed_string(self.write_content.as_deref()),
            "delete.valid" => Some(PolicySubjectValue::Bool(self.delete_path.is_some())),
            "delete.path" => borrowed_string(self.delete_path.as_deref()),
            "delete.name" => borrowed_string(self.delete_name.as_deref()),
            "delete.ext" => borrowed_string(self.delete_ext.as_deref()),
            "delete.mime_type" => borrowed_string(self.delete_mime_type.as_deref()),
            "delete.content" => borrowed_string(self.delete_content.as_deref()),
            "content" => borrowed_string(self.content.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ProcessSecurityEvent {
    pub exec_id: Option<String>,
    pub exec_path: Option<String>,
    pub command: Option<String>,
    pub exit_code: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

impl ProcessSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "exec.valid" => Some(PolicySubjectValue::Bool(
                self.exec_id.is_some()
                    || self.exec_path.is_some()
                    || self.command.is_some()
                    || self.exit_code.is_some(),
            )),
            "exec.id" => borrowed_string(self.exec_id.as_deref()),
            "exec.path" => borrowed_string(self.exec_path.as_deref()),
            "exec.exit_code" => borrowed_string(self.exit_code.as_deref()),
            "exec.stdout" => borrowed_string(self.stdout.as_deref()),
            "exec.stderr" => borrowed_string(self.stderr.as_deref()),
            "audit.valid" => Some(PolicySubjectValue::Bool(self.command.is_some())),
            "command" => borrowed_string(self.command.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct IpSecurityEvent {
    pub value: Option<String>,
    pub version: Option<String>,
}

impl IpSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "value" => borrowed_string(self.value.as_deref()),
            "version" => borrowed_string(self.version.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct TcpSecurityEvent {
    pub port: Option<String>,
}

impl TcpSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "port" => borrowed_string(self.port.as_deref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct UdpSecurityEvent {
    pub port: Option<String>,
}

impl UdpSecurityEvent {
    fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "port" => borrowed_string(self.port.as_deref()),
            _ => None,
        }
    }
}

fn borrowed_string(value: Option<&str>) -> Option<PolicySubjectValue<'_>> {
    value.map(|value| PolicySubjectValue::String(Cow::Borrowed(value)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestSecurityEvent {
    pub domain: String,
    pub ai_provider: Option<ProviderKind>,
    pub headers: http::HeaderMap,
    pub query: Option<String>,
}

impl HttpRequestSecurityEvent {
    pub fn new(
        domain: impl Into<String>,
        ai_provider: Option<ProviderKind>,
        headers: http::HeaderMap,
        query: Option<String>,
    ) -> Self {
        Self {
            domain: domain.into(),
            ai_provider,
            headers,
            query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedHttpRequest {
    pub headers: http::HeaderMap,
    pub query: Option<String>,
    pub credential_ref: Option<String>,
}

pub fn materialize_http_request_for_upstream(
    event: &SecurityEvent,
) -> Result<MaterializedHttpRequest, SecurityActionError> {
    let Some(request) = event.http_request.as_ref() else {
        return Err(SecurityActionError::new(
            "security event does not carry an HTTP request",
        ));
    };

    if !event
        .action_trace
        .contains(&PolicyActionId::CredentialBrokerSubstitute)
    {
        return Ok(MaterializedHttpRequest {
            headers: request.headers.clone(),
            query: request.query.clone(),
            credential_ref: event.credential_ref.clone(),
        });
    }

    let mut headers = request.headers.clone();
    let BrokeredUpstreamCredentials {
        credential_ref,
        query,
    } = crate::credential_broker::substitute_brokered_upstream_credentials(
        &request.domain,
        request.ai_provider,
        &mut headers,
        request.query.as_deref(),
    )
    .map_err(SecurityActionError::new)?;

    Ok(MaterializedHttpRequest {
        headers,
        query,
        credential_ref: event.credential_ref.clone().or(credential_ref),
    })
}

pub fn materialize_http_request_for_upstream_after_enforcement(
    event: &SecurityEvent,
    decision: &SecurityEnforcementDecision,
) -> Result<MaterializedHttpRequest, SecurityActionError> {
    if !decision.is_allowed() {
        return Err(SecurityActionError::new(format!(
            "security rule '{}' requires '{}' before HTTP materialization",
            decision.rule_id.as_deref().unwrap_or("unknown"),
            decision.action.as_str()
        )));
    }
    materialize_http_request_for_upstream(event)
}

impl SecurityEnforcementAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityActionError {
    message: String,
}

impl SecurityActionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SecurityActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SecurityActionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityPluginStage {
    PreDecision,
    PostDecision,
}

pub struct SecurityPluginResult {
    pub event: SecurityEvent,
    pub applied: bool,
}

impl SecurityPluginResult {
    pub const fn applied(event: SecurityEvent) -> Self {
        Self {
            event,
            applied: true,
        }
    }

    pub const fn skipped(event: SecurityEvent) -> Self {
        Self {
            event,
            applied: false,
        }
    }
}

/// A plugin that mutates or annotates the canonical security event on the same
/// rail as CEL enforcement.
pub trait SecurityPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn stage(&self) -> SecurityPluginStage;

    fn apply(&self, event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError>;
}

#[derive(Default)]
pub struct SecurityActionRegistry {
    plugins: BTreeMap<String, Arc<dyn SecurityPlugin>>,
    plugin_policy: BTreeMap<String, SecurityPluginConfig>,
}

impl SecurityActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtin_actions() -> Self {
        Self::new()
            .register_plugin(CredentialBrokerPlugin)
            .expect("built-in security plugin ids are unique")
            .register_plugin(DummyPreEicarPlugin)
            .expect("built-in security plugin ids are unique")
            .register_plugin(DummyPostAllowPlugin)
            .expect("built-in security plugin ids are unique")
    }

    pub fn with_plugin_policy(
        mut self,
        plugin_policy: BTreeMap<String, SecurityPluginConfig>,
    ) -> Self {
        self.plugin_policy = plugin_policy;
        self
    }

    pub fn register_plugin(
        mut self,
        plugin: impl SecurityPlugin + 'static,
    ) -> Result<Self, SecurityActionError> {
        let id = plugin.id();
        if self.plugins.contains_key(id) {
            return Err(SecurityActionError::new(format!(
                "security plugin '{id}' registered twice"
            )));
        }
        self.plugins.insert(id.to_string(), Arc::new(plugin));
        Ok(self)
    }

    pub fn apply_security_plugins(
        &self,
        stage: SecurityPluginStage,
        mut event: SecurityEvent,
    ) -> Result<SecurityEvent, SecurityActionError> {
        for (plugin_id, config) in &self.plugin_policy {
            if config.mode != SecurityPluginMode::Disable && !self.plugins.contains_key(plugin_id) {
                return Err(SecurityActionError::new(format!(
                    "security plugin '{plugin_id}' is not registered"
                )));
            }
        }
        for (plugin_id, plugin) in &self.plugins {
            if plugin.stage() != stage {
                continue;
            }
            let Some(plugin_config) = self.plugin_policy.get(plugin_id).copied() else {
                continue;
            };
            if plugin_config.mode == SecurityPluginMode::Disable {
                continue;
            }
            let result = plugin.apply(event)?;
            event = result.event;
            if !result.applied {
                continue;
            }
            record_plugin_detection(&mut event, plugin_id, plugin_config);
            if let Some(requested) = plugin_mode_decision(plugin_config.mode) {
                event.request_decision(requested);
            }
        }
        Ok(event)
    }
}

fn record_plugin_detection(
    event: &mut SecurityEvent,
    plugin_id: &str,
    config: SecurityPluginConfig,
) {
    let Some(detection_level) = config.active_detection_level() else {
        return;
    };
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Plugin,
        detection_level,
        rule_id: None,
        plugin_id: Some(plugin_id.to_string()),
        action: None,
        plugin_mode: Some(config.mode),
        reason: None,
    });
}

fn plugin_mode_decision(mode: SecurityPluginMode) -> Option<SecurityDecisionKind> {
    match mode {
        SecurityPluginMode::Disable => None,
        SecurityPluginMode::Allow | SecurityPluginMode::Rewrite => {
            Some(SecurityDecisionKind::Allow)
        }
        SecurityPluginMode::Ask => Some(SecurityDecisionKind::Ask),
        SecurityPluginMode::Block => Some(SecurityDecisionKind::Block),
    }
}

pub struct CredentialBrokerPlugin;

impl SecurityPlugin for CredentialBrokerPlugin {
    fn id(&self) -> &'static str {
        "credential_broker"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::PostDecision
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        if event.credential_observations.is_empty() {
            return Ok(SecurityPluginResult::skipped(event));
        }
        for observation in &event.credential_observations {
            let brokered = crate::credential_broker::broker_observed_credential(observation)
                .map_err(SecurityActionError::new)?;
            if event.credential_ref.is_none() {
                event.credential_ref = Some(brokered.credential_ref);
            }
        }
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}

pub struct DummyPreEicarPlugin;

impl SecurityPlugin for DummyPreEicarPlugin {
    fn id(&self) -> &'static str {
        "dummy_pre_eicar"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::PreDecision
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        if !security_event_contains_text(&event, DUMMY_EICAR_TEST_STRING)
            && !security_event_contains_text(&event, "EICAR")
        {
            return Ok(SecurityPluginResult::skipped(event));
        }
        event.request_decision(SecurityDecisionKind::Block);
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}

pub struct DummyPostAllowPlugin;

impl SecurityPlugin for DummyPostAllowPlugin {
    fn id(&self) -> &'static str {
        "dummy_post_allow"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::PostDecision
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(SecurityDecisionKind::Allow);
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerSubstitute);
        Ok(SecurityPluginResult::applied(event))
    }
}

fn security_event_contains_text(event: &SecurityEvent, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    event
        .file
        .as_ref()
        .is_some_and(|file| file_contains_text(file, needle))
        || event
            .http
            .as_ref()
            .and_then(|http| http.body.as_deref())
            .is_some_and(|body| body.contains(needle))
        || event
            .model
            .as_ref()
            .is_some_and(|model| model_contains_text(model, needle))
}

fn file_contains_text(file: &FileSecurityEvent, needle: &str) -> bool {
    [
        file.import_content.as_deref(),
        file.export_content.as_deref(),
        file.read_content.as_deref(),
        file.create_content.as_deref(),
        file.write_content.as_deref(),
        file.delete_content.as_deref(),
        file.content.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|content| content.contains(needle))
}

fn model_contains_text(model: &ModelSecurityEvent, needle: &str) -> bool {
    [
        model.name.as_deref(),
        model.request_body.as_deref(),
        model.response_body.as_deref(),
        model.tool_calls.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|content| content.contains(needle))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityEmitError {
    message: String,
}

impl SecurityEmitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SecurityEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SecurityEmitError {}

/// Single auditable event emission boundary.
pub trait SecurityEventEmitter: Send + Sync {
    fn emit(&self, event: SecurityEvent) -> Result<(), SecurityEmitError>;
}

/// Security-event execution boundary for matched rule actions.
///
/// Runtime/parser paths hand this engine a canonical `SecurityEvent` plus the
/// matched action-bearing rules. The engine applies actions in deterministic
/// order, then emits exactly the final post-action event.
pub struct SecurityEventEngine<E: SecurityEventEmitter> {
    action_registry: SecurityActionRegistry,
    emitter: Arc<E>,
}

impl<E: SecurityEventEmitter> SecurityEventEngine<E> {
    pub fn new(action_registry: SecurityActionRegistry, emitter: Arc<E>) -> Self {
        Self {
            action_registry,
            emitter,
        }
    }

    pub fn with_builtin_actions(emitter: Arc<E>) -> Self {
        Self::new(SecurityActionRegistry::with_builtin_actions(), emitter)
    }

    pub fn apply_matching_rules_and_emit(
        &self,
        rules: &SecurityRuleSet,
        mut event: SecurityEvent,
    ) -> Result<SecurityEvent, SecurityActionError> {
        event = self
            .action_registry
            .apply_security_plugins(SecurityPluginStage::PreDecision, event)?;

        let evaluation = rules.evaluate(&event).map_err(SecurityActionError::new)?;
        for rule in evaluation.matched_rules() {
            record_rule_detection(&mut event, rule);
            event.request_decision(requested_decision_for_rule(rule.action));
        }
        event = self
            .action_registry
            .apply_security_plugins(SecurityPluginStage::PostDecision, event)?;
        self.emitter
            .emit(event.clone())
            .map_err(|error| SecurityActionError::new(error.to_string()))?;
        Ok(event)
    }
}

#[derive(Debug, Default)]
pub struct TracingSecurityEventEmitter;

impl SecurityEventEmitter for TracingSecurityEventEmitter {
    fn emit(&self, event: SecurityEvent) -> Result<(), SecurityEmitError> {
        tracing::debug!(
            event_type = event.event_type.as_str(),
            credential_ref = event.credential_ref.as_deref(),
            action_count = event.action_trace.len(),
            "security event emitted"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
