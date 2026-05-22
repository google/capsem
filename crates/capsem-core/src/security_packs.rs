use capsem_security_engine::{
    CelDetectionRule, EventFamily as EngineEventFamily, RedactionState as EngineRedactionState,
    SecurityEvent, SecurityEventSubject,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;

pub const DETECTION_IR_V1_SCHEMA_JSON: &str =
    include_str!("../../../schemas/capsem.detection.ir.v1.schema.json");

#[derive(Debug, Error)]
pub enum SecurityPackSchemaError {
    #[error("failed to parse security pack JSON: {0}")]
    ParseJson(#[from] serde_json::Error),
    #[error("security pack schema artifact is invalid: {0}")]
    Compile(String),
    #[error("security pack failed schema validation: {0}")]
    Validation(String),
    #[error("unsupported Detection IR: {0}")]
    UnsupportedDetectionIr(String),
}

pub type Result<T> = std::result::Result<T, SecurityPackSchemaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackStatus {
    Active,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackOwner {
    Corp,
    Vendor,
    User,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionOperator {
    EqualsAny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionIRMatcherV1 {
    pub field_path: String,
    pub operator: DetectionOperator,
    pub values: Vec<Value>,
    pub sigma_field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionIRRuleV1 {
    pub id: String,
    pub source_id: String,
    pub sigma_id: Option<String>,
    pub title: String,
    pub event_family: EventFamily,
    pub condition: String,
    pub matchers: Vec<DetectionIRMatcherV1>,
    pub severity: Severity,
    pub confidence: Confidence,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionIRV1 {
    pub schema: String,
    pub pack_id: String,
    pub pack_version: String,
    pub pack_status: PackStatus,
    pub owner: PackOwner,
    pub rules: Vec<DetectionIRRuleV1>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RedactionState {
    #[default]
    Raw,
    Redacted,
    SummaryOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEventV1 {
    pub event_id: String,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
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
    pub event_family: EventFamily,
    pub event_type: String,
    #[serde(default)]
    pub subject: Map<String, Value>,
    #[serde(default)]
    pub redaction_state: RedactionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionFindingV1 {
    pub event_id: String,
    pub rule_id: String,
    pub pack_id: String,
    pub pack_version: String,
    pub sigma_id: Option<String>,
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub tags: Vec<String>,
    pub matched_fields: BTreeMap<String, Value>,
}

pub fn validate_detection_ir_v1_json(input: &str) -> Result<Value> {
    let value = serde_json::from_str::<Value>(input)?;
    let schema = serde_json::from_str::<Value>(DETECTION_IR_V1_SCHEMA_JSON)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| SecurityPackSchemaError::Compile(error.to_string()))?;
    let errors = validator
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(SecurityPackSchemaError::Validation(errors.join("; ")))
    }
}

pub fn parse_detection_ir_v1_json(input: &str) -> Result<DetectionIRV1> {
    let ir = serde_json::from_str(input)?;
    validate_detection_ir_v1_json(input)?;
    Ok(ir)
}

pub fn evaluate_detection_ir(
    ir: &DetectionIRV1,
    event: &SecurityEventV1,
) -> Vec<DetectionFindingV1> {
    let event_value = match serde_json::to_value(event) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    ir.rules
        .iter()
        .filter_map(|rule| evaluate_rule(ir, rule, event, &event_value))
        .collect()
}

pub fn evaluate_detection_ir_security_event(
    ir: &DetectionIRV1,
    event: &SecurityEvent,
) -> Vec<DetectionFindingV1> {
    let event = SecurityEventV1::from(event);
    evaluate_detection_ir(ir, &event)
}

pub fn compile_detection_ir_to_cel_detection_rules(
    ir: &DetectionIRV1,
) -> Result<Vec<CelDetectionRule>> {
    ir.rules
        .iter()
        .map(|rule| {
            let mut terms = vec![event_family_cel_guard(rule.event_family)?];
            for matcher in &rule.matchers {
                match matcher.operator {
                    DetectionOperator::EqualsAny => {
                        let path = runtime_cel_path(rule.event_family, &matcher.field_path)?;
                        let values = matcher
                            .values
                            .iter()
                            .map(cel_literal)
                            .collect::<Result<Vec<_>>>()?;
                        if values.is_empty() {
                            return Err(SecurityPackSchemaError::UnsupportedDetectionIr(format!(
                                "rule {} matcher {} must contain at least one value",
                                rule.id, matcher.field_path
                            )));
                        }
                        let disjunction = values
                            .into_iter()
                            .map(|value| format!("{path} == {value}"))
                            .collect::<Vec<_>>()
                            .join(" || ");
                        terms.push(format!("({disjunction})"));
                    }
                }
            }

            Ok(CelDetectionRule {
                id: rule.id.clone(),
                pack_id: ir.pack_id.clone(),
                sigma_id: rule.sigma_id.clone(),
                title: rule.title.clone(),
                condition: terms.join(" && "),
                severity: rule.severity.into(),
                confidence: rule.confidence.into(),
                tags: rule.tags.clone(),
            })
        })
        .collect()
}

impl From<Severity> for capsem_security_engine::Severity {
    fn from(value: Severity) -> Self {
        match value {
            Severity::Info => Self::Info,
            Severity::Low => Self::Low,
            Severity::Medium => Self::Medium,
            Severity::High => Self::High,
            Severity::Critical => Self::Critical,
        }
    }
}

impl From<Confidence> for capsem_security_engine::Confidence {
    fn from(value: Confidence) -> Self {
        match value {
            Confidence::Low => Self::Low,
            Confidence::Medium => Self::Medium,
            Confidence::High => Self::High,
        }
    }
}

fn runtime_cel_path(event_family: EventFamily, field_path: &str) -> Result<String> {
    let root = event_family_policy_root(event_family)?;
    let Some((scope, suffix)) = field_path
        .strip_prefix(&format!("{root}.request."))
        .map(|suffix| ("request", suffix))
        .or_else(|| {
            field_path
                .strip_prefix(&format!("{root}.response."))
                .map(|suffix| ("response", suffix))
        })
        .or_else(|| {
            field_path
                .strip_prefix(&format!("{root}.activity."))
                .map(|suffix| ("activity", suffix))
        })
    else {
        return Err(unsupported_field_path(field_path));
    };

    if !is_supported_runtime_field(event_family, scope, suffix) {
        return Err(unsupported_field_path(field_path));
    }

    let canonical_suffix = match (event_family, scope, suffix) {
        _ => suffix,
    };

    Ok(format!("{root}.{scope}.{canonical_suffix}"))
}

fn event_family_policy_root(event_family: EventFamily) -> Result<&'static str> {
    match event_family {
        EventFamily::Dns => Ok("dns"),
        EventFamily::Http => Ok("http"),
        EventFamily::Mcp => Ok("mcp"),
        EventFamily::Model => Ok("model"),
        EventFamily::File => Ok("file"),
        EventFamily::Process => Ok("process"),
        EventFamily::Profile => Ok("profile"),
        EventFamily::Credential | EventFamily::Vm | EventFamily::Conversation => {
            Err(SecurityPackSchemaError::UnsupportedDetectionIr(format!(
                "unsupported Detection IR event family {event_family:?} for CEL lowering"
            )))
        }
    }
}

fn event_family_cel_guard(event_family: EventFamily) -> Result<String> {
    let prefix = match event_family {
        EventFamily::Dns => "dns.",
        EventFamily::Http => "http.",
        EventFamily::Mcp => "mcp.",
        EventFamily::Model => "model.",
        EventFamily::File => "file.",
        EventFamily::Process => "process.",
        EventFamily::Profile => "profile.",
        EventFamily::Credential | EventFamily::Vm | EventFamily::Conversation => {
            return Err(SecurityPackSchemaError::UnsupportedDetectionIr(format!(
                "unsupported Detection IR event family {event_family:?} for CEL lowering"
            )));
        }
    };
    Ok(format!(
        "common.event_type.startsWith({})",
        cel_string_literal(prefix)
    ))
}

fn is_supported_runtime_field(event_family: EventFamily, scope: &str, suffix: &str) -> bool {
    match (event_family, scope, suffix) {
        (EventFamily::Dns, "request", "qname" | "domain_class") => true,
        (
            EventFamily::Http,
            "request",
            "method" | "scheme" | "host" | "port" | "path" | "query" | "url" | "path_class"
            | "bytes" | "body.text",
        ) => true,
        (EventFamily::Http, "response", "status" | "bytes" | "body.text") => true,
        (EventFamily::Mcp, "request", "server_id" | "tool_name") => true,
        (
            EventFamily::Model,
            "request",
            "provider"
            | "model"
            | "estimated_input_tokens"
            | "estimated_output_tokens"
            | "estimated_cost_micros",
        ) => true,
        (EventFamily::File, "activity", "operation" | "path" | "path_class" | "byte_count") => true,
        (EventFamily::Process, "activity", "operation" | "command_class") => true,
        (EventFamily::Credential, "activity", "operation" | "credential_id") => true,
        (EventFamily::Vm, "activity", "operation") => true,
        (EventFamily::Profile, "activity", "operation" | "profile_id" | "profile_revision") => true,
        (EventFamily::Conversation, "activity", "operation" | "conversation_id") => true,
        _ => false,
    }
}

fn unsupported_field_path(field_path: &str) -> SecurityPackSchemaError {
    SecurityPackSchemaError::UnsupportedDetectionIr(format!(
        "unsupported Detection IR field path {field_path:?}"
    ))
}

fn cel_literal(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(cel_string_literal(value)),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Null => Ok("null".into()),
        Value::Array(_) | Value::Object(_) => Err(SecurityPackSchemaError::UnsupportedDetectionIr(
            "Detection IR CEL lowering only supports scalar equals_any values".into(),
        )),
    }
}

fn cel_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string literal should not fail")
}

impl From<&SecurityEvent> for SecurityEventV1 {
    fn from(event: &SecurityEvent) -> Self {
        Self {
            event_id: event.common.event_id.clone(),
            trace_id: event.common.trace_id.clone(),
            span_id: event.common.span_id.clone(),
            timestamp: Some(event.common.timestamp_unix_ms.to_string()),
            vm_id: event.common.vm_id.clone(),
            session_id: event.common.session_id.clone(),
            profile_id: event.common.profile_id.clone(),
            profile_revision: event.common.profile_revision.clone(),
            profile_pack_ids: event.common.profile_pack_ids.clone(),
            user_id: event.common.user_id.clone(),
            process_id: event.common.process_id.clone(),
            parent_process_id: event.common.parent_process_id.clone(),
            exec_id: event.common.exec_id.clone(),
            turn_id: event.common.turn_id.clone(),
            message_id: event.common.message_id.clone(),
            tool_call_id: event.common.tool_call_id.clone(),
            mcp_call_id: event.common.mcp_call_id.clone(),
            event_family: EventFamily::from(event.event_family()),
            event_type: event.common.event_type.clone(),
            subject: security_event_subject_value(&event.subject),
            redaction_state: RedactionState::from(event.common.redaction_state),
        }
    }
}

impl From<EngineEventFamily> for EventFamily {
    fn from(value: EngineEventFamily) -> Self {
        match value {
            EngineEventFamily::Dns => Self::Dns,
            EngineEventFamily::Http => Self::Http,
            EngineEventFamily::Mcp => Self::Mcp,
            EngineEventFamily::Model => Self::Model,
            EngineEventFamily::File | EngineEventFamily::Snapshot => Self::File,
            EngineEventFamily::Process => Self::Process,
            EngineEventFamily::Credential => Self::Credential,
            EngineEventFamily::Vm => Self::Vm,
            EngineEventFamily::Profile => Self::Profile,
            EngineEventFamily::Conversation => Self::Conversation,
        }
    }
}

impl From<EngineRedactionState> for RedactionState {
    fn from(value: EngineRedactionState) -> Self {
        match value {
            EngineRedactionState::Raw => Self::Raw,
            EngineRedactionState::Redacted => Self::Redacted,
            EngineRedactionState::SummaryOnly => Self::SummaryOnly,
        }
    }
}

fn security_event_subject_value(subject: &SecurityEventSubject) -> Map<String, Value> {
    match subject {
        SecurityEventSubject::Dns(subject) => map_from_value(serde_json::json!({
            "request": {
                "qname": subject.qname,
                "domain_class": subject.domain_class,
            }
        })),
        SecurityEventSubject::Http(subject) => map_from_value(serde_json::json!({
            "request": {
                "method": subject.method,
                "host": subject.host,
                "path_class": subject.path_class,
                "request_bytes": subject.request_bytes,
            },
            "response": {
                "response_bytes": subject.response_bytes,
            }
        })),
        SecurityEventSubject::Mcp(subject) => map_from_value(serde_json::json!({
            "request": {
                "server_id": subject.server_id,
                "tool_name": subject.tool_name,
            }
        })),
        SecurityEventSubject::Model(subject) => map_from_value(serde_json::json!({
            "request": {
                "provider": subject.provider,
                "model": subject.model,
                "estimated_input_tokens": subject.estimated_input_tokens,
                "estimated_output_tokens": subject.estimated_output_tokens,
                "estimated_cost_micros": subject.estimated_cost_micros,
            }
        })),
        SecurityEventSubject::File(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "path": subject.path,
                "path_class": subject.path_class,
                "byte_count": subject.byte_count,
            }
        })),
        SecurityEventSubject::Process(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "command_class": subject.command_class,
            }
        })),
        SecurityEventSubject::Credential(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "credential_id": subject.credential_id,
            }
        })),
        SecurityEventSubject::VmLifecycle(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
            }
        })),
        SecurityEventSubject::Profile(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "profile_id": subject.profile_id,
                "profile_revision": subject.profile_revision,
            }
        })),
        SecurityEventSubject::Conversation(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "conversation_id": subject.conversation_id,
            }
        })),
        SecurityEventSubject::Snapshot(subject) => map_from_value(serde_json::json!({
            "activity": {
                "operation": subject.operation,
                "snapshot_id": subject.snapshot_id,
            }
        })),
    }
}

fn map_from_value(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn evaluate_rule(
    ir: &DetectionIRV1,
    rule: &DetectionIRRuleV1,
    event: &SecurityEventV1,
    event_value: &Value,
) -> Option<DetectionFindingV1> {
    if rule.event_family != event.event_family {
        return None;
    }
    let mut matched_fields = BTreeMap::new();
    for matcher in &rule.matchers {
        let value = event_field_value(event_value, &matcher.field_path)?;
        match matcher.operator {
            DetectionOperator::EqualsAny => {
                if !matcher.values.iter().any(|expected| expected == value) {
                    return None;
                }
                matched_fields.insert(matcher.field_path.clone(), value.clone());
            }
        }
    }
    Some(DetectionFindingV1 {
        event_id: event.event_id.clone(),
        rule_id: rule.id.clone(),
        pack_id: ir.pack_id.clone(),
        pack_version: ir.pack_version.clone(),
        sigma_id: rule.sigma_id.clone(),
        title: rule.title.clone(),
        severity: rule.severity,
        confidence: rule.confidence,
        tags: rule.tags.clone(),
        matched_fields,
    })
}

fn event_field_value<'a>(event_value: &'a Value, field_path: &str) -> Option<&'a Value> {
    if let Some(canonical_value) = canonical_event_field_value(event_value, field_path) {
        return Some(canonical_value);
    }
    let mut current = event_value;
    for part in field_path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn canonical_event_field_value<'a>(event_value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let event_family = event_value.get("event_family")?.as_str()?;
    let Some((scope, suffix)) = field_path
        .strip_prefix(&format!("{event_family}.request."))
        .map(|suffix| ("request", suffix))
        .or_else(|| {
            field_path
                .strip_prefix(&format!("{event_family}.response."))
                .map(|suffix| ("response", suffix))
        })
        .or_else(|| {
            field_path
                .strip_prefix(&format!("{event_family}.activity."))
                .map(|suffix| ("activity", suffix))
        })
    else {
        return None;
    };
    let mut current = event_value.get("subject")?.get(scope)?;
    for part in suffix.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}
