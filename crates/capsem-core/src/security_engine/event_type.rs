//! Canonical runtime event vocabulary and primary ledger classification.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeSecurityEventFamily {
    Network,
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
            RuntimeSecurityEventFamily::Network => "network",
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
            RuntimeSecurityEventFamily::Network
                | RuntimeSecurityEventFamily::Http
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
    NetworkConnect,
    NetworkConnectResult,
    NetworkClose,
    NetworkLifecycle,
    NetworkProbe,
    NetworkProbeResult,
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
        Self::NetworkConnect,
        Self::NetworkConnectResult,
        Self::NetworkClose,
        Self::NetworkLifecycle,
        Self::NetworkProbe,
        Self::NetworkProbeResult,
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
            Self::NetworkConnect => "network.connect",
            Self::NetworkConnectResult => "network.connect_result",
            Self::NetworkClose => "network.close",
            Self::NetworkLifecycle => "network.lifecycle",
            Self::NetworkProbe => "network.probe",
            Self::NetworkProbeResult => "network.probe_result",
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
            Self::NetworkConnect
            | Self::NetworkConnectResult
            | Self::NetworkClose
            | Self::NetworkLifecycle
            | Self::NetworkProbe
            | Self::NetworkProbeResult => RuntimeSecurityEventFamily::Network,
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
            RuntimeSecurityEventType::CredentialSubstitution => RuntimeSecurityEventFamily::Credential,
            RuntimeSecurityEventType::SecurityRule => RuntimeSecurityEventFamily::Security,
            RuntimeSecurityEventType::SecurityAsk => RuntimeSecurityEventFamily::Security,
        }
    }

    pub const fn uses_ledger_only_family(self) -> bool {
        self.family().is_ledger_only()
    }

    pub fn parse_str(value: &str) -> Result<Self, SecurityEventTypeParseError> {
        match value {
            "network.connect" => Ok(Self::NetworkConnect),
            "network.connect_result" => Ok(Self::NetworkConnectResult),
            "network.close" => Ok(Self::NetworkClose),
            "network.lifecycle" => Ok(Self::NetworkLifecycle),
            "network.probe" => Ok(Self::NetworkProbe),
            "network.probe_result" => Ok(Self::NetworkProbeResult),
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

    pub(super) fn for_write_op(op: &WriteOp) -> Self {
        match op {
            WriteOp::TransportEvent(event) => event.kind().into(),
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
