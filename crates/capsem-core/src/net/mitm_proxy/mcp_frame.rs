//! Framed MCP JSON-RPC over the MITM vsock port.
//!
//! Guest-originated MCP reaches the MITM endpoint as bounded JSON-RPC frames
//! on vsock:5002. The MITM owns parsing, policy decisions, dispatch through
//! the low-privilege aggregator, and `mcp_calls` telemetry.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use anyhow::{bail, Context, Result};
use capsem_logger::{DbWriter, Decision, McpCall, WriteOp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, warn};

use crate::mcp::types::{parse_namespaced, parse_resource_uri, JsonRpcRequest, JsonRpcResponse};
use crate::net::policy_config::SecurityRuleSet;
use crate::security_engine::{
    emit_matching_security_rules, emit_security_write, evaluate_security_boundary,
    McpSecurityEvent, RuntimeSecurityEventType, SecurityEnforcementAction,
    SecurityEnforcementDecision, SecurityEvent,
};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::metrics;
use super::McpEndpointState;

const MCP_JSON_RPC_MAX_BYTES: usize =
    capsem_proto::MCP_FRAME_MAX_SIZE - capsem_proto::MCP_FRAME_HEADER_LEN as usize;
const MCP_REQUEST_PREVIEW_BYTES: usize = 4096;

pub(super) async fn serve(
    initial_buf: Vec<u8>,
    vsock_stream: AsyncFdStream,
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
) -> Result<String, (String, Decision, String)> {
    serve_io(initial_buf, vsock_stream, endpoint, db).await
}

/// Dispatch an MCP JSON-RPC request through the same security-event and
/// ledger rail used by framed guest MCP traffic.
///
/// Host-facing routes use this when they invoke a profile MCP tool on behalf
/// of the user. They must not call the aggregator directly, because the
/// `mcp_calls` row and matching security-rule rows are the audit contract.
pub async fn dispatch_logged_mcp_request(
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    request: JsonRpcRequest,
    process_name: String,
) -> Option<JsonRpcResponse> {
    let summary = interpret_mcp_method(&request);
    let runtime_event_type = runtime_mcp_event_type(&summary.method);
    let request_decision = evaluate_mcp_security_event(
        &endpoint,
        mcp_security_event_from_summary(runtime_event_type, &summary, &process_name, None),
    );

    if !request_decision.is_allowed() {
        let response = policy_blocked_response(request.id.clone(), "request", &request_decision);
        log_mcp_call_with_policy(
            &db,
            &endpoint.security_rules,
            &request,
            &response,
            &process_name,
            0,
            McpCallPolicyFields::from(&request_decision),
        )
        .await;
        return Some(response);
    }

    let start = Instant::now();
    let response = endpoint.handle_request(&request).await?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let response_decision = evaluate_mcp_security_event(
        &endpoint,
        mcp_security_event_from_summary(
            runtime_mcp_event_type(&summary.method),
            &summary,
            &process_name,
            Some(&response),
        ),
    );
    let final_decision = if response_decision.is_allowed() {
        request_decision
    } else {
        response_decision
    };
    let response = if final_decision.is_allowed() {
        response
    } else {
        policy_blocked_response(request.id.clone(), "response", &final_decision)
    };
    log_mcp_call_with_policy(
        &db,
        &endpoint.security_rules,
        &request,
        &response,
        &process_name,
        duration_ms,
        McpCallPolicyFields::from(&final_decision),
    )
    .await;
    Some(response)
}

async fn serve_io<I>(
    initial_buf: Vec<u8>,
    stream: I,
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
) -> Result<String, (String, Decision, String)>
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = ReplayReader::new(initial_buf, stream);
    let (mut reader, mut writer) = tokio::io::split(io);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<OutboundFrame>(256);
    let streams = Arc::new(Mutex::new(StreamTracker::default()));

    let writer_task = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            if let Err(e) = write_frame(&mut writer, &out).await {
                debug!(error = %e, "framed MCP writer failed");
                break;
            }
        }
    });

    let result: Result<()> = async {
        loop {
            let frame = match read_next_frame(&mut reader).await? {
                FrameRead::Eof => return Ok(()),
                FrameRead::InvalidFrame { stream_id, error } => {
                    warn!(stream_id, error, "invalid framed MCP frame discarded");
                    ::metrics::counter!(
                        metrics::PARSER_EVENTS_TOTAL,
                        "parser" => "mcp_frame",
                        "kind" => "invalid_frame",
                    )
                    .increment(1);

                    if let Some(stream_id) = stream_id.filter(|id| *id != 0) {
                        let response = JsonRpcResponse::err(None, -32600, "invalid MCP frame");
                        send_response(&tx, stream_id, "unknown", &response).await?;
                    }
                    continue;
                }
                FrameRead::Frame(frame) => frame,
            };

            let process_name = if frame.process_name.is_empty() {
                "unknown".to_string()
            } else {
                frame.process_name.clone()
            };

            let disposition = {
                streams
                    .lock()
                    .expect("framed MCP stream tracker poisoned")
                    .begin(frame.stream_id, frame.is_notification())
            };
            let disposition = match disposition {
                Ok(disposition) => disposition,
                Err(e) => {
                    warn!(stream_id = frame.stream_id, error = %e, "framed MCP stream protocol error");
                    return Err(e);
                }
            };

            let request = match parse_json_rpc_payload(&frame.payload) {
                Ok(req) => req,
                Err(e) => {
                    warn!(error = %e, "invalid JSON-RPC in framed MCP request");
                    if disposition == StreamDisposition::Request {
                        let response = JsonRpcResponse::err(e.id, e.code, e.message);
                        send_response(&tx, frame.stream_id, &process_name, &response).await?;
                        streams
                            .lock()
                            .expect("framed MCP stream tracker poisoned")
                            .complete(frame.stream_id);
                    }
                    continue;
                }
            };

            if let Err(e) = validate_frame_request_pair(&frame, &request) {
                warn!(stream_id = frame.stream_id, error = %e, "invalid framed MCP stream/request pair");
                if disposition == StreamDisposition::Request {
                    let response = JsonRpcResponse::err(request.id.clone(), -32600, e.to_string());
                    send_response(&tx, frame.stream_id, &process_name, &response).await?;
                    streams
                        .lock()
                        .expect("framed MCP stream tracker poisoned")
                        .complete(frame.stream_id);
                }
                continue;
            }

            let summary = interpret_mcp_method(&request);
            let runtime_event_type = runtime_mcp_event_type(&summary.method);
            record_method_metric(&summary);
            let request_decision = evaluate_mcp_security_event(
                &endpoint,
                mcp_security_event_from_summary(
                    runtime_event_type,
                    &summary,
                    &process_name,
                    None,
                ),
            );

            ::metrics::counter!(
                metrics::PARSER_EVENTS_TOTAL,
                "parser" => "mcp_json_rpc",
                "kind" => summary.kind.label(),
            )
            .increment(1);

            if disposition == StreamDisposition::Notification {
                let endpoint_h = Arc::clone(&endpoint);
                let db_h = Arc::clone(&db);
                let process_name_h = process_name.clone();
                let request_decision_h = request_decision.clone();
                let request_h = request.clone();
                tokio::spawn(async move {
                    if request_decision_h.is_allowed() {
                        let _ = endpoint_h.handle_request(&request_h).await;
                    }
                    let response = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: None,
                        result: None,
                        error: None,
                        meta: None,
                    };
                    log_mcp_call_with_policy(
                        &db_h,
                        &endpoint_h.security_rules,
                        &request_h,
                        &response,
                        &process_name_h,
                        0,
                        McpCallPolicyFields::from(&request_decision_h),
                    )
                    .await;
                });
                continue;
            }

            let dispatch_request = request.clone();
            if !request_decision.is_allowed() {
                let response = policy_blocked_response(request.id.clone(), "request", &request_decision);
                log_mcp_call_with_policy(
                    &db,
                    &endpoint.security_rules,
                    &dispatch_request,
                    &response,
                    &process_name,
                    0,
                    McpCallPolicyFields::from(&request_decision),
                )
                .await;
                streams
                    .lock()
                    .expect("framed MCP stream tracker poisoned")
                    .complete(frame.stream_id);
                send_response(&tx, frame.stream_id, &process_name, &response).await?;
                continue;
            }

            let permit = match Arc::clone(&endpoint.inflight).acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    warn!("framed MCP inflight semaphore closed");
                    continue;
                }
            };

            let endpoint_h = Arc::clone(&endpoint);
            let db_h = Arc::clone(&db);
            let tx_h = tx.clone();
            let streams_h = Arc::clone(&streams);
            let process_name_h = process_name.clone();
            let summary_h = summary.clone();
            let request_decision_h = request_decision.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let start = Instant::now();
                let response = endpoint_h.handle_request(&dispatch_request).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                streams_h
                    .lock()
                    .expect("framed MCP stream tracker poisoned")
                    .complete(frame.stream_id);
                let Some(response) = response else {
                    return;
                };
                let response_decision = evaluate_mcp_security_event(
                    &endpoint_h,
                    mcp_security_event_from_summary(
                        runtime_mcp_event_type(&summary_h.method),
                        &summary_h,
                        &process_name_h,
                        Some(&response),
                    ),
                );
                let final_decision = if response_decision.is_allowed() {
                    request_decision_h
                } else {
                    response_decision
                };
                let response = if final_decision.is_allowed() {
                    response
                } else {
                    policy_blocked_response(dispatch_request.id.clone(), "response", &final_decision)
                };
                let policy_fields = McpCallPolicyFields::from(&final_decision);
                log_mcp_call_with_policy(
                    &db_h,
                    &endpoint_h.security_rules,
                    &dispatch_request,
                    &response,
                    &process_name_h,
                    duration_ms,
                    policy_fields,
                )
                .await;
                if let Err(e) = send_response(&tx_h, frame.stream_id, &process_name_h, &response).await {
                    debug!(error = %e, "framed MCP response dropped");
                }
            });
        }
    }
    .await;

    drop(tx);
    let _ = writer_task.await;
    match &result {
        Ok(()) => {
            ::metrics::counter!(
                metrics::MCP_DISCONNECTS_TOTAL,
                "reason" => "eof",
            )
            .increment(1);
        }
        Err(_) => {
            ::metrics::counter!(
                metrics::MCP_DISCONNECTS_TOTAL,
                "reason" => "error",
            )
            .increment(1);
        }
    }

    result.map_err(|e| {
        (
            "mcp.capsem.internal".to_string(),
            Decision::Error,
            format!("framed MCP: {e:#}"),
        )
    })?;

    Ok("mcp.capsem.internal".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameRead {
    Eof,
    Frame(capsem_proto::McpFrame),
    InvalidFrame {
        stream_id: Option<u32>,
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamDisposition {
    Request,
    Notification,
}

#[derive(Debug, Default)]
struct StreamTracker {
    highest_seen: u32,
    inflight: HashSet<u32>,
}

impl StreamTracker {
    fn begin(&mut self, stream_id: u32, is_notification: bool) -> Result<StreamDisposition> {
        if is_notification {
            if stream_id != 0 {
                bail!("notification frame must use stream id 0");
            }
            return Ok(StreamDisposition::Notification);
        }
        if stream_id == 0 {
            bail!("stream id 0 is reserved for notifications");
        }
        if self.inflight.contains(&stream_id) {
            bail!("duplicate MCP stream id in flight: {stream_id}");
        }
        if stream_id <= self.highest_seen {
            bail!(
                "non-monotonic MCP stream id: got {stream_id} after {}",
                self.highest_seen
            );
        }

        self.highest_seen = stream_id;
        self.inflight.insert(stream_id);
        Ok(StreamDisposition::Request)
    }

    fn complete(&mut self, stream_id: u32) {
        self.inflight.remove(&stream_id);
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.inflight.is_empty()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct McpMethodSummary {
    kind: McpMethodKind,
    method: String,
    server_name: Option<String>,
    tool_name: Option<String>,
    resource_uri: Option<String>,
    prompt_name: Option<String>,
    request_preview: Option<String>,
    request_hash: String,
    has_request_id: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpMethodKind {
    Initialize,
    InitializedNotification,
    ToolsList,
    ToolsCall,
    ResourcesList,
    ResourcesRead,
    PromptsList,
    PromptsGet,
    Unknown,
}

impl McpMethodKind {
    fn label(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::InitializedNotification => "notifications/initialized",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
            Self::ResourcesList => "resources/list",
            Self::ResourcesRead => "resources/read",
            Self::PromptsList => "prompts/list",
            Self::PromptsGet => "prompts/get",
            Self::Unknown => "unknown",
        }
    }
}

fn response_content(response: &JsonRpcResponse) -> Option<String> {
    if let Some(error) = &response.error {
        return Some(error.message.clone());
    }
    response
        .result
        .as_ref()
        .and_then(|result| serde_json::to_string(result).ok())
}

fn response_text(response: &JsonRpcResponse) -> Option<String> {
    if let Some(error) = &response.error {
        return Some(error.message.clone());
    }
    let mut values = Vec::new();
    if let Some(result) = &response.result {
        collect_text_fields(result, &mut values);
    }
    if values.is_empty() {
        None
    } else {
        Some(values.join("\n"))
    }
}

fn collect_text_fields(value: &serde_json::Value, values: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key == "text" {
                    if let Some(text) = value.as_str() {
                        values.push(text.to_string());
                    }
                }
                collect_text_fields(value, values);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text_fields(item, values);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpCallPolicyFields {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

impl From<&SecurityEnforcementDecision> for McpCallPolicyFields {
    fn from(decision: &SecurityEnforcementDecision) -> Self {
        Self {
            policy_mode: Some("security_event".to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
        }
    }
}

async fn log_mcp_call_with_policy(
    db: &DbWriter,
    security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
    policy_fields: McpCallPolicyFields,
) {
    let (server_name, tool_name) = mcp_log_attribution(req);
    let decision = if policy_fields
        .policy_action
        .as_deref()
        .is_some_and(|action| action == "block" || action == "ask")
    {
        "denied"
    } else if resp.error.is_some() {
        if resp
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("blocked by policy"))
        {
            "denied"
        } else {
            "error"
        }
    } else {
        "allowed"
    };
    let request_preview = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_string(params).ok());
    let response_preview = resp
        .result
        .as_ref()
        .and_then(|result| serde_json::to_string(result).ok());
    let bytes_sent = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_vec(params).ok())
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    let bytes_received = resp
        .result
        .as_ref()
        .and_then(|result| serde_json::to_vec(result).ok())
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);

    let call = McpCall {
        event_id: None,
        timestamp: SystemTime::now(),
        server_name,
        method: req.method.clone(),
        tool_name,
        request_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
        request_preview,
        response_preview,
        decision: decision.to_string(),
        duration_ms,
        error_message: resp.error.as_ref().map(|error| error.message.clone()),
        process_name: Some(process_name.to_string()),
        bytes_sent,
        bytes_received,
        policy_mode: policy_fields.policy_mode,
        policy_action: policy_fields.policy_action,
        policy_rule: policy_fields.policy_rule,
        policy_reason: policy_fields.policy_reason,
        trace_id: crate::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    };
    let security_event = security_event_from_mcp_call(&call);
    if let Some(event_id) = emit_security_write(db, WriteOp::McpCall(call)).await {
        let rules = security_rules.read().unwrap().clone();
        if let Err(error) = emit_matching_security_rules(
            db,
            event_id,
            runtime_mcp_event_type(&req.method),
            &rules,
            &security_event,
            current_unix_ms(),
        )
        .await
        {
            warn!(error = %error, "failed to emit MCP security rule ledger rows");
        }
    }
}

fn security_event_from_mcp_call(call: &McpCall) -> SecurityEvent {
    let security_event =
        SecurityEvent::new(RuntimeSecurityEventType::McpToolCall).with_mcp(McpSecurityEvent {
            method: Some(call.method.clone()),
            server_name: Some(call.server_name.clone()),
            tool_call_name: call.tool_name.clone(),
            tool_list: if call.method == "tools/list" {
                call.response_preview.clone()
            } else {
                None
            },
        });
    match call.trace_id.clone() {
        Some(trace_id) => security_event.with_trace_id(trace_id),
        None => security_event,
    }
}

fn runtime_mcp_event_type(method: &str) -> RuntimeSecurityEventType {
    match method {
        "tools/call" => RuntimeSecurityEventType::McpToolCall,
        "tools/list" => RuntimeSecurityEventType::McpToolList,
        _ => RuntimeSecurityEventType::McpEvent,
    }
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn mcp_security_event_from_summary(
    event_type: RuntimeSecurityEventType,
    summary: &McpMethodSummary,
    process_name: &str,
    response: Option<&JsonRpcResponse>,
) -> SecurityEvent {
    let tool_list = if summary.kind == McpMethodKind::ToolsList {
        response.and_then(response_content)
    } else {
        None
    };
    let event = SecurityEvent::new(event_type).with_mcp(McpSecurityEvent {
        method: Some(summary.method.clone()),
        server_name: summary
            .server_name
            .clone()
            .or_else(|| Some(process_name.to_string())),
        tool_call_name: summary.tool_name.clone(),
        tool_list,
    });
    match crate::telemetry::ambient_capsem_trace_id() {
        Some(trace_id) => event.with_trace_id(trace_id),
        None => event,
    }
}

fn evaluate_mcp_security_event(
    endpoint: &McpEndpointState,
    event: SecurityEvent,
) -> SecurityEnforcementDecision {
    let rules = endpoint.security_rules.read().unwrap().clone();
    let plugin_policy = endpoint.plugin_policy.read().unwrap().clone();
    match evaluate_security_boundary(&rules, plugin_policy, event) {
        Ok(evaluation) => evaluation.enforcement,
        Err(error) => {
            warn!(error = %error, "MCP security event evaluation failed closed");
            SecurityEnforcementDecision {
                action: SecurityEnforcementAction::Block,
                rule_id: Some("security.mcp.evaluation_error".to_string()),
                rule_name: Some("mcp_security_evaluation_error".to_string()),
                reason: Some(error.to_string()),
                ask_id: None,
            }
        }
    }
}

fn policy_blocked_response(
    id: Option<serde_json::Value>,
    subject: &str,
    decision: &SecurityEnforcementDecision,
) -> JsonRpcResponse {
    let rule = decision.rule_id.as_deref().unwrap_or("unknown");
    JsonRpcResponse::err(
        id,
        -32600,
        format!("MCP {subject} blocked by security rule: {rule}"),
    )
}

fn json_rpc_id_to_log_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

#[derive(Debug, Clone)]
struct JsonRpcPayloadError {
    code: i64,
    message: String,
    id: Option<serde_json::Value>,
}

impl fmt::Display for JsonRpcPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for JsonRpcPayloadError {}

struct OutboundFrame {
    stream_id: u32,
    process_name: String,
    payload: Vec<u8>,
}

async fn send_response(
    tx: &tokio::sync::mpsc::Sender<OutboundFrame>,
    stream_id: u32,
    process_name: &str,
    response: &JsonRpcResponse,
) -> Result<()> {
    let payload = serde_json::to_vec(response).context("serialize framed MCP response")?;
    tx.send(OutboundFrame {
        stream_id,
        process_name: process_name.to_string(),
        payload,
    })
    .await
    .context("framed MCP writer channel closed")
}

async fn read_next_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<FrameRead> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(FrameRead::Eof),
        Err(e) => return Err(e).context("read MCP frame length"),
    }

    let total_len = u32::from_be_bytes(len_buf) as usize;
    if !(capsem_proto::MCP_FRAME_HEADER_LEN as usize..=capsem_proto::MCP_FRAME_MAX_SIZE)
        .contains(&total_len)
    {
        bail!("invalid MCP frame length: {total_len}");
    }

    let mut body = vec![0u8; total_len];
    reader
        .read_exact(&mut body)
        .await
        .context("read MCP frame body")?;
    match capsem_proto::decode_mcp_frame_body(&body) {
        Ok(frame) => Ok(FrameRead::Frame(frame)),
        Err(e) => Ok(FrameRead::InvalidFrame {
            stream_id: recover_stream_id(&body),
            error: e.to_string(),
        }),
    }
}

fn recover_stream_id(body: &[u8]) -> Option<u32> {
    if body.len() < 8 {
        return None;
    }
    Some(u32::from_be_bytes([body[4], body[5], body[6], body[7]]))
}

fn parse_json_rpc_payload(
    payload: &[u8],
) -> std::result::Result<JsonRpcRequest, JsonRpcPayloadError> {
    if payload.len() > MCP_JSON_RPC_MAX_BYTES {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: format!("JSON-RPC payload too large: {} bytes", payload.len()),
            id: None,
        });
    }

    let value =
        serde_json::from_slice::<serde_json::Value>(payload).map_err(|e| JsonRpcPayloadError {
            code: -32700,
            message: format!("parse error: {e}"),
            id: None,
        })?;

    let id = value.get("id").cloned();
    if value.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: "unsupported JSON-RPC version".to_string(),
            id,
        });
    }
    let missing_method = value
        .get("method")
        .and_then(|v| v.as_str())
        .map(|method| method.is_empty())
        .unwrap_or(true);
    if missing_method {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: "missing JSON-RPC method".to_string(),
            id,
        });
    }

    serde_json::from_value(value).map_err(|e| JsonRpcPayloadError {
        code: -32600,
        message: format!("invalid JSON-RPC request: {e}"),
        id: None,
    })
}

fn validate_frame_request_pair(frame: &capsem_proto::McpFrame, req: &JsonRpcRequest) -> Result<()> {
    match (frame.is_notification(), req.id.is_some()) {
        (true, false) => Ok(()),
        (true, true) => bail!("notification stream carried a JSON-RPC id"),
        (false, true) => Ok(()),
        (false, false) => bail!("request stream is missing a JSON-RPC id"),
    }
}

fn interpret_mcp_method(req: &JsonRpcRequest) -> McpMethodSummary {
    let mut server_name = None;
    let mut tool_name = None;
    let mut resource_uri = None;
    let mut prompt_name = None;

    let kind = match req.method.as_str() {
        "initialize" => McpMethodKind::Initialize,
        "notifications/initialized" => McpMethodKind::InitializedNotification,
        "tools/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::ToolsList
        }
        "tools/call" => {
            if let Some(name) = param_str(req, "name") {
                server_name = parse_namespaced(name)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
                tool_name = Some(name.to_string());
            }
            McpMethodKind::ToolsCall
        }
        "resources/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::ResourcesList
        }
        "resources/read" => {
            if let Some(uri) = param_str(req, "uri") {
                server_name = parse_resource_uri(uri)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
                resource_uri = Some(uri.to_string());
            }
            McpMethodKind::ResourcesRead
        }
        "prompts/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::PromptsList
        }
        "prompts/get" => {
            if let Some(name) = param_str(req, "name") {
                server_name = parse_namespaced(name)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
                prompt_name = Some(name.to_string());
            }
            McpMethodKind::PromptsGet
        }
        _ => McpMethodKind::Unknown,
    };

    let request_bytes = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_vec(params).ok())
        .unwrap_or_default();
    let request_hash = blake3::hash(&request_bytes).to_hex().to_string();
    let request_preview = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_string(params).ok())
        .map(|preview| truncate_preview(&preview, MCP_REQUEST_PREVIEW_BYTES));

    McpMethodSummary {
        kind,
        method: req.method.clone(),
        server_name,
        tool_name,
        resource_uri,
        prompt_name,
        request_preview,
        request_hash,
        has_request_id: req.id.is_some(),
    }
}

fn param_str<'a>(req: &'a JsonRpcRequest, key: &str) -> Option<&'a str> {
    req.params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
}

fn mcp_log_attribution(req: &JsonRpcRequest) -> (String, Option<String>) {
    match req.method.as_str() {
        "tools/call" => {
            let tool_name = param_str(req, "name").map(String::from);
            let server_name = tool_name
                .as_deref()
                .and_then(parse_namespaced)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, tool_name)
        }
        "resources/read" => {
            let server_name = param_str(req, "uri")
                .and_then(parse_resource_uri)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, None)
        }
        "prompts/get" => {
            let server_name = param_str(req, "name")
                .and_then(parse_namespaced)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, None)
        }
        "tools/list" | "resources/list" | "prompts/list" => ("*".to_string(), None),
        _ => ("gateway".to_string(), None),
    }
}

fn truncate_preview(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

fn record_method_metric(summary: &McpMethodSummary) {
    ::metrics::counter!(
        metrics::MCP_METHODS_TOTAL,
        "method" => summary.method.clone(),
        "kind" => summary.kind.label(),
    )
    .increment(1);
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, out: &OutboundFrame) -> Result<()> {
    let bytes = capsem_proto::encode_mcp_frame(out.stream_id, 0, &out.process_name, &out.payload)?;
    writer.write_all(&bytes).await.context("write MCP frame")?;
    writer.flush().await.context("flush MCP frame")
}

#[cfg(test)]
mod tests;
