//! Observation of MCP JSON-RPC requests carried over plain HTTP through the proxy.

use crate::security_engine::{McpRequestSecurityEvent, McpSecurityEvent, RuntimeSecurityEventType, SecurityEvent};

use super::{mcp_frame, MCP_BODY_CAPTURE_LIMIT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ObservedMcpHttpRequest {
    pub(super) method: String,
    pub(super) server_name: String,
    pub(super) tool_name: Option<String>,
    pub(super) request_id: Option<String>,
    pub(super) request_preview: Option<String>,
    pub(super) request_arguments: Option<serde_json::Value>,
    pub(super) bytes_sent: u64,
}

impl ObservedMcpHttpRequest {
    pub(super) fn event_type(&self) -> RuntimeSecurityEventType {
        runtime_mcp_event_type(&self.method)
    }

    pub(super) fn security_event(&self, tool_list: Option<String>, response_preview: Option<&str>) -> SecurityEvent {
        let event = SecurityEvent::new(self.event_type()).with_mcp(
            McpSecurityEvent {
                method: Some(self.method.clone()),
                server_name: Some(self.server_name.clone()),
                tool_call_name: self.tool_name.clone(),
                tool_list,
                request: Some(McpRequestSecurityEvent {
                    id: self.request_id.clone(),
                    method: Some(self.method.clone()),
                    arguments: self.request_arguments.clone(),
                }),
                ..Default::default()
            }
            .with_response_preview(response_preview),
        );
        match capsem_foundation::telemetry::ambient_capsem_trace_id() {
            Some(trace_id) => event.with_trace_id(trace_id),
            None => event,
        }
    }
}

pub(super) fn should_sniff_mcp_http_body(method: &http::Method, headers: &http::HeaderMap) -> bool {
    if !matches!(*method, http::Method::POST | http::Method::PUT | http::Method::PATCH) {
        return false;
    }
    let is_json = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false);
    if !is_json {
        return false;
    }
    let Some(len) = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return false;
    };
    len <= MCP_BODY_CAPTURE_LIMIT
}

pub(super) fn observed_mcp_http_request_for_body(
    body: &[u8],
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> Option<ObservedMcpHttpRequest> {
    if body.len() > MCP_BODY_CAPTURE_LIMIT {
        return None;
    }
    // Targeted deserialization instead of a full serde_json::Value tree: the
    // Keep params borrowed until the request shape is known; policy evaluation
    // then parses arguments in full while the ledger retains a capped preview.
    #[derive(serde::Deserialize)]
    struct McpRequestSniff<'a> {
        #[serde(default)]
        jsonrpc: Option<String>,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        id: Option<serde_json::Value>,
        #[serde(borrow, default)]
        params: Option<&'a serde_json::value::RawValue>,
    }
    #[derive(serde::Deserialize)]
    struct ParamsSniff {
        name: Option<String>,
        arguments: Option<serde_json::Value>,
    }

    let sniff: McpRequestSniff = serde_json::from_slice(body).ok()?;
    if sniff.jsonrpc.as_deref() != Some("2.0") {
        return None;
    }
    let method = sniff.method?;
    if !is_mcp_json_rpc_method(&method) {
        return None;
    }
    let request_id = sniff.id.as_ref().and_then(json_rpc_id_to_log_string);
    let params = sniff
        .params
        .and_then(|params| serde_json::from_str::<ParamsSniff>(params.get()).ok());
    let tool_name = if method == "tools/call" {
        params.as_ref().and_then(|params| params.name.clone())
    } else {
        None
    };
    let request_arguments = params.and_then(|params| params.arguments);
    Some(ObservedMcpHttpRequest {
        server_name: observed_mcp_server_name(domain, upstream_port, path),
        method,
        tool_name,
        request_id,
        request_arguments,
        // Cap the stored preview like the framed MCP path: the body can be up
        // to MCP_BODY_CAPTURE_LIMIT (10 MB) of guest-controlled JSON, which must
        // not be pushed wholesale into the ledger row.
        request_preview: Some(mcp_frame::truncate_preview(
            &String::from_utf8_lossy(body),
            mcp_frame::MCP_REQUEST_PREVIEW_BYTES,
        )),
        bytes_sent: body.len() as u64,
    })
}

fn is_mcp_json_rpc_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "notifications/initialized"
            | "tools/list"
            | "tools/call"
            | "resources/list"
            | "resources/read"
            | "prompts/list"
            | "prompts/get"
    )
}

fn runtime_mcp_event_type(method: &str) -> RuntimeSecurityEventType {
    match method {
        "tools/call" => RuntimeSecurityEventType::McpToolCall,
        "tools/list" => RuntimeSecurityEventType::McpToolList,
        _ => RuntimeSecurityEventType::McpEvent,
    }
}

fn observed_mcp_server_name(domain: &str, upstream_port: u16, path: &str) -> String {
    format!("observed:{domain}:{upstream_port}{path}")
}

fn json_rpc_id_to_log_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}
