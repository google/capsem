//! Framed MCP JSON-RPC over the MITM vsock port.
//!
//! Guest-originated MCP reaches the MITM endpoint as bounded JSON-RPC frames
//! on vsock:5002. The MITM owns parsing, policy decisions, dispatch through
//! the low-privilege aggregator, unified tool-call telemetry, and
//! `tool_calls origin = mcp` for actual tool invocations.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use anyhow::{bail, Context, Result};
use capsem_logger::{DbWriter, Decision, McpCall, WriteOp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, warn};

use crate::net::policy_config::{snapshot_plugin_policy, SecurityRuleSet};
use crate::security_engine::{
    delegate_matching_security_rules_for_evaluated_event, emit_security_write, evaluate_security_boundary,
    McpSecurityEvent, ProcessSecurityEvent, RuntimeSecurityEventType, SecurityEnforcementAction,
    SecurityEnforcementDecision, SecurityEvent,
};
use capsem_proto::mcp_contracts::{parse_namespaced, parse_resource_uri, JsonRpcRequest, JsonRpcResponse};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::metrics;
use super::McpEndpointState;
mod wire;
pub(super) use wire::truncate_preview;
use wire::{
    interpret_mcp_method, mcp_log_attribution, parse_json_rpc_payload, read_next_frame, record_method_metric,
    send_response, validate_frame_request_pair, write_frame, OutboundFrame,
};

const MCP_JSON_RPC_MAX_BYTES: usize = capsem_proto::MCP_FRAME_MAX_SIZE - capsem_proto::MCP_FRAME_HEADER_LEN as usize;
pub(super) const MCP_REQUEST_PREVIEW_BYTES: usize = 4096;

/// Deadline for a framed-MCP body once its length prefix has been read. Bounds a
/// guest that announces a frame and then stalls mid-body (slowloris).
const FRAME_BODY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
/// unified tool-call ledger rows and matching security-rule rows are
/// the audit contract.
#[derive(Debug, Clone)]
pub struct LoggedMcpResponse {
    pub response: JsonRpcResponse,
    pub event_id: Option<String>,
}

pub async fn dispatch_logged_mcp_request(
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    request: JsonRpcRequest,
    process_name: String,
) -> Option<LoggedMcpResponse> {
    let summary = interpret_mcp_method(&request);
    let runtime_event_type = runtime_mcp_event_type(&summary.method);
    let request_decision = evaluate_mcp_security_event(
        &endpoint,
        mcp_security_event_from_summary(runtime_event_type, &summary, &process_name, None),
    );

    if !request_decision.is_allowed() {
        let response = policy_blocked_response(request.id.clone(), "request", &request_decision);
        let emission = log_mcp_call_with_policy(
            Arc::clone(&db),
            &endpoint.security_rules,
            &request,
            &response,
            &process_name,
            0,
            McpCallPolicyFields::from(&request_decision),
        )
        .await;
        return Some(LoggedMcpResponse {
            response,
            event_id: emission.event_id,
        });
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
    let emission = log_mcp_call_with_policy(
        Arc::clone(&db),
        &endpoint.security_rules,
        &request,
        &response,
        &process_name,
        duration_ms,
        McpCallPolicyFields::from(&final_decision),
    )
    .await;
    Some(LoggedMcpResponse {
        response,
        event_id: emission.event_id,
    })
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
                mcp_security_event_from_summary(runtime_event_type, &summary, &process_name, None),
            );

            ::metrics::counter!(
                metrics::PARSER_EVENTS_TOTAL,
                "parser" => "mcp_json_rpc",
                "kind" => summary.kind.label(),
            )
            .increment(1);

            if disposition == StreamDisposition::Notification {
                // A notification is still dispatched to the aggregator when
                // its method is a request-type method such as tools/call, so
                // it takes the same in-flight permit a request does; without
                // one a guest fired unbounded concurrent tool calls by
                // omitting the id.
                let permit = if request_decision.is_allowed() {
                    match Arc::clone(&endpoint.inflight).acquire_owned().await {
                        Ok(permit) => Some(permit),
                        Err(_) => {
                            warn!("framed MCP inflight semaphore closed");
                            continue;
                        }
                    }
                } else {
                    None
                };
                let endpoint_h = Arc::clone(&endpoint);
                let db_h = Arc::clone(&db);
                let process_name_h = process_name.clone();
                let request_decision_h = request_decision.clone();
                let request_h = request.clone();
                tokio::spawn(async move {
                    let _permit = permit;
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
                        db_h,
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
                    Arc::clone(&db),
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
                    db_h,
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
    InvalidFrame { stream_id: Option<u32>, error: String },
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpMethodSummary {
    kind: McpMethodKind,
    method: String,
    request_id: Option<String>,
    server_name: Option<String>,
    tool_name: Option<String>,
    request_preview: Option<String>,
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

#[derive(Debug, Clone, Default)]
struct LoggedMcpEmission {
    event_id: Option<String>,
}

async fn log_mcp_call_with_policy(
    db: Arc<DbWriter>,
    security_rules: &Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
    policy_fields: McpCallPolicyFields,
) -> LoggedMcpEmission {
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
    // Serialized once each: the preview is the bytes, and its length is
    // the byte count (`to_string` and `to_vec` produce the same JSON).
    let request_preview = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_string(params).ok());
    let response_preview = resp
        .result
        .as_ref()
        .and_then(|result| serde_json::to_string(result).ok());
    let bytes_sent = request_preview.as_ref().map_or(0, |preview| preview.len() as u64);
    let bytes_received = response_preview.as_ref().map_or(0, |preview| preview.len() as u64);

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
        transport: "vsock_frame".to_string(),
        policy_mode: policy_fields.policy_mode,
        policy_action: policy_fields.policy_action,
        policy_rule: policy_fields.policy_rule,
        policy_reason: policy_fields.policy_reason,
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    };
    let security_event = security_event_from_mcp_call(&call);
    if let Some(event_id) = emit_security_write(&db, WriteOp::McpCall(call)).await {
        // The call row is accepted before the reply is sent (above); the
        // rule-ledger rows derived from it are written on their own task
        // so the client is not held while a third rule pass runs.
        let rules = security_rules.read().unwrap().clone();
        delegate_matching_security_rules_for_evaluated_event(
            Arc::clone(&db),
            event_id.clone(),
            runtime_mcp_event_type(&req.method),
            rules,
            Arc::new(BTreeMap::new()),
            security_event,
            current_unix_ms(),
            "framed MCP call",
        );
        return LoggedMcpEmission {
            event_id: Some(event_id.as_str().to_string()),
        };
    }
    LoggedMcpEmission::default()
}

fn security_event_from_mcp_call(call: &McpCall) -> SecurityEvent {
    let mut mcp = McpSecurityEvent {
        method: Some(call.method.clone()),
        server_name: Some(call.server_name.clone()),
        tool_call_name: call.tool_name.clone(),
        tool_list: if call.method == "tools/list" {
            call.response_preview.clone()
        } else {
            None
        },
        ..Default::default()
    }
    .with_request_preview(call.request_preview.as_deref())
    .with_response_preview(call.response_preview.as_deref())
    .with_error_message(call.error_message.as_deref());
    ensure_mcp_request_identity(&mut mcp, call.request_id.clone(), Some(call.method.clone()));
    let security_event = SecurityEvent::new(RuntimeSecurityEventType::McpToolCall)
        .with_mcp(mcp)
        .with_process(ProcessSecurityEvent {
            name: call.process_name.clone(),
            ..Default::default()
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
    let response_preview = response.and_then(response_content);
    let error_message = response
        .and_then(|response| response.error.as_ref())
        .map(|error| error.message.as_str());
    let tool_list = if summary.kind == McpMethodKind::ToolsList {
        response_preview.clone()
    } else {
        None
    };
    let mut mcp = McpSecurityEvent {
        method: Some(summary.method.clone()),
        server_name: summary.server_name.clone().or_else(|| Some(process_name.to_string())),
        tool_call_name: summary.tool_name.clone(),
        tool_list,
        ..Default::default()
    }
    .with_request_preview(summary.request_preview.as_deref())
    .with_response_preview(response_preview.as_deref())
    .with_error_message(error_message);
    ensure_mcp_request_identity(&mut mcp, summary.request_id.clone(), Some(summary.method.clone()));
    let event = SecurityEvent::new(event_type)
        .with_mcp(mcp)
        .with_process(ProcessSecurityEvent {
            name: Some(process_name.to_string()),
            ..Default::default()
        });
    match capsem_foundation::telemetry::ambient_capsem_trace_id() {
        Some(trace_id) => event.with_trace_id(trace_id),
        None => event,
    }
}

fn ensure_mcp_request_identity(mcp: &mut McpSecurityEvent, request_id: Option<String>, method: Option<String>) {
    if request_id.is_none() && method.is_none() {
        return;
    }
    let request = mcp.request.get_or_insert_with(Default::default);
    if request.id.is_none() {
        request.id = request_id;
    }
    if request.method.is_none() {
        request.method = method;
    }
}

fn evaluate_mcp_security_event(endpoint: &McpEndpointState, event: SecurityEvent) -> SecurityEnforcementDecision {
    let rules = endpoint.security_rules.read().unwrap().clone();
    let plugin_policy = snapshot_plugin_policy(&endpoint.plugin_policy);
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
    JsonRpcResponse::err(id, -32600, format!("MCP {subject} blocked by security rule: {rule}"))
}

fn json_rpc_id_to_log_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

#[cfg(test)]
mod tests;
