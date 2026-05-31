use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current schema version for typed policy context documents.
pub const POLICY_CONTEXT_SCHEMA_VERSION: u16 = 1;

/// Shared typed policy context passed to policy engines.
///
/// This crate owns only the serde schema. It does not evaluate rules, make
/// policy decisions, or adapt this shape into any particular policy language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyContext {
    pub schema_version: u16,
    #[serde(default)]
    pub common: CommonPolicyContext,
    #[serde(default)]
    pub http: HttpPolicyContext,
    #[serde(default)]
    pub dns: DnsPolicyContext,
    #[serde(default)]
    pub mcp: McpPolicyContext,
    #[serde(default)]
    pub model: ModelPolicyContext,
    #[serde(default)]
    pub file: FilePolicyContext,
    #[serde(default)]
    pub process: ProcessPolicyContext,
    #[serde(default)]
    pub credential: CredentialPolicyContext,
    #[serde(default)]
    pub vm: VmPolicyContext,
    #[serde(default)]
    pub profile: ProfilePolicyContext,
    #[serde(default)]
    pub conversation: ConversationPolicyContext,
    #[serde(default)]
    pub snapshot: SnapshotPolicyContext,
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyContext {
    pub fn new() -> Self {
        Self {
            schema_version: POLICY_CONTEXT_SCHEMA_VERSION,
            common: CommonPolicyContext::default(),
            http: HttpPolicyContext::default(),
            dns: DnsPolicyContext::default(),
            mcp: McpPolicyContext::default(),
            model: ModelPolicyContext::default(),
            file: FilePolicyContext::default(),
            process: ProcessPolicyContext::default(),
            credential: CredentialPolicyContext::default(),
            vm: VmPolicyContext::default(),
            profile: ProfilePolicyContext::default(),
            conversation: ConversationPolicyContext::default(),
            snapshot: SnapshotPolicyContext::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonPolicyContext {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub enforceability: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub process: Option<ProcessIdentityPolicyContext>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentityPolicyContext {
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub ppid: Option<u32>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpPolicyContext {
    #[serde(default)]
    pub request: Option<HttpRequestPolicyContext>,
    #[serde(default)]
    pub response: Option<HttpResponsePolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequestPolicyContext {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub scheme: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path_class: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub headers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub body: BodyPolicyContext,
}

impl HttpRequestPolicyContext {
    /// Return the first header value for `name`, comparing names as ASCII
    /// case-insensitive HTTP field names.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_values(name)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    /// Return all header values for `name`. If duplicate keys differ only by
    /// case, the lexicographically first stored key wins because headers are a
    /// `BTreeMap`.
    pub fn header_values(&self, name: &str) -> Option<&[String]> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, values)| values.as_slice())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpResponsePolicyContext {
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub headers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub body: BodyPolicyContext,
}

impl HttpResponsePolicyContext {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_values(name)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    pub fn header_values(&self, name: &str) -> Option<&[String]> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, values)| values.as_slice())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyPolicyContext {
    pub state: BodyState,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub redaction_reason: Option<String>,
}

impl Default for BodyPolicyContext {
    fn default() -> Self {
        Self::missing()
    }
}

impl BodyPolicyContext {
    pub fn missing() -> Self {
        Self {
            state: BodyState::Missing,
            text: None,
            content_type: None,
            size: None,
            truncated: false,
            redaction_reason: None,
        }
    }

    pub fn redacted(reason: impl Into<String>) -> Self {
        Self {
            state: BodyState::Redacted,
            text: None,
            content_type: None,
            size: None,
            truncated: false,
            redaction_reason: Some(reason.into()),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            state: BodyState::Text,
            text: Some(text.into()),
            content_type: None,
            size: None,
            truncated: false,
            redaction_reason: None,
        }
    }

    pub fn binary(length: u64, content_type: Option<String>) -> Self {
        Self {
            state: BodyState::Binary,
            text: None,
            content_type,
            size: Some(length),
            truncated: false,
            redaction_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyState {
    Missing,
    Redacted,
    Text,
    Binary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsPolicyContext {
    #[serde(default)]
    pub request: Option<DnsRequestPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsRequestPolicyContext {
    #[serde(default)]
    pub qname: Option<String>,
    #[serde(default)]
    pub qtype: Option<String>,
    #[serde(default)]
    pub domain_class: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpPolicyContext {
    #[serde(default)]
    pub request: Option<McpRequestPolicyContext>,
    #[serde(default)]
    pub response: Option<McpResponsePolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpRequestPolicyContext {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub arguments_status: Option<String>,
    #[serde(default)]
    pub arguments: BodyPolicyContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpResponsePolicyContext {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub result_status: Option<String>,
    #[serde(default)]
    pub result: BodyPolicyContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPolicyContext {
    #[serde(default)]
    pub request: Option<ModelRequestPolicyContext>,
    #[serde(default)]
    pub response: Option<ModelResponsePolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestPolicyContext {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_family: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub estimated_input_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
    #[serde(default)]
    pub body: BodyPolicyContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ModelToolCallPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolCallPolicyContext {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub provider_call_id: Option<String>,
    #[serde(default)]
    pub raw_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub arguments_status: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub linked_mcp_call_id: Option<String>,
    #[serde(default)]
    pub parse_confidence: Option<String>,
    #[serde(default)]
    pub arguments: BodyPolicyContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResponsePolicyContext {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_family: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default)]
    pub body: BodyPolicyContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ModelToolResultPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolResultPolicyContext {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub linked_mcp_call_id: Option<String>,
    #[serde(default)]
    pub content_kind: Option<String>,
    #[serde(default)]
    pub content_preview: Option<String>,
    #[serde(default)]
    pub content_json: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub result_status: Option<String>,
    #[serde(default)]
    pub returned_to_model: Option<bool>,
    #[serde(default)]
    pub parse_confidence: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePolicyContext {
    #[serde(default)]
    pub activity: Option<FileActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub path_class: Option<String>,
    #[serde(default)]
    pub byte_count: Option<u64>,
    #[serde(default)]
    pub content: BodyPolicyContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicyContext {
    #[serde(default)]
    pub activity: Option<ProcessActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub command_class: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialPolicyContext {
    #[serde(default)]
    pub activity: Option<CredentialActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmPolicyContext {
    #[serde(default)]
    pub activity: Option<VmActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilePolicyContext {
    #[serde(default)]
    pub activity: Option<ProfileActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyContext {
    #[serde(default)]
    pub activity: Option<ConversationActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPolicyContext {
    #[serde(default)]
    pub activity: Option<SnapshotActivityPolicyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotActivityPolicyContext {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
}

#[cfg(test)]
mod tests;
