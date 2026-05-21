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
    let mut current = event_value;
    for part in field_path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}
