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
use tracing::warn;

use crate::net::dns::private::MCP_HOST;
#[cfg(test)]
use crate::net::policy_config::SecurityRuleSet;
use crate::security_engine::{
    emit_evaluated_security_rules, emit_security_write, evaluate_security_boundary, McpRequestSecurityEvent,
    McpSecurityEvent, ProcessSecurityEvent, RuntimeSecurityEventType, SecurityEnforcementAction,
    SecurityEnforcementDecision, SecurityEvent,
};
use capsem_proto::mcp_contracts::{parse_namespaced, parse_resource_uri, JsonRpcRequest, JsonRpcResponse};

use super::fd_stream::{AsyncFdStream, ReplayReader};
use super::McpEndpointState;
use capsem_telemetry::mitm as m;
mod dispatch;
mod memory;
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
    config: &super::MitmProxyConfig,
) -> Result<String, (String, Decision, String)> {
    let Some(endpoint) = &config.mcp_endpoint else {
        return Err((
            MCP_HOST.to_string(),
            Decision::Error,
            "framed MCP endpoint disabled".into(),
        ));
    };
    serve_io(initial_buf, vsock_stream, Arc::clone(endpoint), Arc::clone(&config.db)).await
}

/// Dispatch an MCP JSON-RPC request through the same security-event and
/// ledger rail used by framed guest MCP traffic.
///
/// Host-facing routes use this when they invoke an MCP tool on behalf
/// of the user. They must not call the aggregator directly, because the
/// unified tool-call ledger rows and matching security-rule rows are
/// the audit contract.
#[derive(Debug, Clone)]
pub struct LoggedMcpResponse {
    pub response: JsonRpcResponse,
    pub event_id: Option<String>,
}

/// The transport an MCP message reached Capsem on, as the ledger names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    /// Bounded frames from the in-guest relay on the MITM vsock port.
    VsockFrame,
    /// Streamable HTTP from a workload to the internal MCP name.
    Http,
    /// A host-side caller dispatching on the user's behalf.
    Direct,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VsockFrame => "vsock_frame",
            Self::Http => "http",
            Self::Direct => "direct",
        }
    }
}

pub async fn dispatch_cancellable_mcp_request(
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    request: JsonRpcRequest,
    process_name: String,
    transport: McpTransport,
    cancellation: tokio_util::sync::CancellationToken,
) -> Option<LoggedMcpResponse> {
    // Direct host calls have no framed-session owner to reserve this budget.
    let _admission = if transport == McpTransport::Direct {
        match endpoint.admit_frame(memory::request(&request)) {
            Some(admission) => Some(admission),
            None => {
                return Some(LoggedMcpResponse {
                    response: JsonRpcResponse::err(request.id.clone(), -32603, "MCP request capacity exceeded"),
                    event_id: None,
                })
            }
        }
    } else {
        None
    };
    let caller = McpCaller {
        process_name: &process_name,
        transport,
    };
    let summary = interpret_mcp_method(&request);
    let runtime_event_type = runtime_mcp_event_type(&summary.method);
    let (request_decision, request_approval_event_id) = evaluate_mcp_security_event(
        &endpoint,
        &db,
        mcp_security_event_from_summary(runtime_event_type, &summary, &process_name, None),
        memory::request(&request),
        Some(&cancellation),
    )
    .await;

    if !request_decision.is_allowed() {
        let response = policy_blocked_response(request.id.clone(), "request", &request_decision);
        let emission = log_mcp_call_with_policy(
            Arc::clone(&db),
            &endpoint,
            &request,
            &response,
            caller,
            0,
            McpCallPolicyFields {
                approval_event_id: request_approval_event_id,
                ..McpCallPolicyFields::from(&request_decision)
            },
        )
        .await;
        return Some(LoggedMcpResponse {
            response,
            event_id: emission.event_id,
        });
    }

    let start = Instant::now();
    let response = {
        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            permit = endpoint.inflight.acquire() => permit.ok(),
        };
        match permit {
            Some(_permit) => endpoint.handle_request(&request).await?,
            None => JsonRpcResponse::err(request.id.clone(), -32603, "MCP caller or endpoint stopped"),
        }
    };
    let duration_ms = start.elapsed().as_millis() as u64;

    let (response_decision, response_approval_event_id) = evaluate_mcp_security_event(
        &endpoint,
        &db,
        mcp_security_event_from_summary(
            runtime_mcp_event_type(&summary.method),
            &summary,
            &process_name,
            Some(&response),
        ),
        memory::request(&request).saturating_add(memory::response(&response)),
        Some(&cancellation),
    )
    .await;
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
        &endpoint,
        &request,
        &response,
        caller,
        duration_ms,
        McpCallPolicyFields {
            approval_event_id: request_approval_event_id.or(response_approval_event_id),
            ..McpCallPolicyFields::from(&final_decision)
        },
    )
    .await;
    Some(LoggedMcpResponse {
        response,
        event_id: emission.event_id,
    })
}

pub fn dispatch_logged_mcp_request(
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    request: JsonRpcRequest,
    process_name: String,
    transport: McpTransport,
) -> impl std::future::Future<Output = Option<LoggedMcpResponse>> + Send {
    dispatch_cancellable_mcp_request(
        endpoint,
        db,
        request,
        process_name,
        transport,
        tokio_util::sync::CancellationToken::new(),
    )
}

/// What one streamable-HTTP MCP message gets back.
#[derive(Debug)]
pub(super) enum HttpMcpReply {
    /// A request's answer: 200 with the JSON-RPC response.
    Answer(JsonRpcResponse),
    /// A notification, or a client's response to nothing we asked: 202.
    Accepted,
    /// Bytes that are no JSON-RPC request: 400 with the error.
    Invalid(JsonRpcResponse),
}

/// Dispatch one MCP message a workload POSTed to the internal MCP name:
/// the parser, in-flight bound, security rail and ledger of a framed one,
/// recorded as `http`.
pub(super) async fn dispatch_http_message(
    endpoint: Arc<McpEndpointState>,
    db: Arc<DbWriter>,
    payload: &[u8],
    process_name: &str,
    cancellation: tokio_util::sync::CancellationToken,
) -> HttpMcpReply {
    let request = match parse_json_rpc_payload(payload) {
        Ok(request) => request,
        Err(_) if is_client_response(payload) => return HttpMcpReply::Accepted,
        Err(error) => return HttpMcpReply::Invalid(JsonRpcResponse::err(error.id, error.code, error.message)),
    };
    let notification = request.id.is_none();
    let logged = dispatch_cancellable_mcp_request(
        endpoint,
        db,
        request,
        process_name.to_string(),
        McpTransport::Http,
        cancellation,
    )
    .await;
    match logged {
        Some(logged) if !notification => HttpMcpReply::Answer(logged.response),
        _ => HttpMcpReply::Accepted,
    }
}

/// A JSON-RPC response object: what a client sends back to a server request.
/// This server sends none, so there is nothing to route it to.
fn is_client_response(payload: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Shape<'a> {
        #[serde(borrow)]
        method: Option<&'a serde_json::value::RawValue>,
        #[serde(borrow)]
        result: Option<&'a serde_json::value::RawValue>,
        #[serde(borrow)]
        error: Option<&'a serde_json::value::RawValue>,
    }
    serde_json::from_slice::<Shape<'_>>(payload)
        .is_ok_and(|shape| shape.method.is_none() && (shape.result.is_some() || shape.error.is_some()))
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

    let mut writer_task = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            write_frame(&mut writer, &out)
                .await
                .context("framed MCP writer failed")?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let dispatch = Arc::new(dispatch::Dispatcher {
        endpoint: Arc::clone(&endpoint),
        db: Arc::clone(&db),
        streams: Arc::clone(&streams),
        tx: tx.clone(),
    });
    let mut workers = tokio::task::JoinSet::<Result<()>>::new();
    let mut graceful = false;
    let (mut result, writer_completed): (Result<()>, bool) = tokio::select! {
        result = async {
        loop {
            let frame = {
                // A frame parser owns consumed bytes and its body deadline.
                // Reaping another call must never replace that live future.
                let frame = read_next_frame(&mut reader);
                tokio::pin!(frame);
                loop {
                    tokio::select! {
                        biased;
                        completed = workers.join_next(), if !workers.is_empty() => {
                            if let Some(completed) = completed { completed??; }
                        }
                        frame = &mut frame => break frame?,
                    }
                }
            };
            let frame = match frame {
                FrameRead::Eof => return Ok(()),
                FrameRead::End => { graceful = true; return Ok(()); },
                FrameRead::InvalidFrame { stream_id, error } => {
                    warn!(stream_id, error, "invalid framed MCP frame discarded");
                    ::metrics::counter!(
                        m::PARSER_EVENTS_TOTAL,
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
            ::metrics::counter!(m::PARSER_EVENTS_TOTAL, "parser" => "mcp_json_rpc", "kind" => summary.kind.label()).increment(1);
            let held_bytes = memory::request(&request).saturating_add(process_name.capacity().saturating_mul(2));
            let admission = endpoint.admit_frame(held_bytes).ok_or_else(|| anyhow::anyhow!("MCP pending request capacity reached"))?;
            let dispatch = Arc::clone(&dispatch);
            workers.spawn(async move {
                let _admission = admission;
                dispatch.run(dispatch::Call { stream_id: frame.stream_id, process_name, request, disposition }, held_bytes).await
            });
        }
        } => (result, false),
        writer_result = &mut writer_task => {
            let result = match writer_result {
                Ok(Ok(())) => Err(anyhow::anyhow!("framed MCP writer stopped before the reader")),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(error.into()),
            };
            (result, true)
        }
    };

    if !graceful || writer_completed {
        workers.abort_all();
    }
    while let Some(completed) = workers.join_next().await {
        match completed {
            Ok(Ok(())) => {}
            Err(error) if error.is_cancelled() && !graceful => {}
            Ok(Err(error)) => result = Err(error),
            Err(error) => result = Err(error.into()),
        }
    }
    drop(dispatch);
    drop(tx);
    let result = if writer_completed {
        result
    } else {
        match writer_task.await {
            Ok(Ok(())) => result,
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error.into()),
        }
    };
    match &result {
        Ok(()) => {
            ::metrics::counter!(
                m::MCP_DISCONNECTS_TOTAL,
                "reason" => "eof",
            )
            .increment(1);
        }
        Err(_) => {
            ::metrics::counter!(
                m::MCP_DISCONNECTS_TOTAL,
                "reason" => "error",
            )
            .increment(1);
        }
    }

    result.map_err(|e| (MCP_HOST.to_string(), Decision::Error, format!("framed MCP: {e:#}")))?;

    Ok(MCP_HOST.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameRead {
    Eof,
    /// The guest said it is done; answers already owed still go out.
    End,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpMethodSummary {
    kind: McpMethodKind,
    method: String,
    request_id: Option<String>,
    server_name: Option<String>,
    tool_name: Option<String>,
    request_preview: Option<String>,
    request_arguments: Option<serde_json::Value>,
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
    approval_event_id: Option<String>,
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

impl From<&SecurityEnforcementDecision> for McpCallPolicyFields {
    fn from(decision: &SecurityEnforcementDecision) -> Self {
        Self {
            approval_event_id: None,
            policy_mode: Some("security_event".to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
        }
    }
}

/// Who sent an MCP message and over what, as one ledger attribution.
#[derive(Debug, Clone, Copy)]
struct McpCaller<'a> {
    process_name: &'a str,
    transport: McpTransport,
}

impl<'a> McpCaller<'a> {
    fn frame(process_name: &'a str) -> Self {
        Self {
            process_name,
            transport: McpTransport::VsockFrame,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct LoggedMcpEmission {
    event_id: Option<String>,
}

async fn log_mcp_call_with_policy(
    db: Arc<DbWriter>,
    endpoint: &McpEndpointState,
    req: &JsonRpcRequest,
    resp: &JsonRpcResponse,
    caller: McpCaller<'_>,
    duration_ms: u64,
    policy_fields: McpCallPolicyFields,
) -> LoggedMcpEmission {
    let McpCaller {
        process_name,
        transport,
    } = caller;
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
        event_id: policy_fields.approval_event_id,
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
        transport: transport.as_str().to_string(),
        policy_mode: policy_fields.policy_mode,
        policy_action: policy_fields.policy_action,
        policy_rule: policy_fields.policy_rule,
        policy_reason: policy_fields.policy_reason,
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    };
    let security_event = security_event_from_mcp_call(&call);
    if let Some(event_id) = emit_security_write(&db, WriteOp::McpCall(call)).await {
        // Complete the derived rule emission before replying so an immediate
        // shutdown cannot lose audit rows for an already-completed call.
        let (rules, _) = endpoint.policy_snapshot();
        emit_evaluated_security_rules(
            Arc::clone(&db),
            event_id.clone(),
            runtime_mcp_event_type(&req.method),
            rules,
            Arc::new(BTreeMap::new()),
            security_event,
            current_unix_ms(),
            "framed MCP call",
        )
        .await;
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
        request: Some(McpRequestSecurityEvent {
            arguments: summary.request_arguments.clone(),
            ..Default::default()
        }),
        ..Default::default()
    }
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

async fn evaluate_mcp_security_event(
    endpoint: &McpEndpointState,
    db: &DbWriter,
    event: SecurityEvent,
    held_bytes: usize,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> (SecurityEnforcementDecision, Option<String>) {
    let (rules, plugin_policy) = endpoint.policy_snapshot();
    let result = async {
        let mut evaluation =
            evaluate_security_boundary(&rules, Arc::clone(&plugin_policy), event).map_err(|error| error.to_string())?;
        if evaluation.enforcement.action == SecurityEnforcementAction::Ask {
            Box::pin(crate::security_engine::approval::enforcement::resolve_proxy(
                db,
                endpoint.approvals.as_deref(),
                &rules,
                &mut evaluation,
                held_bytes,
                cancellation,
                None,
            ))
            .await?;
            let (current_rules, current_plugins) = endpoint.policy_snapshot();
            if !Arc::ptr_eq(&rules, &current_rules) || !Arc::ptr_eq(&plugin_policy, &current_plugins) {
                evaluation.enforcement.action = SecurityEnforcementAction::Block;
                evaluation.enforcement.reason = Some("MCP policy changed while approval was pending".into());
            }
        }
        Ok::<_, String>((
            evaluation.enforcement,
            evaluation.approval_event_id.map(|id| id.as_str().to_string()),
        ))
    }
    .await;
    match result {
        Ok(enforcement) => enforcement,
        Err(error) => {
            warn!(error = %error, "MCP security event evaluation failed closed");
            (
                SecurityEnforcementDecision {
                    action: SecurityEnforcementAction::Block,
                    rule_id: Some("security.mcp.evaluation_error".to_string()),
                    rule_name: Some("mcp_security_evaluation_error".to_string()),
                    reason: Some(error),
                    ask_id: None,
                },
                None,
            )
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
