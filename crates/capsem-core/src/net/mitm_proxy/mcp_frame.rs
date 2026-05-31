//! Framed MCP JSON-RPC over the MITM vsock port.
//!
//! Guest-originated MCP reaches the MITM endpoint as bounded JSON-RPC frames
//! on vsock:5002. The MITM owns parsing, policy decisions, dispatch through
//! the low-privilege aggregator, and `mcp_calls` telemetry.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use anyhow::{bail, Context, Result};
use capsem_logger::{DbWriter, Decision, McpCall, WriteOp};
use capsem_network_engine::mcp_security::{
    build_mcp_resolved_security_event as build_network_mcp_resolved_security_event,
    build_mcp_security_event as build_network_mcp_security_event,
    mcp_security_result_allows_dispatch as network_mcp_security_result_allows_dispatch,
    McpPolicyFields as NetworkMcpPolicyFields, McpSecurityEventInput,
};
use capsem_security_engine::{
    AiContentKind, LinkStatus, McpSecuritySubject, McpToolExecutionEvidence, ResolvedSecurityEvent,
    SecurityAction, SecurityEvent, SecurityEventSubject, SecurityEventType, SecurityResult,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, warn};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::metrics;
use super::{McpEndpointState, RuntimeSecurityEngine as _};
use crate::mcp::types::{parse_namespaced, parse_resource_uri, JsonRpcRequest, JsonRpcResponse};

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
            record_method_metric(&summary);
            let mut request_decision =
                allow_mcp_decision("mcp.runtime.default", "runtime security engine allowed MCP request");
            let mut runtime_block_event = None;
            if endpoint.security_engine.has_engine() {
                let runtime_event = build_mcp_security_event_from_request(
                    &process_name,
                    &request,
                    &summary,
                    crate::telemetry::ambient_capsem_trace_id(),
                    SystemTime::now(),
                );
                match endpoint.security_engine.evaluate(runtime_event) {
                    Ok(runtime_result) => {
                        if !mcp_security_result_allows_dispatch(&runtime_result) {
                            request_decision = mcp_policy_decision_from_security_result(
                                &runtime_result,
                                "mcp.runtime.blocked",
                            );
                            runtime_block_event = Some(runtime_result.resolved_event);
                        }
                    }
                    Err(error) => {
                        request_decision = McpEnforcementDecision {
                            mode: McpPolicyMode::Enforce,
                            action: McpEnforcementAction::Block,
                            rule: "mcp.runtime.error".into(),
                            reason: format!("security engine error: {error}"),
                            rewrite_target: None,
                            rewrite_value: None,
                            policy_rule_name: None,
                        };
                    }
                }
            }

            ::metrics::counter!(
                metrics::PARSER_EVENTS_TOTAL,
                "parser" => "mcp_json_rpc",
                "kind" => summary.kind.label(),
            )
            .increment(1);

            if disposition == StreamDisposition::Notification {
                if is_allowed_mcp_notification(&request) {
                    let endpoint_h = Arc::clone(&endpoint);
                    tokio::spawn(async move {
                        let _ = endpoint_h.handle_request(&request).await;
                    });
                } else {
                    let decision = disallowed_notification_decision(&request);
                    let response = policy_blocked_response(None, "notification", &decision);
                    let safe_request = policy_request_with_redacted_arguments(&request);
                    log_mcp_call_with_policy(
                        &db,
                        &safe_request,
                        &response,
                        &process_name,
                        0,
                        McpCallEnforcementFields::from(&decision),
                        None,
                    )
                    .await;
                }
                continue;
            }

            let dispatch_request = request.clone();

            if request_decision.action.blocks_dispatch() {
                let response =
                    policy_blocked_response(request.id.clone(), "request", &request_decision);
                let log_request =
                    policy_safe_request_for_pre_dispatch_denial(&dispatch_request, &request_decision);
                log_mcp_call_with_policy(
                    &db,
                    log_request.as_ref(),
                    &response,
                    &process_name,
                    0,
                    McpCallEnforcementFields::from(&request_decision),
                    runtime_block_event,
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
                let mut final_decision = request_decision;
                let mut runtime_response_event = None;
                if endpoint_h.security_engine.has_engine() {
                    let runtime_event = build_mcp_security_event_from_response(
                        &process_name,
                        &dispatch_request,
                        &response,
                        &summary,
                        duration_ms,
                        crate::telemetry::ambient_capsem_trace_id(),
                        SystemTime::now(),
                    );
                    match endpoint_h.security_engine.evaluate(runtime_event) {
                        Ok(runtime_result) => {
                            if !mcp_security_result_allows_dispatch(&runtime_result) {
                                final_decision = mcp_policy_decision_from_security_result(
                                    &runtime_result,
                                    "mcp.runtime.response_blocked",
                                );
                                runtime_response_event = Some(runtime_result.resolved_event);
                            }
                        }
                        Err(error) => {
                            final_decision = McpEnforcementDecision {
                                mode: McpPolicyMode::Enforce,
                                action: McpEnforcementAction::Block,
                                rule: "mcp.runtime.response_error".into(),
                                reason: format!("security engine error: {error}"),
                                rewrite_target: None,
                                rewrite_value: None,
                                policy_rule_name: None,
                            };
                        }
                    }
                }
                let response = match final_decision.action {
                    McpEnforcementAction::Ask | McpEnforcementAction::Block => {
                        policy_blocked_response(
                            dispatch_request.id.clone(),
                            "response",
                            &final_decision,
                        )
                    }
                    McpEnforcementAction::Rewrite => policy_blocked_response(
                        dispatch_request.id.clone(),
                        "response rewrite",
                        &final_decision,
                    ),
                    McpEnforcementAction::Allow => response,
                };
                let policy_fields = McpCallEnforcementFields::from(&final_decision);
                log_mcp_call_with_policy(
                    &db_h,
                    &dispatch_request,
                    &response,
                    &process_name,
                    duration_ms,
                    policy_fields,
                    runtime_response_event,
                )
                .await;
                if let Err(e) = send_response(&tx_h, frame.stream_id, &process_name, &response).await {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum McpPolicyMode {
    AuditOnly,
    Enforce,
}

impl McpPolicyMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::AuditOnly => "audit_only",
            Self::Enforce => "enforce",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum McpEnforcementAction {
    Allow,
    Ask,
    Block,
    Rewrite,
}

impl McpEnforcementAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
            Self::Rewrite => "rewrite",
        }
    }

    fn blocks_dispatch(self) -> bool {
        !matches!(self, Self::Allow)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct McpEnforcementDecision {
    mode: McpPolicyMode,
    action: McpEnforcementAction,
    rule: String,
    reason: String,
    rewrite_target: Option<String>,
    rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy_rule_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpCallEnforcementFields {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

impl From<&McpEnforcementDecision> for McpCallEnforcementFields {
    fn from(decision: &McpEnforcementDecision) -> Self {
        Self {
            policy_mode: Some(decision.mode.as_str().to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: Some(decision.rule.clone()),
            policy_reason: Some(decision.reason.clone()),
        }
    }
}

fn build_mcp_security_event_from_request(
    _process_name: &str,
    req: &JsonRpcRequest,
    summary: &McpMethodSummary,
    trace_id: Option<String>,
    timestamp: SystemTime,
) -> SecurityEvent {
    build_mcp_security_event_from_frame(
        SecurityEventType::McpRequest,
        req,
        None,
        summary,
        0,
        trace_id,
        timestamp,
    )
}

fn build_mcp_security_event_from_response(
    _process_name: &str,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    summary: &McpMethodSummary,
    duration_ms: u64,
    trace_id: Option<String>,
    timestamp: SystemTime,
) -> SecurityEvent {
    build_mcp_security_event_from_frame(
        SecurityEventType::McpResponse,
        req,
        Some(resp),
        summary,
        duration_ms,
        trace_id,
        timestamp,
    )
}

fn build_mcp_security_event_from_frame(
    event_type: SecurityEventType,
    req: &JsonRpcRequest,
    resp: Option<&JsonRpcResponse>,
    summary: &McpMethodSummary,
    duration_ms: u64,
    trace_id: Option<String>,
    timestamp: SystemTime,
) -> SecurityEvent {
    let mut event = build_network_mcp_security_event(
        &mcp_security_input_from_summary(req, summary, None, None, None),
        trace_id,
        timestamp,
    );
    event.common.event_type = event_type;
    event.subject = SecurityEventSubject::Mcp(McpSecuritySubject {
        method: Some(summary.method.clone()),
        server_id: summary
            .server_name
            .clone()
            .unwrap_or_else(|| "gateway".to_string()),
        tool_name: subject_tool_name_from_summary(summary),
        evidence: Some(Box::new(mcp_execution_evidence(
            req,
            resp,
            summary,
            duration_ms,
        ))),
    });
    event
}

fn mcp_execution_evidence(
    req: &JsonRpcRequest,
    resp: Option<&JsonRpcResponse>,
    summary: &McpMethodSummary,
    duration_ms: u64,
) -> McpToolExecutionEvidence {
    let request_arguments_json = request_arguments_json(req);
    let (result_kind, result_preview, result_json, is_error) = match resp {
        Some(resp) => mcp_response_result_fields(resp),
        None => (AiContentKind::Unknown, None, None, false),
    };
    McpToolExecutionEvidence {
        mcp_call_id: req
            .id
            .as_ref()
            .and_then(json_rpc_id_to_log_string)
            .unwrap_or_else(|| summary.request_hash.clone()),
        server_id: summary
            .server_name
            .clone()
            .unwrap_or_else(|| "gateway".to_string()),
        tool_name: subject_tool_name_from_summary(summary),
        namespaced_tool_name: summary
            .tool_name
            .clone()
            .unwrap_or_else(|| subject_tool_name_from_summary(summary)),
        transport: "vsock-framed".into(),
        request_arguments_raw: request_arguments_json.clone(),
        request_arguments_json,
        result_kind,
        result_preview,
        result_json,
        is_error,
        latency_ms: duration_ms,
        linked_model_interaction_id: None,
        linked_model_tool_call_id: None,
        link_status: LinkStatus::NotApplicable,
    }
}

fn request_arguments_json(req: &JsonRpcRequest) -> Option<String> {
    req.params
        .as_ref()
        .and_then(|params| params.get("arguments"))
        .and_then(|arguments| serde_json::to_string(arguments).ok())
}

fn mcp_response_result_fields(
    resp: &JsonRpcResponse,
) -> (AiContentKind, Option<String>, Option<String>, bool) {
    if let Some(result) = resp.result.as_ref() {
        let json = serde_json::to_string(result).ok();
        return (AiContentKind::Json, json.clone(), json, false);
    }
    if let Some(error) = resp.error.as_ref() {
        return (AiContentKind::Text, Some(error.message.clone()), None, true);
    }
    (AiContentKind::Unknown, None, None, false)
}

fn subject_tool_name_from_summary(summary: &McpMethodSummary) -> String {
    summary
        .tool_name
        .as_deref()
        .and_then(parse_namespaced)
        .map(|(_, tool)| tool.to_string())
        .or_else(|| summary.tool_name.clone())
        .or_else(|| summary.resource_uri.clone())
        .or_else(|| summary.prompt_name.clone())
        .unwrap_or_else(|| summary.method.clone())
}

fn mcp_security_input_from_summary(
    req: &JsonRpcRequest,
    summary: &McpMethodSummary,
    policy_fields: Option<NetworkMcpPolicyFields>,
    decision: Option<String>,
    response_error_message: Option<String>,
) -> McpSecurityEventInput {
    let server_name = summary
        .server_name
        .clone()
        .unwrap_or_else(|| "gateway".to_string());
    let subject_tool_name = summary
        .tool_name
        .as_deref()
        .and_then(parse_namespaced)
        .map(|(_, tool)| tool.to_string())
        .or_else(|| summary.tool_name.clone())
        .or_else(|| summary.resource_uri.clone())
        .or_else(|| summary.prompt_name.clone())
        .unwrap_or_else(|| summary.method.clone());
    McpSecurityEventInput {
        server_name,
        tool_name: subject_tool_name,
        request_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
        policy_fields: policy_fields.unwrap_or_default(),
        decision,
        response_error_message,
    }
}

fn mcp_security_result_allows_dispatch(result: &SecurityResult) -> bool {
    network_mcp_security_result_allows_dispatch(result)
}

fn mcp_policy_decision_from_security_result(
    result: &SecurityResult,
    fallback_rule: &str,
) -> McpEnforcementDecision {
    let action = match result.action {
        SecurityAction::Continue | SecurityAction::ObserveOnly => McpEnforcementAction::Allow,
        SecurityAction::Ask(_) => McpEnforcementAction::Ask,
        SecurityAction::Rewrite(_) => McpEnforcementAction::Block,
        SecurityAction::Block(_)
        | SecurityAction::Throttle(_)
        | SecurityAction::Quarantine(_)
        | SecurityAction::Restore(_)
        | SecurityAction::DropConnection(_)
        | SecurityAction::Error(_) => McpEnforcementAction::Block,
    };
    McpEnforcementDecision {
        mode: McpPolicyMode::Enforce,
        action,
        rule: mcp_security_result_rule_id(result).unwrap_or_else(|| fallback_rule.to_string()),
        reason: mcp_security_result_reason(result),
        rewrite_target: None,
        rewrite_value: None,
        policy_rule_name: None,
    }
}

fn mcp_security_result_rule_id(result: &SecurityResult) -> Option<String> {
    result
        .resolved_event
        .event
        .decision
        .as_ref()
        .and_then(|decision| decision.rule.clone())
        .or_else(|| match &result.action {
            SecurityAction::Block(block) => block.rule_id.clone(),
            _ => None,
        })
}

fn mcp_security_result_reason(result: &SecurityResult) -> String {
    result
        .resolved_event
        .event
        .decision
        .as_ref()
        .and_then(|decision| decision.reason.clone())
        .or_else(|| match &result.action {
            SecurityAction::Ask(plan) => Some(plan.reason_code.clone()),
            SecurityAction::Block(block) => Some(block.reason_code.clone()),
            SecurityAction::Throttle(plan) => Some(plan.reason_code.clone()),
            SecurityAction::Error(error) => Some(error.message.clone()),
            SecurityAction::DropConnection(reason) => Some(reason.reason_code.clone()),
            SecurityAction::Rewrite(patch) => Some(patch.replacement_ref.clone()),
            SecurityAction::Quarantine(plan) => Some(plan.quarantine_id.clone()),
            SecurityAction::Restore(plan) => Some(plan.reason_code.clone()),
            SecurityAction::Continue | SecurityAction::ObserveOnly => None,
        })
        .unwrap_or_else(|| "MCP request blocked by security engine".into())
}

fn allow_mcp_decision(rule: &str, reason: &str) -> McpEnforcementDecision {
    McpEnforcementDecision {
        mode: McpPolicyMode::Enforce,
        action: McpEnforcementAction::Allow,
        rule: rule.to_string(),
        reason: reason.to_string(),
        rewrite_target: None,
        rewrite_value: None,
        policy_rule_name: None,
    }
}

async fn log_mcp_call_with_policy(
    db: &DbWriter,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
    policy_fields: McpCallEnforcementFields,
    resolved_event: Option<ResolvedSecurityEvent>,
) {
    let tool_name = req
        .params
        .as_ref()
        .and_then(|params| params.get("name"))
        .and_then(|name| name.as_str());
    let server_name = match tool_name {
        Some(tool) => parse_namespaced(tool)
            .map(|(server, _)| server)
            .unwrap_or("gateway"),
        None => "gateway",
    };
    let decision = if resp.error.is_some() {
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

    let timestamp = SystemTime::now();
    let trace_id = crate::telemetry::ambient_capsem_trace_id();
    db.write(WriteOp::McpCall(McpCall {
        timestamp,
        server_name: server_name.to_string(),
        method: req.method.clone(),
        tool_name: tool_name.map(String::from),
        request_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
        request_preview,
        response_preview,
        decision: decision.to_string(),
        duration_ms,
        error_message: resp.error.as_ref().map(|error| error.message.clone()),
        process_name: Some(process_name.to_string()),
        bytes_sent,
        bytes_received,
        policy_mode: policy_fields.policy_mode.clone(),
        policy_action: policy_fields.policy_action.clone(),
        policy_rule: policy_fields.policy_rule.clone(),
        policy_reason: policy_fields.policy_reason.clone(),
        trace_id: trace_id.clone(),
    }))
    .await;
    let resolved_event = resolved_event.unwrap_or_else(|| {
        build_mcp_resolved_security_event(
            req,
            resp,
            server_name,
            tool_name,
            decision,
            &policy_fields,
            timestamp,
            trace_id,
        )
    });
    db.write(WriteOp::ResolvedSecurityEvent(resolved_event))
        .await;
}

#[allow(clippy::too_many_arguments)]
fn build_mcp_resolved_security_event(
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    server_name: &str,
    tool_name: Option<&str>,
    decision: &str,
    policy_fields: &McpCallEnforcementFields,
    timestamp: SystemTime,
    trace_id: Option<String>,
) -> ResolvedSecurityEvent {
    let subject_tool_name = tool_name
        .and_then(parse_namespaced)
        .map(|(_, tool)| tool.to_string())
        .or_else(|| tool_name.map(str::to_string))
        .unwrap_or_else(|| req.method.clone());
    let input = McpSecurityEventInput {
        server_name: server_name.to_string(),
        tool_name: subject_tool_name,
        request_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
        policy_fields: NetworkMcpPolicyFields {
            policy_action: policy_fields.policy_action.clone(),
            policy_rule: policy_fields.policy_rule.clone(),
            policy_reason: policy_fields.policy_reason.clone(),
        },
        decision: Some(decision.to_string()),
        response_error_message: resp.error.as_ref().map(|error| error.message.clone()),
    };
    build_network_mcp_resolved_security_event(&input, trace_id, timestamp)
}

fn policy_blocked_response(
    id: Option<serde_json::Value>,
    subject: &str,
    decision: &McpEnforcementDecision,
) -> JsonRpcResponse {
    JsonRpcResponse::err(
        id,
        -32600,
        format!("MCP {subject} blocked by policy: {}", decision.rule),
    )
}

fn is_allowed_mcp_notification(request: &JsonRpcRequest) -> bool {
    request.method == "notifications/initialized"
}

fn disallowed_notification_decision(request: &JsonRpcRequest) -> McpEnforcementDecision {
    McpEnforcementDecision {
        mode: McpPolicyMode::Enforce,
        action: McpEnforcementAction::Block,
        rule: "mcp.notification.disallowed".to_string(),
        reason: format!("MCP notification method {} is not allowed", request.method),
        rewrite_target: None,
        rewrite_value: None,
        policy_rule_name: None,
    }
}

fn policy_safe_request_for_pre_dispatch_denial<'a>(
    request: &'a JsonRpcRequest,
    decision: &McpEnforcementDecision,
) -> Cow<'a, JsonRpcRequest> {
    if decision.rule.starts_with("policy.mcp.") {
        Cow::Owned(policy_request_with_redacted_arguments(request))
    } else {
        Cow::Borrowed(request)
    }
}

fn policy_request_with_redacted_arguments(request: &JsonRpcRequest) -> JsonRpcRequest {
    let mut safe = request.clone();
    if let Some(serde_json::Value::Object(params)) = safe.params.as_mut() {
        if params.contains_key("arguments") {
            params.insert(
                "arguments".to_string(),
                serde_json::json!({ "redacted_by_policy": true }),
            );
        }
    }
    safe
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
