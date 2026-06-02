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
    ResolvedSecurityEvent, SecurityAction, SecurityEvent, SecurityResult,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, warn};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::metrics;
use super::{McpEndpointState, RuntimeSecurityEngine as _};
use crate::mcp::policy::{
    McpDecisionRule, McpDecisionRuleAction, McpDecisionRuleMatch, McpPolicy, ToolDecision,
};
use crate::mcp::types::{parse_namespaced, parse_resource_uri, JsonRpcRequest, JsonRpcResponse};

const MCP_JSON_RPC_MAX_BYTES: usize =
    capsem_proto::MCP_FRAME_MAX_SIZE - capsem_proto::MCP_FRAME_HEADER_LEN as usize;
const MCP_REQUEST_PREVIEW_BYTES: usize = 4096;
const TRANSPORT_ECHO_METHOD: &str = "capsem.transport/echo";

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
        let mut batch = Vec::with_capacity(64);
        while let Some(out) = rx.recv().await {
            batch.push(out);
            while batch.len() < 64 {
                match rx.try_recv() {
                    Ok(out) => batch.push(out),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            let started = Instant::now();
            if let Err(e) = write_frame_batch(&mut writer, &batch).await {
                for out in batch.drain(..) {
                    record_mcp_stage_labels(
                        "response_write",
                        out.method_kind,
                        out.tool_kind,
                        "error",
                        started,
                    );
                }
                debug!(error = %e, "framed MCP writer failed");
                break;
            }
            for out in batch.drain(..) {
                record_mcp_stage_labels(
                    "response_write",
                    out.method_kind,
                    out.tool_kind,
                    "ok",
                    started,
                );
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

            let parse_started = Instant::now();
            let request = match parse_json_rpc_payload(frame.payload()) {
                Ok(req) => req,
                Err(e) => {
                    record_mcp_stage_labels(
                        "parse_json_rpc",
                        "unknown",
                        "unknown",
                        "error",
                        parse_started,
                    );
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

            if let Some(response) = transport_echo_response(&request) {
                record_mcp_stage_labels(
                    "parse_json_rpc",
                    "unknown",
                    "none",
                    "ok",
                    parse_started,
                );
                streams
                    .lock()
                    .expect("framed MCP stream tracker poisoned")
                    .complete(frame.stream_id);
                let send_started = Instant::now();
                send_response_with_labels(
                    &tx,
                    frame.stream_id,
                    &process_name,
                    &response,
                    "unknown",
                    "none",
                )
                .await?;
                record_mcp_stage_labels(
                    "response_enqueue",
                    "unknown",
                    "none",
                    "ok",
                    send_started,
                );
                continue;
            }

            let summary = interpret_mcp_method(&request);
            record_mcp_stage("parse_json_rpc", &summary, "ok", parse_started);
            record_method_metric(&summary);

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
            let method_kind = summary.kind.label();
            let tool_kind = mcp_tool_kind_from_summary(&summary);
            tokio::spawn(async move {
                let _permit = permit;
                let decision_request =
                    McpDecisionRequest::from_request(&process_name, &request, &summary);
                let policy = endpoint_h.policy.read().await.clone();
                let decision_provider = LocalMcpDecisionProvider::enforce_arc(Arc::clone(&policy));
                let mut request_decision = decision_provider.decide(&decision_request);
                let mut runtime_block_event = None;
                if endpoint_h.security_engine.has_engine() {
                    let runtime_event_started = Instant::now();
                    let runtime_event = build_mcp_security_event_from_request(
                        &process_name,
                        &request,
                        &summary,
                        crate::telemetry::ambient_capsem_trace_id(),
                        SystemTime::now(),
                    );
                    record_mcp_stage_labels(
                        "runtime_security_project",
                        method_kind,
                        tool_kind,
                        "ok",
                        runtime_event_started,
                    );
                    let runtime_eval_started = Instant::now();
                    match endpoint_h.security_engine.evaluate(runtime_event) {
                        Ok(runtime_result) => {
                            let allows_dispatch =
                                mcp_security_result_allows_dispatch(&runtime_result);
                            record_mcp_stage_labels(
                                "runtime_security_evaluate",
                                method_kind,
                                tool_kind,
                                if allows_dispatch { "ok" } else { "block" },
                                runtime_eval_started,
                            );
                            if !allows_dispatch {
                                request_decision = mcp_policy_decision_from_security_result(
                                    &runtime_result,
                                    "mcp.runtime.blocked",
                                );
                                runtime_block_event = Some(runtime_result.resolved_event);
                            }
                        }
                        Err(error) => {
                            record_mcp_stage_labels(
                                "runtime_security_evaluate",
                                method_kind,
                                tool_kind,
                                "error",
                                runtime_eval_started,
                            );
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

                let mut dispatch_request = request.clone();
                let response_decision_request =
                    if request_decision.action == McpEnforcementAction::Rewrite {
                        match rewrite_mcp_request(dispatch_request, &request_decision) {
                            Ok(rewritten) => {
                                dispatch_request = rewritten;
                                McpDecisionRequest::from_request(
                                    &process_name,
                                    &dispatch_request,
                                    &summary,
                                )
                            }
                            Err(error) => {
                                let failed_decision = McpEnforcementDecision {
                                    reason: error,
                                    ..request_decision.clone()
                                };
                                let response = policy_blocked_response(
                                    request.id.clone(),
                                    "request rewrite",
                                    &failed_decision,
                                );
                                log_mcp_call_with_policy(
                                    &db_h,
                                    &policy_safe_request_for_rewrite_error(&request),
                                    &response,
                                    &process_name,
                                    0,
                                    McpCallEnforcementFields::from(&failed_decision),
                                    None,
                                )
                                .await;
                                streams_h
                                    .lock()
                                    .expect("framed MCP stream tracker poisoned")
                                    .complete(frame.stream_id);
                                let send_started = Instant::now();
                                if let Err(e) = send_response_with_labels(
                                    &tx_h,
                                    frame.stream_id,
                                    &process_name,
                                    &response,
                                    method_kind,
                                    tool_kind,
                                )
                                .await
                                {
                                    record_mcp_stage_labels(
                                        "response_enqueue",
                                        method_kind,
                                        tool_kind,
                                        "error",
                                        send_started,
                                    );
                                    debug!(error = %e, "framed MCP response dropped");
                                } else {
                                    record_mcp_stage_labels(
                                        "response_enqueue",
                                        method_kind,
                                        tool_kind,
                                        mcp_response_result(&response),
                                        send_started,
                                    );
                                }
                                return;
                            }
                        }
                    } else {
                        decision_request.clone()
                    };

                if request_decision.action.blocks_dispatch()
                    && request_decision.action != McpEnforcementAction::Rewrite
                {
                    let response =
                        policy_blocked_response(request.id.clone(), "request", &request_decision);
                    let log_request = policy_safe_request_for_pre_dispatch_denial(
                        &dispatch_request,
                        &request_decision,
                    );
                    log_mcp_call_with_policy(
                        &db_h,
                        log_request.as_ref(),
                        &response,
                        &process_name,
                        0,
                        McpCallEnforcementFields::from(&request_decision),
                        runtime_block_event,
                    )
                    .await;
                    streams_h
                        .lock()
                        .expect("framed MCP stream tracker poisoned")
                        .complete(frame.stream_id);
                    let send_started = Instant::now();
                    if let Err(e) = send_response_with_labels(
                        &tx_h,
                        frame.stream_id,
                        &process_name,
                        &response,
                        method_kind,
                        tool_kind,
                    )
                    .await
                    {
                        record_mcp_stage_labels(
                            "response_enqueue",
                            method_kind,
                            tool_kind,
                            "error",
                            send_started,
                        );
                        debug!(error = %e, "framed MCP response dropped");
                    } else {
                        record_mcp_stage_labels(
                            "response_enqueue",
                            method_kind,
                            tool_kind,
                            mcp_response_result(&response),
                            send_started,
                        );
                    }
                    return;
                }

                let start = Instant::now();
                let response = endpoint_h.handle_request(&dispatch_request).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                record_mcp_stage_labels(
                    "endpoint_dispatch",
                    method_kind,
                    tool_kind,
                    mcp_optional_response_result(response.as_ref()),
                    start,
                );
                streams_h
                    .lock()
                    .expect("framed MCP stream tracker poisoned")
                    .complete(frame.stream_id);
                let Some(response) = response else {
                    return;
                };
                let final_decision = decision_provider.decide_response(
                    &response_decision_request,
                    &response,
                    request_decision,
                );
                let response = match final_decision.action {
                    McpEnforcementAction::Ask | McpEnforcementAction::Block => {
                        policy_blocked_response(
                            dispatch_request.id.clone(),
                            "response",
                            &final_decision,
                        )
                    }
                    McpEnforcementAction::Rewrite
                        if final_decision
                            .rewrite_target
                            .as_deref()
                            .is_some_and(|target| target.trim_start().starts_with("response.")) =>
                    {
                        rewrite_mcp_response(response, &final_decision).unwrap_or_else(|error| {
                            policy_blocked_response(
                                dispatch_request.id.clone(),
                                "response rewrite",
                                &McpEnforcementDecision {
                                    reason: error,
                                    ..final_decision.clone()
                                },
                            )
                        })
                    }
                    McpEnforcementAction::Rewrite => response,
                    McpEnforcementAction::Allow => response,
                };
                let policy_fields = McpCallEnforcementFields::from(&final_decision);
                let send_started = Instant::now();
                let send_result = send_response_with_labels(
                    &tx_h,
                    frame.stream_id,
                    &process_name,
                    &response,
                    method_kind,
                    tool_kind,
                )
                .await;
                if let Err(e) = send_result
                {
                    record_mcp_stage_labels(
                        "response_enqueue",
                        method_kind,
                        tool_kind,
                        "error",
                        send_started,
                    );
                    debug!(error = %e, "framed MCP response dropped");
                } else {
                    record_mcp_stage_labels(
                        "response_enqueue",
                        method_kind,
                        tool_kind,
                        mcp_response_result(&response),
                        send_started,
                    );
                }
                drop(_permit);
                log_mcp_call_with_policy(
                    &db_h,
                    &dispatch_request,
                    &response,
                    &process_name,
                    duration_ms,
                    policy_fields,
                    None,
                )
                .await;
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
    Frame(InboundFrame),
    InvalidFrame {
        stream_id: Option<u32>,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InboundFrame {
    stream_id: u32,
    flags: u16,
    process_name: String,
    body: Vec<u8>,
    payload_start: usize,
}

impl InboundFrame {
    fn is_notification(&self) -> bool {
        self.stream_id == 0 && self.flags & capsem_proto::MCP_FRAME_FLAG_NOTIFICATION != 0
    }

    fn payload(&self) -> &[u8] {
        &self.body[self.payload_start..]
    }
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

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct McpDecisionRequest {
    process_name: String,
    method: String,
    method_kind: String,
    server_name: Option<String>,
    tool_name: Option<String>,
    resource_uri: Option<String>,
    prompt_name: Option<String>,
    arguments: Option<serde_json::Value>,
    request_preview: Option<String>,
    request_hash: String,
}

impl McpDecisionRequest {
    fn from_summary(process_name: &str, summary: &McpMethodSummary) -> Self {
        Self {
            process_name: process_name.to_string(),
            method: summary.method.clone(),
            method_kind: summary.kind.label().to_string(),
            server_name: summary.server_name.clone(),
            tool_name: summary.tool_name.clone(),
            resource_uri: summary.resource_uri.clone(),
            prompt_name: summary.prompt_name.clone(),
            arguments: None,
            request_preview: summary.request_preview.clone(),
            request_hash: summary.request_hash.clone(),
        }
    }

    fn from_request(process_name: &str, req: &JsonRpcRequest, summary: &McpMethodSummary) -> Self {
        let mut request = Self::from_summary(process_name, summary);
        request.arguments = match summary.kind {
            McpMethodKind::ToolsCall | McpMethodKind::PromptsGet => req
                .params
                .as_ref()
                .and_then(|params| params.get("arguments"))
                .cloned(),
            _ => None,
        };
        request
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
    build_network_mcp_security_event(
        &mcp_security_input_from_summary(req, summary, None, None, None),
        trace_id,
        timestamp,
    )
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

async fn log_mcp_call_with_policy(
    db: &DbWriter,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
    policy_fields: McpCallEnforcementFields,
    resolved_event: Option<ResolvedSecurityEvent>,
) {
    let started = Instant::now();
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
    let bytes_sent = request_preview
        .as_ref()
        .map(|preview| preview.len() as u64)
        .unwrap_or(0);
    let bytes_received = response_preview
        .as_ref()
        .map(|preview| preview.len() as u64)
        .unwrap_or(0);

    let timestamp = SystemTime::now();
    let trace_id = crate::telemetry::ambient_capsem_trace_id();
    let mcp_call = WriteOp::McpCall(McpCall {
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
    });
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
    db.write_many([mcp_call, WriteOp::ResolvedSecurityEvent(resolved_event)])
        .await;
    record_mcp_stage_labels(
        "telemetry_enqueue",
        mcp_method_kind_label(&req.method),
        mcp_tool_kind_from_name(tool_name),
        mcp_response_result(resp),
        started,
    );
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

#[derive(Clone)]
struct LocalMcpDecisionProvider {
    policy: Arc<McpPolicy>,
    mode: McpPolicyMode,
}

impl LocalMcpDecisionProvider {
    #[cfg(test)]
    fn audit_only(policy: McpPolicy) -> Self {
        Self::audit_only_arc(Arc::new(policy))
    }

    fn audit_only_arc(policy: Arc<McpPolicy>) -> Self {
        Self {
            policy,
            mode: McpPolicyMode::AuditOnly,
        }
    }

    fn enforce_arc(policy: Arc<McpPolicy>) -> Self {
        Self {
            policy,
            mode: McpPolicyMode::Enforce,
        }
    }

    fn decide(&self, request: &McpDecisionRequest) -> McpEnforcementDecision {
        if let Some(rule) = self.matching_request_rule(request) {
            let decision = self.decision_from_audit_rule(rule);
            if decision.action.blocks_dispatch() {
                return decision;
            }
            return decision;
        }

        match request.method_kind.as_str() {
            "tools/call" => self.decide_tool_call(request),
            "resources/read" => self.decide_server_method(request, "resource"),
            "prompts/get" => self.decide_server_method(request, "prompt"),
            _ => self.allow(
                format!("mcp.method.{}", request.method_kind.replace('/', "_")),
                format!(
                    "audit-only local policy allows method {} for dispatcher handling",
                    request.method
                ),
            ),
        }
    }

    fn decide_response(
        &self,
        request: &McpDecisionRequest,
        response: &JsonRpcResponse,
        base: McpEnforcementDecision,
    ) -> McpEnforcementDecision {
        if matches!(
            base.action,
            McpEnforcementAction::Ask | McpEnforcementAction::Block
        ) {
            return base;
        }
        if let Some(rule) = self.matching_response_rule(request, response) {
            let decision = self.decision_from_audit_rule(rule);
            if decision.action.blocks_dispatch() {
                return decision;
            }
        }
        base
    }

    fn decide_tool_call(&self, request: &McpDecisionRequest) -> McpEnforcementDecision {
        let Some(tool_name) = request.tool_name.as_deref().filter(|name| !name.is_empty()) else {
            return self.block(
                "mcp.method.tools_call.invalid".to_string(),
                "audit-only local policy denies tools/call without a tool name".to_string(),
            );
        };
        let Some(server_name) = request
            .server_name
            .as_deref()
            .filter(|server| !server.is_empty())
        else {
            return self.block(
                format!("mcp.tool.{tool_name}"),
                format!("audit-only local policy denies unnamespaced tool {tool_name}"),
            );
        };

        self.decision_from_tool(
            self.policy.evaluate(server_name, Some(tool_name)),
            format!("mcp.tool.{tool_name}"),
            format!("tools/call {tool_name}"),
        )
    }

    fn decide_server_method(
        &self,
        request: &McpDecisionRequest,
        method_subject: &str,
    ) -> McpEnforcementDecision {
        let Some(server_name) = request
            .server_name
            .as_deref()
            .filter(|server| !server.is_empty())
        else {
            return self.block(
                format!("mcp.{method_subject}.invalid"),
                format!(
                    "audit-only local policy denies {} without a namespaced server",
                    request.method
                ),
            );
        };

        self.decision_from_tool(
            self.policy.evaluate(server_name, None),
            format!("mcp.{method_subject}.{server_name}"),
            format!("{} on server {server_name}", request.method),
        )
    }

    fn decision_from_tool(
        &self,
        decision: ToolDecision,
        rule: String,
        subject: String,
    ) -> McpEnforcementDecision {
        match decision {
            ToolDecision::Block => {
                self.block(rule, format!("audit-only local policy block for {subject}"))
            }
            ToolDecision::Warn => self.allow(
                rule,
                format!("audit-only local policy warn for {subject}; v1 action remains allow"),
            ),
            ToolDecision::Allow => {
                self.allow(rule, format!("audit-only local policy allow for {subject}"))
            }
        }
    }

    fn matching_request_rule(&self, request: &McpDecisionRequest) -> Option<&McpDecisionRule> {
        select_rule(
            self.policy
                .audit_rules
                .iter()
                .filter(|rule| rule_matches_request(rule, request)),
        )
    }

    fn matching_response_rule(
        &self,
        request: &McpDecisionRequest,
        response: &JsonRpcResponse,
    ) -> Option<&McpDecisionRule> {
        select_rule(
            self.policy
                .audit_rules
                .iter()
                .filter(|rule| rule_matches_response(rule, request, response)),
        )
    }

    fn decision_from_audit_rule(&self, rule: &McpDecisionRule) -> McpEnforcementDecision {
        match rule.action {
            McpDecisionRuleAction::Allow => self.allow(rule_name(rule), rule_reason(rule)),
            McpDecisionRuleAction::Deny => self.block(rule_name(rule), rule_reason(rule)),
            McpDecisionRuleAction::Rewrite => self.rewrite(
                rule_name(rule),
                rule_reason(rule),
                rule.rewrite_target.clone(),
                rule.rewrite_value.clone(),
            ),
        }
    }

    fn allow(&self, rule: String, reason: String) -> McpEnforcementDecision {
        McpEnforcementDecision {
            mode: self.mode,
            action: McpEnforcementAction::Allow,
            rule,
            reason,
            rewrite_target: None,
            rewrite_value: None,
            policy_rule_name: None,
        }
    }

    fn ask(&self, rule: String, reason: String) -> McpEnforcementDecision {
        McpEnforcementDecision {
            mode: self.mode,
            action: McpEnforcementAction::Ask,
            rule,
            reason,
            rewrite_target: None,
            rewrite_value: None,
            policy_rule_name: None,
        }
    }

    fn block(&self, rule: String, reason: String) -> McpEnforcementDecision {
        McpEnforcementDecision {
            mode: self.mode,
            action: McpEnforcementAction::Block,
            rule,
            reason,
            rewrite_target: None,
            rewrite_value: None,
            policy_rule_name: None,
        }
    }

    fn rewrite(
        &self,
        rule: String,
        reason: String,
        rewrite_target: Option<String>,
        rewrite_value: Option<String>,
    ) -> McpEnforcementDecision {
        McpEnforcementDecision {
            mode: self.mode,
            action: McpEnforcementAction::Rewrite,
            rule,
            reason,
            rewrite_target,
            rewrite_value,
            policy_rule_name: None,
        }
    }
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

fn policy_safe_request_for_rewrite_error(request: &JsonRpcRequest) -> JsonRpcRequest {
    policy_request_with_redacted_arguments(request)
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

fn rewrite_mcp_request(
    mut request: JsonRpcRequest,
    decision: &McpEnforcementDecision,
) -> Result<JsonRpcRequest, String> {
    let target = decision
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    let replacement = decision
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    let Some(arguments) = request
        .params
        .as_mut()
        .and_then(|params| params.get_mut("arguments"))
    else {
        return Ok(request);
    };

    match field.as_str() {
        "arguments" => rewrite_json_strings(arguments, &regex, replacement),
        field => {
            let Some(path) = field.strip_prefix("arguments.") else {
                return Err(format!(
                    "unsupported MCP request rewrite target field '{field}'"
                ));
            };
            rewrite_json_path(arguments, path, &regex, replacement);
        }
    }

    Ok(request)
}

fn rewrite_mcp_response(
    mut response: JsonRpcResponse,
    decision: &McpEnforcementDecision,
) -> Result<JsonRpcResponse, String> {
    let target = decision
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    let replacement = decision
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    let Some(result) = response.result.as_mut() else {
        return Ok(response);
    };

    match field.as_str() {
        "response.content" | "response.text" => rewrite_json_strings(result, &regex, replacement),
        field => {
            let Some(path) = field.strip_prefix("response.") else {
                return Err(format!(
                    "unsupported MCP response rewrite target field '{field}'"
                ));
            };
            rewrite_json_path(result, path, &regex, replacement);
        }
    }

    Ok(response)
}

fn parse_regex_rewrite_target(target: &str) -> Result<(String, regex::Regex), String> {
    let Some((field, regex_text)) = target.split_once("=~") else {
        return Err("rewrite_target must use '<field> =~ <regex>'".into());
    };
    let field = field.trim();
    if field.is_empty() {
        return Err("rewrite_target field must not be empty".into());
    }
    let regex_text = regex_text.trim();
    if regex_text.len() < 2 {
        return Err("rewrite_target regex must be quoted".into());
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        return Err("rewrite_target regex must be quoted".into());
    }
    let Some(end) = regex_text[1..].rfind(quote) else {
        return Err("rewrite_target regex is missing a closing quote".into());
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        return Err("rewrite_target regex has trailing content after closing quote".into());
    }
    let pattern = &regex_text[1..=end];
    let regex = regex::Regex::new(pattern)
        .map_err(|error| format!("invalid rewrite_target regex: {error}"))?;
    Ok((field.to_string(), regex))
}

fn rewrite_json_strings(value: &mut serde_json::Value, regex: &regex::Regex, replacement: &str) {
    match value {
        serde_json::Value::String(text) => {
            *text = regex.replace_all(text, replacement).to_string();
        }
        serde_json::Value::Array(items) => {
            for item in items {
                rewrite_json_strings(item, regex, replacement);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values_mut() {
                rewrite_json_strings(value, regex, replacement);
            }
        }
        _ => {}
    }
}

fn rewrite_json_path(
    value: &mut serde_json::Value,
    path: &str,
    regex: &regex::Regex,
    replacement: &str,
) {
    let mut current = value;
    for segment in path.split('.') {
        let Some(next) = current.get_mut(segment) else {
            return;
        };
        current = next;
    }
    rewrite_json_strings(current, regex, replacement);
}

fn select_rule<'a, I>(rules: I) -> Option<&'a McpDecisionRule>
where
    I: IntoIterator<Item = &'a McpDecisionRule>,
{
    let mut first_allow = None;
    for rule in rules {
        match rule.action {
            McpDecisionRuleAction::Deny | McpDecisionRuleAction::Rewrite => return Some(rule),
            McpDecisionRuleAction::Allow => first_allow.get_or_insert(rule),
        };
    }
    first_allow
}

fn rule_matches_request(rule: &McpDecisionRule, request: &McpDecisionRequest) -> bool {
    match &rule.matches {
        McpDecisionRuleMatch::ToolName { name } => request.tool_name.as_deref() == Some(name),
        McpDecisionRuleMatch::ResourceUri { uri } => request.resource_uri.as_deref() == Some(uri),
        McpDecisionRuleMatch::ArgumentName { method, name } => {
            method_matches(method.as_deref(), request)
                && request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.as_object())
                    .is_some_and(|args| args.contains_key(name))
        }
        McpDecisionRuleMatch::ArgumentValue {
            method,
            name,
            equals,
        } => {
            method_matches(method.as_deref(), request)
                && request.arguments.as_ref().and_then(|args| args.get(name)) == Some(equals)
        }
        McpDecisionRuleMatch::ReturnValue { .. } => false,
        McpDecisionRuleMatch::Condition {
            callback,
            condition,
        } => callback == "mcp.request" && mcp_condition_matches_request(condition, request),
    }
}

fn rule_matches_response(
    rule: &McpDecisionRule,
    request: &McpDecisionRequest,
    response: &JsonRpcResponse,
) -> bool {
    match &rule.matches {
        McpDecisionRuleMatch::ReturnValue {
            method,
            path,
            equals,
        } => {
            method_matches(method.as_deref(), request)
                && response
                    .result
                    .as_ref()
                    .and_then(|result| json_path(result, path))
                    == Some(equals)
        }
        McpDecisionRuleMatch::Condition {
            callback,
            condition,
        } => {
            callback == "mcp.response"
                && mcp_condition_matches_request(condition, request)
                && mcp_condition_matches_response(condition, response)
        }
        _ => false,
    }
}

fn mcp_condition_matches_request(condition: &str, request: &McpDecisionRequest) -> bool {
    condition
        .split("&&")
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .all(|term| mcp_request_condition_term_matches(term, request))
}

fn mcp_condition_matches_response(condition: &str, response: &JsonRpcResponse) -> bool {
    condition
        .split("&&")
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .all(|term| {
            if term.starts_with("response.") {
                mcp_response_condition_term_matches(term, response)
            } else {
                true
            }
        })
}

fn mcp_request_condition_term_matches(term: &str, request: &McpDecisionRequest) -> bool {
    if term == "true" || term.starts_with("response.") {
        return true;
    }
    if let Some(expected) = quoted_equality_rhs(term, "method") {
        return request.method == expected;
    }
    if let Some(expected) = quoted_equality_rhs(term, "tool.name") {
        return request.tool_name.as_deref() == Some(expected);
    }
    if let Some(path) = term
        .strip_prefix("has(")
        .and_then(|value| value.strip_suffix(')'))
        .and_then(|value| value.trim().strip_prefix("arguments."))
    {
        return request
            .arguments
            .as_ref()
            .is_some_and(|arguments| json_path(arguments, path).is_some());
    }
    if let Some((path, expected)) = quoted_path_equality_rhs(term, "arguments.") {
        return request
            .arguments
            .as_ref()
            .and_then(|arguments| json_path(arguments, path))
            == Some(&serde_json::Value::String(expected.to_string()));
    }
    if let Some((path, needle)) = quoted_contains_rhs(term, "arguments.") {
        return request
            .arguments
            .as_ref()
            .and_then(|arguments| json_path(arguments, path))
            .is_some_and(|value| json_value_contains_text(value, needle));
    }
    false
}

fn mcp_response_condition_term_matches(term: &str, response: &JsonRpcResponse) -> bool {
    if let Some((path, needle)) = quoted_contains_rhs(term, "response.") {
        let Some(result) = response.result.as_ref() else {
            return false;
        };
        if path == "text" || path == "content" {
            return json_value_contains_text(result, needle);
        }
        return json_path(result, path)
            .is_some_and(|value| json_value_contains_text(value, needle));
    }
    false
}

fn quoted_equality_rhs<'a>(term: &'a str, lhs: &str) -> Option<&'a str> {
    let (left, right) = term.split_once("==")?;
    if left.trim() != lhs {
        return None;
    }
    unquote(right.trim())
}

fn quoted_path_equality_rhs<'a>(term: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let (left, right) = term.split_once("==")?;
    let path = left.trim().strip_prefix(prefix)?;
    let expected = unquote(right.trim())?;
    Some((path, expected))
}

fn quoted_contains_rhs<'a>(term: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let (left, right) = term.split_once(".contains(")?;
    let path = left.trim().strip_prefix(prefix)?;
    let needle = unquote(right.trim().strip_suffix(')')?.trim())?;
    Some((path, needle))
}

fn unquote(value: &str) -> Option<&str> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
}

fn json_value_contains_text(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains(needle),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_value_contains_text(value, needle)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|value| json_value_contains_text(value, needle)),
        _ => false,
    }
}

fn method_matches(method: Option<&str>, request: &McpDecisionRequest) -> bool {
    method.is_none_or(|method| method == request.method)
}

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn json_rpc_id_to_log_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

fn rule_name(rule: &McpDecisionRule) -> String {
    if rule.id.starts_with("policy.") {
        return rule.id.clone();
    }
    format!("mcp.rule.{}", rule.id)
}

fn rule_reason(rule: &McpDecisionRule) -> String {
    rule.reason
        .clone()
        .unwrap_or_else(|| format!("audit-only local enforcement rule {} matched", rule.id))
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
    method_kind: &'static str,
    tool_kind: &'static str,
}

async fn send_response(
    tx: &tokio::sync::mpsc::Sender<OutboundFrame>,
    stream_id: u32,
    process_name: &str,
    response: &JsonRpcResponse,
) -> Result<()> {
    send_response_with_labels(tx, stream_id, process_name, response, "unknown", "unknown").await
}

async fn send_response_with_labels(
    tx: &tokio::sync::mpsc::Sender<OutboundFrame>,
    stream_id: u32,
    process_name: &str,
    response: &JsonRpcResponse,
    method_kind: &'static str,
    tool_kind: &'static str,
) -> Result<()> {
    let payload = serde_json::to_vec(response).context("serialize framed MCP response")?;
    tx.send(OutboundFrame {
        stream_id,
        process_name: process_name.to_string(),
        payload,
        method_kind,
        tool_kind,
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
    match decode_inbound_frame(body) {
        Ok(frame) => Ok(FrameRead::Frame(frame)),
        Err(e) => Ok(FrameRead::InvalidFrame {
            stream_id: recover_stream_id(e.body()),
            error: e.to_string(),
        }),
    }
}

struct InboundFrameDecodeError {
    body: Vec<u8>,
    error: anyhow::Error,
}

impl InboundFrameDecodeError {
    fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Display for InboundFrameDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)
    }
}

fn decode_inbound_frame(
    body: Vec<u8>,
) -> std::result::Result<InboundFrame, InboundFrameDecodeError> {
    let frame = match capsem_proto::decode_mcp_frame_body_ref(&body) {
        Ok(frame) => frame,
        Err(error) => return Err(InboundFrameDecodeError { body, error }),
    };
    let payload_start = frame.payload.as_ptr() as usize - body.as_ptr() as usize;
    let stream_id = frame.stream_id;
    let flags = frame.flags;
    let process_name = frame.process_name.to_string();
    Ok(InboundFrame {
        stream_id,
        flags,
        process_name,
        body,
        payload_start,
    })
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

fn validate_frame_request_pair(frame: &InboundFrame, req: &JsonRpcRequest) -> Result<()> {
    match (frame.is_notification(), req.id.is_some()) {
        (true, false) => Ok(()),
        (true, true) => bail!("notification stream carried a JSON-RPC id"),
        (false, true) => Ok(()),
        (false, false) => bail!("request stream is missing a JSON-RPC id"),
    }
}

fn transport_echo_response(req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
    if req.method != TRANSPORT_ECHO_METHOD {
        return None;
    }

    let payload = req
        .params
        .as_ref()
        .and_then(|params| params.get("payload"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(JsonRpcResponse::ok(
        req.id.clone(),
        serde_json::json!({ "payload": payload }),
    ))
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

fn record_mcp_stage(
    stage: &'static str,
    summary: &McpMethodSummary,
    result: &'static str,
    started: Instant,
) {
    record_mcp_stage_labels(
        stage,
        summary.kind.label(),
        mcp_tool_kind_from_summary(summary),
        result,
        started,
    );
}

fn record_mcp_stage_labels(
    stage: &'static str,
    method_kind: &'static str,
    tool_kind: &'static str,
    result: &'static str,
    started: Instant,
) {
    ::metrics::histogram!(
        metrics::MCP_STAGE_DURATION_MS,
        "stage" => stage,
        "method_kind" => method_kind,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

fn mcp_method_kind_label(method: &str) -> &'static str {
    match method {
        "initialize" => "initialize",
        "notifications/initialized" => "notifications/initialized",
        "tools/list" => "tools/list",
        "tools/call" => "tools/call",
        "resources/list" => "resources/list",
        "resources/read" => "resources/read",
        "prompts/list" => "prompts/list",
        "prompts/get" => "prompts/get",
        _ => "unknown",
    }
}

fn mcp_tool_kind_from_summary(summary: &McpMethodSummary) -> &'static str {
    mcp_tool_kind_from_name(summary.tool_name.as_deref())
}

fn mcp_tool_kind_from_name(tool_name: Option<&str>) -> &'static str {
    match tool_name {
        Some("local__echo") => "local_echo",
        Some(name) if name.starts_with("local__snapshots_") => "local_snapshot",
        Some("local__fetch_http" | "local__grep_http" | "local__http_headers") => "local_http",
        Some(name) if name.starts_with("local__") => "local_other",
        Some(_) => "external",
        None => "none",
    }
}

fn mcp_optional_response_result(response: Option<&JsonRpcResponse>) -> &'static str {
    response.map_or("no_response", mcp_response_result)
}

fn mcp_response_result(response: &JsonRpcResponse) -> &'static str {
    if response.error.is_some() {
        "error"
    } else {
        "ok"
    }
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, out: &OutboundFrame) -> Result<()> {
    let bytes = capsem_proto::encode_mcp_frame(out.stream_id, 0, &out.process_name, &out.payload)?;
    writer.write_all(&bytes).await.context("write MCP frame")?;
    writer.flush().await.context("flush MCP frame")
}

async fn write_frame_batch<W: AsyncWrite + Unpin>(
    writer: &mut W,
    batch: &[OutboundFrame],
) -> Result<()> {
    if batch.len() == 1 {
        return write_frame(writer, &batch[0]).await;
    }

    let mut bytes = Vec::new();
    for out in batch {
        let frame =
            capsem_proto::encode_mcp_frame(out.stream_id, 0, &out.process_name, &out.payload)?;
        bytes.extend_from_slice(&frame);
    }
    writer
        .write_all(&bytes)
        .await
        .context("write MCP frame batch")?;
    writer.flush().await.context("flush MCP frame batch")
}

#[cfg(test)]
mod tests;
