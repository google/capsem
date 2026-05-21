//! Policy Hook Spec0 wire contract and OpenAPI export.
//!
//! The Rust values in this module are the source for the checked-in
//! `config/policy-hook-openapi.json` artifact. The artifact is compact:
//! it captures the stable envelope, callback and decision enums, rewrite
//! fields, and audit-safe metadata without trying to mirror every
//! per-callback subject field as a separate schema.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const POLICY_HOOK_SPEC_VERSION: &str = "policy-hook/v0";
pub const POLICY_HOOK_OPENAPI_PATH: &str = "config/policy-hook-openapi.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookDecision {
    Allow,
    Ask,
    Block,
    Rewrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookCallback {
    #[serde(rename = "mcp.request")]
    McpRequest,
    #[serde(rename = "mcp.response")]
    McpResponse,
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.response")]
    HttpResponse,
    #[serde(rename = "dns.query")]
    DnsQuery,
    #[serde(rename = "dns.response")]
    DnsResponse,
    #[serde(rename = "model.request")]
    ModelRequest,
    #[serde(rename = "model.response")]
    ModelResponse,
    #[serde(rename = "model.tool_call")]
    ModelToolCall,
    #[serde(rename = "model.tool_response")]
    ModelToolResponse,
}

impl HookCallback {
    pub const ALL: [Self; 10] = [
        Self::McpRequest,
        Self::McpResponse,
        Self::HttpRequest,
        Self::HttpResponse,
        Self::DnsQuery,
        Self::DnsResponse,
        Self::ModelRequest,
        Self::ModelResponse,
        Self::ModelToolCall,
        Self::ModelToolResponse,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::McpRequest => "mcp.request",
            Self::McpResponse => "mcp.response",
            Self::HttpRequest => "http.request",
            Self::HttpResponse => "http.response",
            Self::DnsQuery => "dns.query",
            Self::DnsResponse => "dns.response",
            Self::ModelRequest => "model.request",
            Self::ModelResponse => "model.response",
            Self::ModelToolCall => "model.tool_call",
            Self::ModelToolResponse => "model.tool_response",
        }
    }
}

impl HookDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
            Self::Rewrite => "rewrite",
        }
    }

    pub fn audit_status(&self) -> &'static str {
        match self {
            Self::Allow | Self::Rewrite => "allowed",
            Self::Ask | Self::Block => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookAuditContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDecisionRequest {
    pub spec_version: String,
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub on: HookCallback,
    pub subject: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hashes: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<HookAuditContext>,
}

impl HookDecisionRequest {
    pub fn validate_semantics(&self) -> Result<(), String> {
        require_object("subject", &self.subject)?;
        if let Some(preview) = &self.preview {
            require_object("preview", preview)?;
        }
        if let Some(hashes) = &self.hashes {
            require_object("hashes", hashes)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDecisionResponse {
    pub decision: HookDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit_tags: Vec<String>,
}

impl HookDecisionResponse {
    pub fn validate_semantics(&self) -> Result<(), String> {
        let has_target = self
            .rewrite_target
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_value = self.rewrite_value.is_some();
        match self.decision {
            HookDecision::Rewrite => {
                if !has_target {
                    return Err("rewrite decision requires rewrite_target".to_string());
                }
                if !has_value {
                    return Err("rewrite decision requires rewrite_value".to_string());
                }
            }
            HookDecision::Allow | HookDecision::Ask | HookDecision::Block => {
                if self.rewrite_target.is_some() || self.rewrite_value.is_some() {
                    return Err("non-rewrite decisions must not include rewrite fields".to_string());
                }
            }
        }
        Ok(())
    }
}

fn require_object(field: &str, value: &Value) -> Result<(), String> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("{field} must be a JSON object"))
    }
}

pub fn policy_hook_openapi_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Capsem Policy Hook Spec0",
            "version": POLICY_HOOK_SPEC_VERSION,
            "description": "External decision hook contract for Capsem Policy V2."
        },
        "paths": {
            "/v1/health": {
                "get": {
                    "operationId": "health",
                    "responses": {
                        "200": {
                            "description": "Hook server health and supported spec versions.",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/HealthResponse"}
                                }
                            }
                        }
                    }
                }
            },
            "/v1/policy/spec": {
                "get": {
                    "operationId": "policySpec",
                    "responses": {
                        "200": {
                            "description": "The OpenAPI document or compatibility hash implemented by the hook server.",
                            "content": {
                                "application/json": {
                                    "schema": {"type": "object"}
                                }
                            }
                        }
                    }
                }
            },
            "/v1/policy/decision": {
                "post": {
                    "operationId": "policyDecision",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/HookDecisionRequest"}
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "A typed allow, ask, block, or rewrite decision.",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/HookDecisionResponse"}
                                }
                            }
                        }
                    }
                }
            },
            "/v1/policy/batch-decision": {
                "post": {
                    "operationId": "policyBatchDecision",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "array",
                                    "items": {"$ref": "#/components/schemas/HookDecisionRequest"}
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "A decision for every request in order.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": {"$ref": "#/components/schemas/HookDecisionResponse"}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "HealthResponse": {
                    "type": "object",
                    "required": ["ok", "spec_versions"],
                    "properties": {
                        "ok": {"type": "boolean"},
                        "spec_versions": {
                            "type": "array",
                            "items": {"type": "string", "enum": [POLICY_HOOK_SPEC_VERSION]}
                        }
                    }
                },
                "HookCallback": {
                    "type": "string",
                    "enum": [
                        "mcp.request",
                        "mcp.response",
                        "http.request",
                        "http.response",
                        "dns.query",
                        "dns.response",
                        "model.request",
                        "model.response",
                        "model.tool_call",
                        "model.tool_response"
                    ]
                },
                "HookDecision": {
                    "type": "string",
                    "enum": ["allow", "ask", "block", "rewrite"]
                },
                "HookAuditContext": {
                    "type": "object",
                    "properties": {
                        "process_name": {"type": "string"},
                        "pid": {"type": "integer", "minimum": 0},
                        "provider": {"type": "string"},
                        "server_name": {"type": "string"},
                        "domain": {"type": "string"},
                        "config_source": {"type": "string"}
                    },
                    "additionalProperties": false
                },
                "HookDecisionRequest": {
                    "type": "object",
                    "required": ["spec_version", "decision_id", "on", "subject"],
                    "properties": {
                        "spec_version": {"type": "string", "enum": [POLICY_HOOK_SPEC_VERSION]},
                        "decision_id": {"type": "string"},
                        "trace_id": {"type": "string"},
                        "session_id": {"type": "string"},
                        "on": {"$ref": "#/components/schemas/HookCallback"},
                        "subject": {"type": "object", "additionalProperties": true},
                        "preview": {"type": "object", "additionalProperties": true},
                        "hashes": {"type": "object", "additionalProperties": true},
                        "audit_context": {"$ref": "#/components/schemas/HookAuditContext"}
                    },
                    "additionalProperties": false
                },
                "HookDecisionResponse": {
                    "type": "object",
                    "required": ["decision"],
                    "properties": {
                        "decision": {"$ref": "#/components/schemas/HookDecision"},
                        "decision_id": {"type": "string"},
                        "rule_id": {"type": "string"},
                        "priority": {"type": "integer"},
                        "reason": {"type": "string"},
                        "ttl_ms": {"type": "integer", "minimum": 0},
                        "rewrite_target": {"type": "string"},
                        "rewrite_value": {"type": "string"},
                        "redactions": {
                            "type": "array",
                            "items": {"type": "string"}
                        },
                        "audit_tags": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    },
                    "additionalProperties": false
                }
            }
        }
    })
}

pub fn policy_hook_openapi_pretty() -> String {
    let mut text = serde_json::to_string_pretty(&policy_hook_openapi_document())
        .expect("Policy Hook OpenAPI document serializes");
    text.push('\n');
    text
}

pub fn policy_hook_schema_hash() -> String {
    blake3::hash(policy_hook_openapi_pretty().as_bytes())
        .to_hex()
        .to_string()
}

#[cfg(test)]
mod tests;
