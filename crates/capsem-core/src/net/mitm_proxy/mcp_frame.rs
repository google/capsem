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
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, BlockResponse, Enforceability, McpSecuritySubject,
    RedactionState, ResolvedEventStep, ResolvedEventStepKind, ResolvedSecurityEvent,
    SecurityAction, SecurityDecision, SecurityDecisionAction, SecurityError, SecurityEvent,
    SecurityEventCommon, SourceEngine, StepStatus, RESOLVED_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, warn};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::metrics;
use super::McpEndpointState;
use crate::mcp::policy::{
    McpDecisionRule, McpDecisionRuleAction, McpDecisionRuleMatch, McpPolicy, ToolDecision,
};
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
            let decision_request =
                McpDecisionRequest::from_request(&process_name, &request, &summary);
            let policy = endpoint.policy.read().await.clone();
            let decision_provider = LocalMcpDecisionProvider::audit_only_arc(Arc::clone(&policy));
            let request_decision = decision_provider.decide(&decision_request);

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
                        McpCallPolicyFields::from(&decision),
                    )
                    .await;
                }
                continue;
            }

            let mut dispatch_request = request.clone();
            let response_decision_request = if request_decision.action == McpPolicyAction::Rewrite {
                match rewrite_mcp_request(dispatch_request, &request_decision) {
                    Ok(rewritten) => {
                        dispatch_request = rewritten;
                        McpDecisionRequest::from_request(&process_name, &dispatch_request, &summary)
                    }
                    Err(error) => {
                        let failed_decision = McpPolicyDecision {
                            reason: error,
                            ..request_decision.clone()
                        };
                        let response = policy_blocked_response(
                            request.id.clone(),
                            "request rewrite",
                            &failed_decision,
                        );
                        log_mcp_call_with_policy(
                            &db,
                            &policy_safe_request_for_rewrite_error(&request),
                            &response,
                            &process_name,
                            0,
                            McpCallPolicyFields::from(&failed_decision),
                        )
                        .await;
                        streams
                            .lock()
                            .expect("framed MCP stream tracker poisoned")
                            .complete(frame.stream_id);
                        send_response(&tx, frame.stream_id, &process_name, &response).await?;
                        continue;
                    }
                }
            } else {
                decision_request.clone()
            };

            if request_decision.action.blocks_dispatch() && request_decision.action != McpPolicyAction::Rewrite {
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
                let final_decision = decision_provider.decide_response(
                    &response_decision_request,
                    &response,
                    request_decision,
                );
                let response = match final_decision.action {
                    McpPolicyAction::Ask | McpPolicyAction::Block => {
                        policy_blocked_response(
                            dispatch_request.id.clone(),
                            "response",
                            &final_decision,
                        )
                    }
                    McpPolicyAction::Rewrite
                        if final_decision
                            .rewrite_target
                            .as_deref()
                            .is_some_and(|target| target.trim_start().starts_with("response.")) =>
                    {
                        rewrite_mcp_response(response, &final_decision).unwrap_or_else(|error| {
                            policy_blocked_response(
                                dispatch_request.id.clone(),
                                "response rewrite",
                                &McpPolicyDecision {
                                    reason: error,
                                    ..final_decision.clone()
                                },
                            )
                        })
                    }
                    McpPolicyAction::Rewrite => response,
                    McpPolicyAction::Allow => response,
                };
                let policy_fields = McpCallPolicyFields::from(&final_decision);
                log_mcp_call_with_policy(
                    &db_h,
                    &dispatch_request,
                    &response,
                    &process_name,
                    duration_ms,
                    policy_fields,
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
enum McpPolicyAction {
    Allow,
    Ask,
    Block,
    Rewrite,
}

impl McpPolicyAction {
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
struct McpPolicyDecision {
    mode: McpPolicyMode,
    action: McpPolicyAction,
    rule: String,
    reason: String,
    rewrite_target: Option<String>,
    rewrite_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy_rule_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpCallPolicyFields {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

impl From<&McpPolicyDecision> for McpCallPolicyFields {
    fn from(decision: &McpPolicyDecision) -> Self {
        Self {
            policy_mode: Some(decision.mode.as_str().to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: Some(decision.rule.clone()),
            policy_reason: Some(decision.reason.clone()),
        }
    }
}

async fn log_mcp_call_with_policy(
    db: &DbWriter,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    process_name: &str,
    duration_ms: u64,
    policy_fields: McpCallPolicyFields,
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
    db.write(WriteOp::ResolvedSecurityEvent(
        build_mcp_resolved_security_event(
            req,
            resp,
            server_name,
            tool_name,
            decision,
            &policy_fields,
            timestamp,
            trace_id,
        ),
    ))
    .await;
}

#[allow(clippy::too_many_arguments)]
fn build_mcp_resolved_security_event(
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    server_name: &str,
    tool_name: Option<&str>,
    decision: &str,
    policy_fields: &McpCallPolicyFields,
    timestamp: SystemTime,
    trace_id: Option<String>,
) -> ResolvedSecurityEvent {
    let timestamp_unix_ms = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let subject_tool_name = tool_name
        .and_then(parse_namespaced)
        .map(|(_, tool)| tool.to_string())
        .or_else(|| tool_name.map(str::to_string))
        .unwrap_or_else(|| req.method.clone());
    let event_id = mcp_security_event_id(
        trace_id.as_deref(),
        server_name,
        &subject_tool_name,
        req.id.as_ref(),
        timestamp_unix_ms,
    );
    let mut event = SecurityEvent::mcp(
        SecurityEventCommon {
            event_id,
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::Network,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: None,
            enforceability: Enforceability::InlineBlockable,
            trace_id,
            span_id: None,
            timestamp_unix_ms,
            vm_id: non_empty_env(crate::telemetry::CAPSEM_VM_ID_ENV),
            session_id: non_empty_env(crate::telemetry::CAPSEM_SESSION_ID_ENV),
            profile_id: non_empty_env(crate::telemetry::CAPSEM_PROFILE_ID_ENV),
            profile_revision: non_empty_env(crate::telemetry::CAPSEM_PROFILE_REVISION_ENV),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: non_empty_env(crate::telemetry::CAPSEM_USER_ID_ENV),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
            mcp_call_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
            event_type: "mcp.request".into(),
            redaction_state: RedactionState::Raw,
        },
        McpSecuritySubject {
            server_id: server_name.to_string(),
            tool_name: subject_tool_name,
            evidence: None,
        },
    );

    let mut steps = Vec::new();
    if let Some(action) = policy_fields
        .policy_action
        .as_deref()
        .and_then(mcp_security_decision_action)
    {
        event.decision = Some(SecurityDecision {
            action,
            rule: policy_fields.policy_rule.clone(),
            pack_id: None,
            reason: policy_fields.policy_reason.clone(),
            terminal: matches!(
                action,
                SecurityDecisionAction::Ask
                    | SecurityDecisionAction::Block
                    | SecurityDecisionAction::Rewrite
                    | SecurityDecisionAction::Throttle
            ),
        });
        steps.push(ResolvedEventStep {
            kind: ResolvedEventStepKind::EnforcementMatch,
            status: StepStatus::Matched,
            rule_id: policy_fields.policy_rule.clone(),
            pack_id: None,
            message: policy_fields.policy_reason.clone(),
        });
    }

    let final_action = match decision {
        "denied" => SecurityAction::Block(BlockResponse {
            reason_code: policy_fields
                .policy_reason
                .clone()
                .unwrap_or_else(|| "mcp_call_denied".into()),
            rule_id: policy_fields.policy_rule.clone(),
        }),
        "error" => SecurityAction::Error(SecurityError {
            code: "mcp_error".into(),
            message: resp
                .error
                .as_ref()
                .map(|error| error.message.clone())
                .unwrap_or_else(|| "MCP call failed".into()),
        }),
        _ => SecurityAction::Continue,
    };

    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event,
        steps,
        plugin_transforms: Vec::new(),
        detection_findings: Vec::new(),
        final_action,
        emitter_results: Vec::new(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn mcp_security_decision_action(action: &str) -> Option<SecurityDecisionAction> {
    match action {
        "allow" => Some(SecurityDecisionAction::Allow),
        "ask" => Some(SecurityDecisionAction::Ask),
        "block" => Some(SecurityDecisionAction::Block),
        "rewrite" => Some(SecurityDecisionAction::Rewrite),
        _ => None,
    }
}

fn mcp_security_event_id(
    trace_id: Option<&str>,
    server_name: &str,
    tool_name: &str,
    request_id: Option<&serde_json::Value>,
    timestamp_unix_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(server_name.as_bytes());
    hasher.update(tool_name.as_bytes());
    if let Some(request_id) = request_id {
        hasher.update(request_id.to_string().as_bytes());
    }
    hasher.update(&timestamp_unix_ms.to_le_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    format!("mcp-{}", &hash[..16])
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

    fn decide(&self, request: &McpDecisionRequest) -> McpPolicyDecision {
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
        base: McpPolicyDecision,
    ) -> McpPolicyDecision {
        if matches!(base.action, McpPolicyAction::Ask | McpPolicyAction::Block) {
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

    fn decide_tool_call(&self, request: &McpDecisionRequest) -> McpPolicyDecision {
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
    ) -> McpPolicyDecision {
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
    ) -> McpPolicyDecision {
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

    fn decision_from_audit_rule(&self, rule: &McpDecisionRule) -> McpPolicyDecision {
        match rule.action {
            McpDecisionRuleAction::Allow => self.allow(rule_name(rule), rule_reason(rule)),
            McpDecisionRuleAction::Deny => self.block(rule_name(rule), rule_reason(rule)),
        }
    }

    fn allow(&self, rule: String, reason: String) -> McpPolicyDecision {
        McpPolicyDecision {
            mode: self.mode,
            action: McpPolicyAction::Allow,
            rule,
            reason,
            rewrite_target: None,
            rewrite_value: None,
            policy_rule_name: None,
        }
    }

    fn ask(&self, rule: String, reason: String) -> McpPolicyDecision {
        McpPolicyDecision {
            mode: self.mode,
            action: McpPolicyAction::Ask,
            rule,
            reason,
            rewrite_target: None,
            rewrite_value: None,
            policy_rule_name: None,
        }
    }

    fn block(&self, rule: String, reason: String) -> McpPolicyDecision {
        McpPolicyDecision {
            mode: self.mode,
            action: McpPolicyAction::Block,
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
    ) -> McpPolicyDecision {
        McpPolicyDecision {
            mode: self.mode,
            action: McpPolicyAction::Rewrite,
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
    decision: &McpPolicyDecision,
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

fn disallowed_notification_decision(request: &JsonRpcRequest) -> McpPolicyDecision {
    McpPolicyDecision {
        mode: McpPolicyMode::Enforce,
        action: McpPolicyAction::Block,
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
    decision: &McpPolicyDecision,
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
    decision: &McpPolicyDecision,
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
    decision: &McpPolicyDecision,
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
            McpDecisionRuleAction::Deny => return Some(rule),
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
    format!("mcp.rule.{}", rule.id)
}

fn rule_reason(rule: &McpDecisionRule) -> String {
    rule.reason
        .clone()
        .unwrap_or_else(|| format!("audit-only local policy rule {} matched", rule.id))
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
