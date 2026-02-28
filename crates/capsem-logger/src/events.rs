use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The outcome of a domain policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allowed,
    Denied,
    Error,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Denied => "denied",
            Decision::Error => "error",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "allowed" => Decision::Allowed,
            "denied" => Decision::Denied,
            "error" => Decision::Error,
            other => {
                tracing::warn!(value = other, "unknown decision string in DB, treating as Error");
                Decision::Error
            }
        }
    }
}

/// Serialize SystemTime as f64 epoch seconds (for frontend compatibility).
fn serialize_timestamp<S: serde::Serializer>(ts: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let epoch = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    s.serialize_f64(epoch.as_secs_f64())
}

/// Deserialize f64 epoch seconds back to SystemTime.
fn deserialize_timestamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let secs: f64 = serde::Deserialize::deserialize(d)?;
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs))
}

/// A single network connection event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
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
    pub conn_type: Option<String>,
}

/// A tool call emitted by the model in a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEntry {
    pub call_index: u32,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Option<String>,
}

/// A tool result sent back to the model in a subsequent request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseEntry {
    pub call_id: String,
    pub content_preview: Option<String>,
    pub is_error: bool,
}

/// A denormalized AI model API call (one row per request+response cycle),
/// with nested tool data inserted into separate tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCall {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub provider: String,
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
    // Response metadata
    pub message_id: Option<String>,
    pub status_code: Option<u16>,
    pub text_content: Option<String>,
    pub thinking_content: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub duration_ms: u64,
    pub response_bytes: u64,
    // Cost estimate
    pub estimated_cost_usd: f64,
    // Trace grouping
    pub trace_id: Option<String>,
    // Nested tool data (inserted into separate tables)
    pub tool_calls: Vec<ToolCallEntry>,
    pub tool_responses: Vec<ToolResponseEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn decision_roundtrip() {
        for decision in [Decision::Allowed, Decision::Denied, Decision::Error] {
            assert_eq!(Decision::parse_str(decision.as_str()), decision);
        }
    }

    #[test]
    fn decision_json_roundtrip() {
        let event = NetEvent {
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
            conn_type: None,
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
}
