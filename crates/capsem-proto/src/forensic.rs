//! Sparse, named MessagePack for Capsem-owned security forensic projections.
//!
//! Captured HTTP/model bodies are separate exact archive records. This type
//! describes only the structured projection; absent optional fields and empty
//! event lists decode to their declared defaults.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::repeated::MAX_ENCODED_EVENT_BYTES;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityForensicEvent {
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_observations: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_injections: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_trace: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detections: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_executions: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_request: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl SecurityForensicEvent {
    pub fn from_json(json: &str, event_type: &str) -> Result<Self> {
        let mut value: Value = serde_json::from_str(json).context("parse security forensic JSON")?;
        let fields = value
            .as_object_mut()
            .context("security forensic projection must be an object")?;
        fields
            .entry("event_type")
            .or_insert_with(|| Value::String(event_type.to_owned()));
        prune_declared_defaults(&mut value, true, false);
        serde_json::from_value(value).context("decode security forensic projection")
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let encoded = rmp_serde::to_vec_named(self).context("encode security forensic MessagePack")?;
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "security forensic payload exceeds limit"
        );
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            encoded.len() <= MAX_ENCODED_EVENT_BYTES,
            "security forensic payload exceeds limit"
        );
        rmp_serde::from_slice(encoded).context("decode security forensic MessagePack")
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("render security forensic JSON")
    }
}

/// Forensic subobjects are Capsem-owned optional fields. Opaque MCP arguments
/// and content are third-party JSON and must retain explicit nulls and empty
/// collections: pruning those would change what the source actually sent.
fn prune_declared_defaults(value: &mut Value, root: bool, opaque: bool) {
    if opaque {
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, nested) in fields.iter_mut() {
                if !root || known_root_field(key) {
                    prune_declared_defaults(nested, false, matches!(key.as_str(), "arguments" | "content"));
                }
            }
            fields.retain(|key, nested| {
                (root && !known_root_field(key))
                    || (!nested.is_null()
                        && !(nested.as_array().is_some_and(Vec::is_empty)
                            && matches!(
                                key.as_str(),
                                "credential_observations"
                                    | "credential_injections"
                                    | "action_trace"
                                    | "detections"
                                    | "plugin_executions"
                            ))
                        && !(nested.as_object().is_some_and(serde_json::Map::is_empty)
                            && matches!(key.as_str(), "server" | "tool_call" | "request" | "response" | "event"))
                        && !(nested == false && matches!(key.as_str(), "body_truncated" | "valid")))
            });
        }
        Value::Array(values) => {
            for nested in values {
                prune_declared_defaults(nested, false, false);
            }
        }
        _ => {}
    }
}

fn known_root_field(key: &str) -> bool {
    matches!(
        key,
        "event_type"
            | "credential_ref"
            | "credential_observations"
            | "credential_injections"
            | "action_trace"
            | "decision"
            | "detections"
            | "plugin_executions"
            | "container"
            | "http_request"
            | "http"
            | "dns"
            | "mcp"
            | "model"
            | "file"
            | "process"
            | "ip"
            | "tcp"
            | "udp"
            | "network"
    )
}
