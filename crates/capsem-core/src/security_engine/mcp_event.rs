use super::{borrowed_string, json_string};
use capsem_config::PolicySubjectValue;
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpSecurityEvent {
    pub method: Option<String>,
    pub server_name: Option<String>,
    pub tool_call_name: Option<String>,
    pub tool_list: Option<String>,
    pub request: Option<McpRequestSecurityEvent>,
    pub response: Option<McpResponseSecurityEvent>,
    pub error: Option<McpErrorSecurityEvent>,
}

impl Serialize for McpSecurityEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct ValidFact<'a> {
            valid: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
        }

        let server = ValidFact {
            valid: self.server_name.is_some(),
            name: self.server_name.as_deref(),
        };
        let tool_call = ValidFact {
            valid: self.tool_call_name.is_some(),
            name: self.tool_call_name.as_deref(),
        };

        let mut state = serializer.serialize_struct("McpSecurityEvent", 10)?;
        state.serialize_field("method", &self.method)?;
        state.serialize_field("server_name", &self.server_name)?;
        state.serialize_field("tool_call_name", &self.tool_call_name)?;
        state.serialize_field("tool_list", &self.tool_list)?;
        state.serialize_field("request", &self.request)?;
        state.serialize_field("response", &self.response)?;
        state.serialize_field("error", &self.error)?;
        state.serialize_field("server", &server)?;
        state.serialize_field("tool_call", &tool_call)?;
        state.serialize_field(
            "event",
            &ValidFact {
                valid: self.method.is_some(),
                name: None,
            },
        )?;
        state.end()
    }
}

impl McpSecurityEvent {
    pub fn with_request_preview(mut self, preview: Option<&str>) -> Self {
        self.request = preview.and_then(mcp_request_from_preview);
        self
    }

    pub fn with_response_preview(mut self, preview: Option<&str>) -> Self {
        self.response = preview.map(mcp_response_from_preview);
        self
    }

    pub fn with_error_message(mut self, message: Option<&str>) -> Self {
        self.error = message.map(|message| McpErrorSecurityEvent {
            message: Some(message.to_string()),
        });
        self
    }

    pub(super) fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(true)),
            "method" => borrowed_string(self.method.as_deref()),
            "server.name" => borrowed_string(self.server_name.as_deref()),
            "server.valid" => Some(PolicySubjectValue::Bool(self.server_name.is_some())),
            "tool_call.valid" => Some(PolicySubjectValue::Bool(self.tool_call_name.is_some())),
            "tool_call.name" => borrowed_string(self.tool_call_name.as_deref()),
            "tool_list.valid" => Some(PolicySubjectValue::Bool(self.tool_list.is_some())),
            "tool_list" => borrowed_string(self.tool_list.as_deref()),
            "request.valid" => Some(PolicySubjectValue::Bool(self.request.is_some())),
            "request.id" => borrowed_string(self.request.as_ref().and_then(|request| request.id.as_deref())),
            "request.method" => borrowed_string(self.request.as_ref().and_then(|request| request.method.as_deref())),
            "request.arguments" => json_string(self.request.as_ref().and_then(|request| request.arguments.as_ref())),
            "response.valid" => Some(PolicySubjectValue::Bool(self.response.is_some())),
            "response.content" => json_string(self.response.as_ref().and_then(|response| response.content.as_ref())),
            "event.valid" => Some(PolicySubjectValue::Bool(self.method.is_some())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct McpRequestSecurityEvent {
    pub id: Option<String>,
    pub method: Option<String>,
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct McpResponseSecurityEvent {
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct McpErrorSecurityEvent {
    pub message: Option<String>,
}

fn mcp_request_from_preview(preview: &str) -> Option<McpRequestSecurityEvent> {
    let value: serde_json::Value = serde_json::from_str(preview).ok()?;
    let arguments = value
        .pointer("/params/arguments")
        .or_else(|| value.pointer("/arguments"))
        .cloned();
    Some(McpRequestSecurityEvent {
        id: None,
        method: None,
        arguments,
    })
}

fn mcp_response_from_preview(preview: &str) -> McpResponseSecurityEvent {
    // A JSON-RPC error preview may be plain text. Response presence records
    // the observed phase, independently of whether structured content exists.
    let content = serde_json::from_str::<serde_json::Value>(preview)
        .ok()
        .and_then(|mut value| {
            if let Some(content) = value.pointer_mut("/result/content") {
                Some(content.take())
            } else {
                value.pointer_mut("/content").map(|content| content.take())
            }
        });
    McpResponseSecurityEvent { content }
}
