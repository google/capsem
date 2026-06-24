use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub const CREDENTIAL_REF_PREFIX: &str = "credential:blake3:";
const CREDENTIAL_REF_DOMAIN: &[u8] = b"capsem.credential.v1";

/// Build the canonical brokered credential reference used downstream by
/// security events, logs, CEL, and session.db.
pub fn credential_reference(provider: &str, raw_credential: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CREDENTIAL_REF_DOMAIN);
    hasher.update(&[0]);
    hasher.update(provider.as_bytes());
    hasher.update(&[0]);
    hasher.update(raw_credential.as_bytes());
    format!("{CREDENTIAL_REF_PREFIX}{}", hasher.finalize().to_hex())
}

pub fn is_credential_reference(value: &str) -> bool {
    value
        .strip_prefix(CREDENTIAL_REF_PREFIX)
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Canonical action vocabulary for security rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityRuleAction {
    Allow,
    Ask,
    Block,
    Preprocess,
    Rewrite,
    Postprocess,
}

impl SecurityRuleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityRuleAction::Allow => "allow",
            SecurityRuleAction::Ask => "ask",
            SecurityRuleAction::Block => "block",
            SecurityRuleAction::Preprocess => "preprocess",
            SecurityRuleAction::Rewrite => "rewrite",
            SecurityRuleAction::Postprocess => "postprocess",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(SecurityRuleAction::Allow),
            "ask" => Some(SecurityRuleAction::Ask),
            "block" => Some(SecurityRuleAction::Block),
            "preprocess" => Some(SecurityRuleAction::Preprocess),
            "rewrite" => Some(SecurityRuleAction::Rewrite),
            "postprocess" => Some(SecurityRuleAction::Postprocess),
            _ => None,
        }
    }
}

/// Sigma-aligned detection level metadata attached to a rule match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityDetectionLevel {
    None,
    Informational,
    Low,
    Medium,
    High,
    Critical,
}

impl SecurityDetectionLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityDetectionLevel::None => "none",
            SecurityDetectionLevel::Informational => "informational",
            SecurityDetectionLevel::Low => "low",
            SecurityDetectionLevel::Medium => "medium",
            SecurityDetectionLevel::High => "high",
            SecurityDetectionLevel::Critical => "critical",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "none" => Some(SecurityDetectionLevel::None),
            "informational" => Some(SecurityDetectionLevel::Informational),
            "low" => Some(SecurityDetectionLevel::Low),
            "medium" => Some(SecurityDetectionLevel::Medium),
            "high" => Some(SecurityDetectionLevel::High),
            "critical" => Some(SecurityDetectionLevel::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityDecision {
    Allow,
    Ask,
    Block,
}

impl SecurityDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityDecision::Allow => "allow",
            SecurityDecision::Ask => "ask",
            SecurityDecision::Block => "block",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(SecurityDecision::Allow),
            "ask" => Some(SecurityDecision::Ask),
            "block" => Some(SecurityDecision::Block),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityDecisionStage {
    Preprocess,
    Rule,
    Rewrite,
    Postprocess,
    AskResolution,
}

impl SecurityDecisionStage {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityDecisionStage::Preprocess => "preprocess",
            SecurityDecisionStage::Rule => "rule",
            SecurityDecisionStage::Rewrite => "rewrite",
            SecurityDecisionStage::Postprocess => "postprocess",
            SecurityDecisionStage::AskResolution => "ask_resolution",
        }
    }
}

/// Append-only decision transition row. This is the durable truth for what a
/// stage wanted and what the effective decision became.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityDecisionEvent {
    pub timestamp_unix_ms: i64,
    pub event_id: String,
    pub event_type: String,
    pub stage: SecurityDecisionStage,
    pub actor: String,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub plugin_id: Option<String>,
    pub previous_decision: SecurityDecision,
    pub requested_decision: SecurityDecision,
    pub effective_decision: SecurityDecision,
    #[serde(default)]
    pub reason: Option<String>,
    pub event_json: String,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A stored security rule match. This is the source for runtime `latest`
/// projections; every field here is intentionally DB-backed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityRuleEvent {
    pub timestamp_unix_ms: i64,
    pub event_id: String,
    pub event_type: String,
    pub rule_id: String,
    pub rule_action: SecurityRuleAction,
    pub detection_level: SecurityDetectionLevel,
    /// Canonical serialized rule snapshot at match time. This must be enough
    /// for later forensic review even if the active ruleset has changed.
    pub rule_json: String,
    /// Canonical serialized normalized SecurityEvent payload that the rule
    /// matched. Raw secrets must already be brokered before this row.
    pub event_json: String,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMutationEvent {
    pub timestamp_unix_ms: i64,
    pub mutation_id: String,
    pub profile_id: String,
    pub actor: String,
    pub category: String,
    pub filename: String,
    pub affected_path: String,
    pub target_kind: String,
    pub target_key: String,
    pub operation: String,
    #[serde(default)]
    pub rule_id: Option<String>,
    pub old_hash: String,
    pub old_size: u64,
    pub new_hash: String,
    pub new_size: u64,
    pub status: ProfileMutationStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileMutationStatus {
    Applied,
    Failed,
}

impl ProfileMutationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Failed => "failed",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "applied" => Some(Self::Applied),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Append-only ask lifecycle status for an ask enforcement decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityAskStatus {
    Pending,
    Approved,
    Denied,
}

impl SecurityAskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityAskStatus::Pending => "pending",
            SecurityAskStatus::Approved => "approved",
            SecurityAskStatus::Denied => "denied",
        }
    }

    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(SecurityAskStatus::Pending),
            "approved" => Some(SecurityAskStatus::Approved),
            "denied" => Some(SecurityAskStatus::Denied),
            _ => None,
        }
    }
}

/// A DB-backed ask lifecycle row. Pending and resolution records are appended
/// rather than updated so forensic replay does not depend on live state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAskEvent {
    pub timestamp_unix_ms: i64,
    pub ask_id: String,
    pub event_id: String,
    pub event_type: String,
    pub rule_id: String,
    pub rule_name: String,
    pub status: SecurityAskStatus,
    pub rule_json: String,
    pub event_json: String,
    #[serde(default)]
    pub resolver: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityAskPending {
    pub timestamp_unix_ms: i64,
    pub ask_id: String,
    pub event_id: String,
    pub event_type: String,
    pub rule_id: String,
    pub rule_name: String,
    pub rule_json: String,
    pub event_json: String,
}

impl SecurityAskEvent {
    pub fn pending(pending: SecurityAskPending) -> Self {
        Self {
            timestamp_unix_ms: pending.timestamp_unix_ms,
            ask_id: pending.ask_id,
            event_id: pending.event_id,
            event_type: pending.event_type,
            rule_id: pending.rule_id,
            rule_name: pending.rule_name,
            status: SecurityAskStatus::Pending,
            rule_json: pending.rule_json,
            event_json: pending.event_json,
            resolver: None,
            reason: None,
            trace_id: None,
        }
    }

    pub fn with_status(mut self, status: SecurityAskStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_resolver(mut self, resolver: impl Into<String>) -> Self {
        self.resolver = Some(resolver.into());
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

impl SecurityRuleEvent {
    pub fn new(
        timestamp_unix_ms: i64,
        event_id: impl Into<String>,
        event_type: impl Into<String>,
        rule_id: impl Into<String>,
        rule_json: impl Into<String>,
        event_json: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_unix_ms,
            event_id: event_id.into(),
            event_type: event_type.into(),
            rule_id: rule_id.into(),
            rule_action: SecurityRuleAction::Allow,
            detection_level: SecurityDetectionLevel::None,
            rule_json: rule_json.into(),
            event_json: event_json.into(),
            trace_id: None,
            turn_id: None,
            credential_ref: None,
        }
    }

    pub fn with_rule_action(mut self, rule_action: SecurityRuleAction) -> Self {
        self.rule_action = rule_action;
        self
    }

    pub fn with_detection_level(mut self, detection_level: SecurityDetectionLevel) -> Self {
        self.detection_level = detection_level;
        self
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_credential_ref(mut self, credential_ref: impl Into<String>) -> Self {
        self.credential_ref = Some(credential_ref.into());
        self
    }
}

/// The outcome of a domain policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allowed,
    Denied,
    Error,
    /// DNS-only outcome (T3.d): an admin-configured `DnsRedirect`
    /// rule rewrote the answer to a local IP. The query never
    /// touches the upstream resolver. `dns_events.decision =
    /// "redirected"` lets ops query `WHERE decision = 'redirected'`
    /// to see every override that fired in a session.
    Redirected,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Denied => "denied",
            Decision::Error => "error",
            Decision::Redirected => "redirected",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "allowed" => Decision::Allowed,
            "denied" => Decision::Denied,
            "error" => Decision::Error,
            "redirected" => Decision::Redirected,
            other => {
                tracing::warn!(
                    value = other,
                    "unknown decision string in DB, treating as Error"
                );
                Decision::Error
            }
        }
    }
}

/// Serialize SystemTime as f64 epoch seconds (for frontend compatibility).
fn serialize_timestamp<S: serde::Serializer>(ts: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let epoch = ts
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    s.serialize_f64(epoch.as_secs_f64())
}

/// Deserialize f64 epoch seconds back to SystemTime.
fn deserialize_timestamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let secs: f64 = serde::Deserialize::deserialize(d)?;
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs))
}

/// The canonical file action vocabulary for filesystem and explicit boundary events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Created,
    Modified,
    Deleted,
    Restored,
    Read,
    Imported,
    Exported,
}

impl FileAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileAction::Created => "created",
            FileAction::Modified => "modified",
            FileAction::Deleted => "deleted",
            FileAction::Restored => "restored",
            FileAction::Read => "read",
            FileAction::Imported => "import",
            FileAction::Exported => "export",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "created" => FileAction::Created,
            "modified" => FileAction::Modified,
            "deleted" => FileAction::Deleted,
            "restored" => FileAction::Restored,
            "read" => FileAction::Read,
            "import" => FileAction::Imported,
            "export" => FileAction::Exported,
            other => {
                tracing::warn!(
                    value = other,
                    "unknown file action string in DB, treating as Modified"
                );
                FileAction::Modified
            }
        }
    }
}

/// A single filesystem event from the in-VM inotify watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub action: FileAction,
    pub path: String,
    pub size: Option<u64>,
    /// W6: ambient trace_id for the operation that triggered this event
    /// (lower 16 hex of the W3C trace_id). None when no trace context.
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A single network connection event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub domain: String,
    pub port: u16,
    pub decision: Decision,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub duration_ms: u64,
    pub matched_rule: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_body_preview: Option<String>,
    pub response_body_preview: Option<String>,
    #[serde(default)]
    pub request_body_full: Option<String>,
    #[serde(default)]
    pub response_body_full: Option<String>,
    pub conn_type: Option<String>,
    #[serde(default)]
    pub policy_mode: Option<String>,
    #[serde(default)]
    pub policy_action: Option<String>,
    #[serde(default)]
    pub policy_rule: Option<String>,
    #[serde(default)]
    pub policy_reason: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A tool call emitted by the model in a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEntry {
    pub call_index: u32,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Option<String>,
    /// "native" (model built-in, executed in VM) or "mcp" (routed through MCP endpoint).
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default)]
    pub trace_id: Option<String>,
}

fn default_origin() -> String {
    "native".to_string()
}

/// A tool result sent back to the model in a subsequent request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseEntry {
    pub call_id: String,
    pub content_preview: Option<String>,
    pub is_error: bool,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A single MCP tool call event (one row per tools/call or tools/list request).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCall {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub server_name: String,
    pub method: String,
    pub tool_name: Option<String>,
    pub request_id: Option<String>,
    pub request_preview: Option<String>,
    pub response_preview: Option<String>,
    /// "allowed", "warned", "denied", "error"
    pub decision: String,
    pub duration_ms: u64,
    pub error_message: Option<String>,
    pub process_name: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    #[serde(default)]
    pub policy_mode: Option<String>,
    #[serde(default)]
    pub policy_action: Option<String>,
    #[serde(default)]
    pub policy_rule: Option<String>,
    #[serde(default)]
    pub policy_reason: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A denormalized AI model API call (one row per request+response cycle),
/// with nested tool data inserted into separate tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCall {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub provider: String,
    #[serde(default)]
    pub protocol: Option<String>,
    pub model: Option<String>,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub method: String,
    pub path: String,
    pub stream: bool,
    // Request metadata
    pub system_prompt_preview: Option<String>,
    pub messages_count: usize,
    pub tools_count: usize,
    pub request_bytes: u64,
    pub request_body_preview: Option<String>,
    #[serde(default)]
    pub request_body_full: Option<String>,
    // Response metadata
    pub message_id: Option<String>,
    pub status_code: Option<u16>,
    pub text_content: Option<String>,
    pub thinking_content: Option<String>,
    #[serde(default)]
    pub response_body_full: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_details: BTreeMap<String, u64>,
    pub duration_ms: u64,
    pub response_bytes: u64,
    // Cost estimate
    pub estimated_cost_usd: f64,
    // Trace grouping
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    // Nested tool data (inserted into separate tables)
    pub tool_calls: Vec<ToolCallEntry>,
    pub tool_responses: Vec<ToolResponseEntry>,
}

/// A structured exec command event (Layer 1: host-side recording of API-path commands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub exec_id: u64,
    pub command: String,
    /// Request origin: "mcp", "cli", "api", "frontend".
    pub source: String,
    pub trace_id: Option<String>,
    pub process_name: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// Completion data for a structured exec command (sent when GuestToHost::ExecDone arrives).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecEventComplete {
    pub exec_id: u64,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub pid: Option<u32>,
}

/// A single DNS resolution event recorded by the host-side DNS proxy
/// (T3). One row per query, with the policy decision + upstream
/// resolver wall time + structured query / answer metadata. The
/// `trace_id` correlates back to the same agent action that emitted
/// the corresponding `net_events` / `model_calls` row -- a
/// `dig anthropic.com` followed by a `curl https://anthropic.com/`
/// shows up as one trace_id with one dns_events row + one net_events
/// row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    /// Query hostname, lowercased, no trailing dot
    /// (e.g. "anthropic.com", not "anthropic.com.").
    pub qname: String,
    /// DNS qtype as a u16 (1 = A, 28 = AAAA, 16 = TXT, ...).
    pub qtype: u16,
    /// DNS qclass as a u16 (almost always 1 = IN).
    pub qclass: u16,
    /// DNS response code (0 = NoError, 2 = ServFail, 3 = NXDomain).
    pub rcode: u16,
    /// First A/AAAA answer observed in the response, when the DNS proxy
    /// received a parseable answer packet.
    #[serde(default)]
    pub answer_ip: Option<String>,
    /// "allowed" / "denied" / "error" (mirrors `Decision::as_str`).
    pub decision: String,
    /// Policy rule that produced a Denied decision, e.g.
    /// "api.openai.com", "*.openai.com", or "default". None for
    /// Allowed / Error.
    pub matched_rule: Option<String>,
    /// "udp" or "tcp" -- the source-side transport from the guest
    /// (NOT the upstream-side transport, which is always UDP today).
    pub source_proto: Option<String>,
    /// Optional process name from the guest agent. T3.3 ships None
    /// because the agent can't reliably correlate a UDP source port
    /// to a guest pid before the transient socket is gone; T5
    /// hardening may revisit if `/proc/net/udp` poll timing improves.
    pub process_name: Option<String>,
    /// Wall time of the upstream resolve attempt in milliseconds.
    /// 0 when the policy short-circuits (Denied) or input parsing
    /// fails (Error).
    pub upstream_resolver_ms: u64,
    /// W6: ambient trace_id for the agent action that triggered this
    /// DNS query. None when no trace context is set.
    #[serde(default)]
    pub trace_id: Option<String>,
    /// Policy engine mode that produced this decision, if any.
    #[serde(default)]
    pub policy_mode: Option<String>,
    /// Typed policy action (`allow`, `ask`, `block`, `rewrite`) when
    /// security rule matched.
    #[serde(default)]
    pub policy_action: Option<String>,
    /// Fully qualified policy rule id, e.g. `policy.dns.block_openai`.
    #[serde(default)]
    pub policy_rule: Option<String>,
    /// Human-readable policy reason or fail-closed detail.
    #[serde(default)]
    pub policy_reason: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A kernel audit event (Layer 3: execve syscalls captured by auditd).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub comm: Option<String>,
    pub argv: String,
    pub cwd: Option<String>,
    pub tty: Option<String>,
    pub session_id: Option<u32>,
    pub audit_id: Option<String>,
    pub exec_event_id: Option<i64>,
    pub parent_exe: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A redacted audit record emitted by the brokered substitution pre-plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstitutionEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(
        serialize_with = "serialize_timestamp",
        deserialize_with = "deserialize_timestamp"
    )]
    pub timestamp: SystemTime,
    pub material_class: String,
    pub source: String,
    pub event_type: Option<String>,
    pub algorithm: String,
    pub substitution_ref: String,
    pub outcome: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub context_json: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn decision_roundtrip() {
        for decision in [
            Decision::Allowed,
            Decision::Denied,
            Decision::Error,
            Decision::Redirected,
        ] {
            assert_eq!(Decision::parse_str(decision.as_str()), decision);
        }
    }

    #[test]
    fn decision_redirected_string() {
        assert_eq!(Decision::Redirected.as_str(), "redirected");
        assert_eq!(Decision::parse_str("redirected"), Decision::Redirected);
    }

    #[test]
    fn decision_json_roundtrip() {
        let event = NetEvent {
            event_id: None,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            domain: "elie.net".to_string(),
            port: 443,
            decision: Decision::Allowed,
            process_name: None,
            pid: None,
            method: None,
            path: None,
            query: None,
            status_code: None,
            bytes_sent: 0,
            bytes_received: 0,
            duration_ms: 0,
            matched_rule: None,
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            request_body_full: None,
            response_body_full: None,
            conn_type: None,
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: NetEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.decision, Decision::Allowed);
        assert_eq!(decoded.domain, "elie.net");
    }

    #[test]
    fn decision_unknown_str() {
        assert_eq!(Decision::parse_str("bogus"), Decision::Error);
        assert_eq!(Decision::parse_str(""), Decision::Error);
    }

    #[test]
    fn file_action_roundtrip() {
        for action in [
            FileAction::Created,
            FileAction::Modified,
            FileAction::Deleted,
            FileAction::Restored,
            FileAction::Read,
            FileAction::Imported,
            FileAction::Exported,
        ] {
            assert_eq!(FileAction::parse_str(action.as_str()), action);
        }
    }

    #[test]
    fn file_action_unknown_str() {
        assert_eq!(FileAction::parse_str("bogus"), FileAction::Modified);
        assert_eq!(FileAction::parse_str(""), FileAction::Modified);
    }

    /// "error" must be an explicit match arm, not caught by the _ wildcard.
    /// This ensures adding future variants (e.g. Timeout) won't silently
    /// map their as_str() to Decision::Error via the catchall.
    #[test]
    fn decision_from_str_explicitly_matches_error() {
        // "error" should match explicitly, not via _ => Error.
        assert_eq!(Decision::parse_str("error"), Decision::Error);
        // Verify the roundtrip: as_str -> from_str for all variants.
        assert_eq!(Decision::parse_str("allowed"), Decision::Allowed);
        assert_eq!(Decision::parse_str("denied"), Decision::Denied);
        assert_eq!(Decision::parse_str("error"), Decision::Error);
    }

    #[test]
    fn credential_reference_is_domain_separated_and_stable() {
        let raw = "sk-test-credential";
        let openai = credential_reference("openai", raw);
        let openai_again = credential_reference("openai", raw);
        let github = credential_reference("github", raw);

        assert_eq!(openai, openai_again);
        assert_ne!(openai, github);
        assert!(is_credential_reference(&openai));
        assert!(!is_credential_reference(raw));
        assert!(openai.starts_with(CREDENTIAL_REF_PREFIX));
    }
}
