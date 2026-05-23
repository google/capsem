//! `TelemetryHook`: persists per-request telemetry (`NetEvent` plus an
//! optional `ModelCall` for AI-provider traffic) as a sync `ChunkHook`
//! firing on `on_response_end`.
//!
//! T1 slice 8. Replaces the logic in `telemetry::TelemetryEmitter`
//! and the body-wrapper firing surface from `telemetry::TelemetryBody`.
//! The ChunkHook owns its own response-side byte counting + preview
//! while per-request context (method, path, status, headers, decision,
//! matched-rule, request-side stats, etc.) is seeded into `HookState`
//! by `handle_request`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use bytes::Bytes;
use capsem_logger::{
    DbWriter, Decision, ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry, WriteOp,
};
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, BlockResponse, Enforceability, HttpBodySecuritySubject,
    HttpSecuritySubject, RedactionState, ResolvedEventStep, ResolvedEventStepKind,
    ResolvedSecurityEvent, SecurityAction, SecurityDecision, SecurityDecisionAction, SecurityError,
    SecurityEvent, SecurityEventCommon, SecurityResult, SourceEngine, StepStatus,
    RESOLVED_EVENT_SCHEMA_VERSION,
};
use tracing::{info, warn};

use super::body::BodyStats;
use super::hooks::{ChunkCtx, ChunkHook};
use super::interpreter_hook::LlmEventStream;
use super::util::is_llm_api_path;
use crate::net::ai_traffic::events::{collect_summary, parse_non_streaming_usage, StopReason};
use crate::net::ai_traffic::evidence::{build_model_interaction_evidence, ModelEvidenceInput};
use crate::net::ai_traffic::pricing::PricingTable;
use crate::net::ai_traffic::provider::{extract_model_from_path, tool_origin, ProviderKind};
use crate::net::ai_traffic::{request_parser, TraceState};

/// Per-request snapshot of the request-side fields that the response
/// completion handler needs in order to build a `NetEvent` /
/// `ModelCall`. `handle_request` seeds this into `HookState` after
/// the request head and upstream response head have been observed,
/// before the body wrapper begins iterating chunks.
#[derive(Clone)]
pub struct TelemetryRequestContext {
    pub event_id_seed: String,
    pub domain: String,
    pub process_name: Option<String>,
    pub ai_provider: Option<ProviderKind>,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub decision: Decision,
    pub matched_rule: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub start_time: Instant,
    /// Request-side byte count + preview, populated by the
    /// `TrackedBody` wrapper around the upstream request body. The
    /// hook reads the final value at `on_response_end`.
    pub request_body_stats: Arc<Mutex<BodyStats>>,
    /// `max_body_capture` for the response side (controls preview
    /// growth in the hook's own response stats).
    pub max_response_preview: usize,
    /// Upstream port for this request. 443 for the TLS path, 80
    /// (or another allowlisted port) for the plain-HTTP path. Lands
    /// in `NetEvent.port` so operators can distinguish HTTPS from
    /// plain-HTTP traffic in session.db.
    pub port: u16,
    /// `NetEvent.conn_type` label. `https-mitm` for TLS,
    /// `http-mitm` for plain HTTP.
    pub conn_type: &'static str,
    pub identity: TelemetryIdentityContext,
    pub policy_mode: Option<String>,
    pub policy_action: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
    pub runtime_security_results: Vec<SecurityResult>,
}

pub fn new_http_event_id_seed() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryIdentityContext {
    pub vm_id: Option<String>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub profile_revision: Option<String>,
    pub user_id: Option<String>,
}

impl TelemetryIdentityContext {
    pub fn from_env() -> Self {
        Self {
            vm_id: non_empty_env(crate::telemetry::CAPSEM_VM_ID_ENV),
            session_id: non_empty_env(crate::telemetry::CAPSEM_SESSION_ID_ENV),
            profile_id: non_empty_env(crate::telemetry::CAPSEM_PROFILE_ID_ENV),
            profile_revision: non_empty_env(crate::telemetry::CAPSEM_PROFILE_REVISION_ENV),
            user_id: non_empty_env(crate::telemetry::CAPSEM_USER_ID_ENV),
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn cost_micros(estimated_cost_usd: f64) -> Option<u64> {
    if estimated_cost_usd.is_finite() && estimated_cost_usd > 0.0 {
        Some((estimated_cost_usd * 1_000_000.0).round() as u64)
    } else {
        None
    }
}

/// Per-request response-side counters owned by the hook. Updated on
/// every `on_response_chunk`. The cap on the preview is taken from
/// `TelemetryRequestContext::max_response_preview` if seeded;
/// otherwise zero (no preview captured -- shadow mode).
#[derive(Default)]
pub struct TelemetryResponseStats {
    pub bytes: u64,
    pub preview: Vec<u8>,
    pub max_preview: usize,
}

/// Shared dependencies handed to `TelemetryHook` at construction --
/// the bits that need to outlive a single request and aren't
/// derivable from the per-request context.
pub struct TelemetryDeps {
    pub db: Arc<DbWriter>,
    pub pricing: Arc<PricingTable>,
    pub trace_state: Arc<Mutex<TraceState>>,
}

/// Sync `ChunkHook` that tracks response bytes/preview and, on
/// `on_response_end`, builds and writes `NetEvent` + (optionally)
/// `ModelCall` for the request just completed.
pub struct TelemetryHook {
    deps: Arc<TelemetryDeps>,
}

impl TelemetryHook {
    pub fn new(deps: Arc<TelemetryDeps>) -> Self {
        Self { deps }
    }
}

impl ChunkHook for TelemetryHook {
    fn name(&self) -> &'static str {
        "telemetry"
    }

    fn on_response_chunk(&self, chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        // Determine the per-request preview cap by peeking at the
        // request context (if any). We touch the response stats slot
        // only if the request context has been seeded -- shadow mode
        // skips the slot allocation entirely.
        let max_preview = match ctx
            .state::<Option<TelemetryRequestContext>>(|| None)
            .as_ref()
        {
            Some(req_ctx) => req_ctx.max_response_preview,
            None => return,
        };

        let stats = ctx.state::<TelemetryResponseStats>(TelemetryResponseStats::default);
        if stats.max_preview == 0 {
            stats.max_preview = max_preview;
        }
        stats.bytes += chunk.len() as u64;
        let remaining = stats.max_preview.saturating_sub(stats.preview.len());
        if remaining > 0 {
            let to_copy = remaining.min(chunk.len());
            stats.preview.extend_from_slice(&chunk[..to_copy]);
        }
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        // Move the request context out of the slot so we can take
        // ownership of its fields. After this the slot is `None` --
        // duplicate end firings (Drop fallback in ChunkDispatchBody)
        // are no-ops.
        let req_ctx = match ctx.state::<Option<TelemetryRequestContext>>(|| None).take() {
            Some(c) => c,
            None => return, // shadow mode: no seed, nothing to emit
        };

        let resp_stats =
            std::mem::take(ctx.state::<TelemetryResponseStats>(TelemetryResponseStats::default));
        let llm_events = ctx
            .state::<LlmEventStream>(LlmEventStream::default)
            .events
            .clone();

        emit_completed_http_request_with_llm_events(&self.deps, req_ctx, resp_stats, &llm_events);
    }
}

pub async fn emit_synthetic_http_response(
    deps: &TelemetryDeps,
    req_ctx: TelemetryRequestContext,
    response_body: &[u8],
) {
    let mut resp_stats = TelemetryResponseStats {
        bytes: response_body.len() as u64,
        preview: Vec::new(),
        max_preview: req_ctx.max_response_preview,
    };
    let preview_len = resp_stats.max_preview.min(response_body.len());
    if preview_len > 0 {
        resp_stats
            .preview
            .extend_from_slice(&response_body[..preview_len]);
    }
    let (net_event, resolved_events, model_call) =
        completed_http_records(deps, &req_ctx, &resp_stats, &[]);
    log_outcome(&req_ctx);

    deps.db.write(WriteOp::NetEvent(net_event)).await;
    for resolved_event in resolved_events {
        deps.db
            .write(WriteOp::ResolvedSecurityEvent(resolved_event))
            .await;
    }
    if let Some(mc) = model_call {
        deps.db.write(WriteOp::ModelCall(mc)).await;
    }
}

fn emit_completed_http_request_with_llm_events(
    deps: &TelemetryDeps,
    req_ctx: TelemetryRequestContext,
    resp_stats: TelemetryResponseStats,
    llm_events: &[crate::net::ai_traffic::events::LlmEvent],
) {
    let (net_event, resolved_events, model_call) =
        completed_http_records(deps, &req_ctx, &resp_stats, llm_events);
    log_outcome(&req_ctx);

    // Spawn DB writes so the response path doesn't block on backpressure.
    let db = Arc::clone(&deps.db);
    tokio::spawn(async move {
        db.write(WriteOp::NetEvent(net_event)).await;
        for resolved_event in resolved_events {
            db.write(WriteOp::ResolvedSecurityEvent(resolved_event))
                .await;
        }
        if let Some(mc) = model_call {
            db.write(WriteOp::ModelCall(mc)).await;
        }
    });
}

fn completed_http_records(
    deps: &TelemetryDeps,
    req_ctx: &TelemetryRequestContext,
    resp_stats: &TelemetryResponseStats,
    llm_events: &[crate::net::ai_traffic::events::LlmEvent],
) -> (NetEvent, Vec<ResolvedSecurityEvent>, Option<ModelCall>) {
    let net_event = build_net_event(&req_ctx, &resp_stats);
    let resolved_events = if req_ctx.runtime_security_results.is_empty() {
        vec![build_http_resolved_security_event(
            &req_ctx,
            &resp_stats,
            &net_event,
        )]
    } else {
        req_ctx
            .runtime_security_results
            .iter()
            .cloned()
            .map(|result| result.resolved_event)
            .collect()
    };
    let model_call = maybe_build_model_call(
        &req_ctx,
        &resp_stats,
        llm_events,
        &deps.pricing,
        &deps.trace_state,
    );
    (net_event, resolved_events, model_call)
}

/// Pure builder: assembles a `NetEvent` from the context and stats.
/// Trace ID is sampled from the ambient OTel context.
pub fn build_net_event(
    req_ctx: &TelemetryRequestContext,
    resp_stats: &TelemetryResponseStats,
) -> NetEvent {
    let duration_ms = req_ctx.start_time.elapsed().as_millis() as u64;
    let (bytes_sent, req_preview) = {
        let st = req_ctx
            .request_body_stats
            .lock()
            .expect("req body stats lock");
        let preview = if st.preview.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&st.preview).into_owned())
        };
        (st.bytes, preview)
    };
    let resp_preview = if resp_stats.preview.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&resp_stats.preview).into_owned())
    };

    NetEvent {
        timestamp: SystemTime::now(),
        domain: req_ctx.domain.clone(),
        port: req_ctx.port,
        decision: req_ctx.decision,
        process_name: req_ctx.process_name.clone(),
        pid: None,
        bytes_sent,
        bytes_received: resp_stats.bytes,
        duration_ms,
        method: Some(req_ctx.method.clone()),
        path: Some(req_ctx.path.clone()),
        query: req_ctx.query.clone(),
        status_code: req_ctx.status_code,
        matched_rule: req_ctx.matched_rule.clone(),
        request_headers: req_ctx.request_headers.clone(),
        response_headers: req_ctx.response_headers.clone(),
        request_body_preview: req_preview,
        response_body_preview: resp_preview,
        conn_type: Some(req_ctx.conn_type.to_string()),
        policy_mode: req_ctx.policy_mode.clone(),
        policy_action: req_ctx.policy_action.clone(),
        policy_rule: req_ctx.policy_rule.clone(),
        policy_reason: req_ctx.policy_reason.clone(),
        trace_id: crate::telemetry::ambient_capsem_trace_id(),
    }
}

pub fn build_http_resolved_security_event(
    req_ctx: &TelemetryRequestContext,
    resp_stats: &TelemetryResponseStats,
    net_event: &NetEvent,
) -> ResolvedSecurityEvent {
    let timestamp_unix_ms = net_event
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let rule_id = req_ctx
        .policy_rule
        .clone()
        .or_else(|| req_ctx.matched_rule.clone());
    let reason = req_ctx
        .policy_reason
        .clone()
        .or_else(|| req_ctx.matched_rule.clone());
    let mut event = build_http_security_event(
        req_ctx,
        timestamp_unix_ms,
        net_event.trace_id.clone(),
        Some(resp_stats.bytes),
        net_event.response_body_preview.clone(),
    );

    let mut steps = Vec::new();
    let final_action = match net_event.decision {
        Decision::Allowed | Decision::Redirected => {
            if let Some(rule_id) = rule_id.clone() {
                event.decision = Some(SecurityDecision {
                    action: SecurityDecisionAction::Allow,
                    rule: Some(rule_id.clone()),
                    pack_id: None,
                    reason: reason.clone(),
                    terminal: false,
                });
                steps.push(ResolvedEventStep {
                    kind: ResolvedEventStepKind::EnforcementMatch,
                    status: StepStatus::Matched,
                    rule_id: Some(rule_id),
                    pack_id: None,
                    message: reason.clone(),
                });
            }
            SecurityAction::Continue
        }
        Decision::Denied => {
            event.decision = Some(SecurityDecision {
                action: SecurityDecisionAction::Block,
                rule: rule_id.clone(),
                pack_id: None,
                reason: reason.clone(),
                terminal: true,
            });
            steps.push(ResolvedEventStep {
                kind: ResolvedEventStepKind::EnforcementMatch,
                status: StepStatus::Matched,
                rule_id: rule_id.clone(),
                pack_id: None,
                message: reason.clone(),
            });
            SecurityAction::Block(BlockResponse {
                reason_code: reason
                    .clone()
                    .unwrap_or_else(|| "network_request_denied".into()),
                rule_id,
            })
        }
        Decision::Error => {
            steps.push(ResolvedEventStep {
                kind: ResolvedEventStepKind::EnforcementMatch,
                status: StepStatus::Error,
                rule_id: rule_id.clone(),
                pack_id: None,
                message: reason.clone(),
            });
            SecurityAction::Error(SecurityError {
                code: "network_error".into(),
                message: reason.unwrap_or_else(|| "network request failed".into()),
            })
        }
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

pub fn build_http_security_event(
    req_ctx: &TelemetryRequestContext,
    timestamp_unix_ms: u64,
    trace_id: Option<String>,
    response_bytes: Option<u64>,
    response_body_preview: Option<String>,
) -> SecurityEvent {
    let event_id =
        http_security_event_id_from_trace(req_ctx, trace_id.as_deref(), timestamp_unix_ms);
    let (request_bytes, request_body_preview) = {
        let st = req_ctx
            .request_body_stats
            .lock()
            .expect("req body stats lock");
        let preview = if st.preview.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&st.preview).into_owned())
        };
        (st.bytes, preview)
    };
    SecurityEvent::http(
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
            vm_id: req_ctx.identity.vm_id.clone(),
            session_id: req_ctx.identity.session_id.clone(),
            profile_id: req_ctx.identity.profile_id.clone(),
            profile_revision: req_ctx.identity.profile_revision.clone(),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: req_ctx.identity.user_id.clone(),
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "http.request".into(),
            redaction_state: RedactionState::Raw,
        },
        HttpSecuritySubject {
            method: req_ctx.method.clone(),
            scheme: Some(http_scheme(req_ctx).into()),
            host: req_ctx.domain.clone(),
            port: Some(req_ctx.port),
            path: Some(req_ctx.path.clone()),
            query: req_ctx.query.clone(),
            url: Some(http_url(req_ctx)),
            path_class: http_path_class(&req_ctx.path),
            request_bytes,
            request_headers: parse_headers(req_ctx.request_headers.as_deref()),
            request_body: request_body_preview.map(HttpBodySecuritySubject::text),
            response_status: req_ctx.status_code,
            response_headers: parse_headers(req_ctx.response_headers.as_deref()),
            response_bytes,
            response_body: response_body_preview.map(HttpBodySecuritySubject::text),
        },
    )
}

pub fn build_http_response_security_event(
    req_ctx: &TelemetryRequestContext,
    timestamp_unix_ms: u64,
    trace_id: Option<String>,
    response_bytes: Option<u64>,
    response_body_preview: Option<String>,
) -> SecurityEvent {
    let mut event = build_http_security_event(
        req_ctx,
        timestamp_unix_ms,
        trace_id,
        response_bytes,
        response_body_preview,
    );
    event.common.event_type = "http.response".into();
    event
}

fn http_security_event_id(
    req_ctx: &TelemetryRequestContext,
    net_event: &NetEvent,
    timestamp_unix_ms: u64,
) -> String {
    http_security_event_id_from_trace(req_ctx, net_event.trace_id.as_deref(), timestamp_unix_ms)
}

fn http_security_event_id_from_trace(
    req_ctx: &TelemetryRequestContext,
    trace_id: Option<&str>,
    timestamp_unix_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(req_ctx.event_id_seed.as_bytes());
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(req_ctx.domain.as_bytes());
    hasher.update(req_ctx.method.as_bytes());
    hasher.update(req_ctx.path.as_bytes());
    if let Some(query) = &req_ctx.query {
        hasher.update(query.as_bytes());
    }
    hasher.update(&timestamp_unix_ms.to_le_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    format!("net-http-{}", &hash[..16])
}

fn http_scheme(req_ctx: &TelemetryRequestContext) -> &'static str {
    if req_ctx.conn_type == "http-mitm" {
        "http"
    } else {
        "https"
    }
}

fn http_url(req_ctx: &TelemetryRequestContext) -> String {
    match &req_ctx.query {
        Some(query) if !query.is_empty() => {
            format!(
                "{}://{}{}?{}",
                http_scheme(req_ctx),
                req_ctx.domain,
                req_ctx.path,
                query
            )
        }
        _ => format!(
            "{}://{}{}",
            http_scheme(req_ctx),
            req_ctx.domain,
            req_ctx.path
        ),
    }
}

fn http_path_class(path: &str) -> String {
    if path == "/" {
        "root".into()
    } else {
        path.trim_start_matches('/')
            .split('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or("unknown")
            .to_owned()
    }
}

fn parse_headers(headers: Option<&str>) -> BTreeMap<String, Vec<String>> {
    let mut parsed = BTreeMap::new();
    let Some(headers) = headers else {
        return parsed;
    };
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        parsed
            .entry(name)
            .or_insert_with(Vec::new)
            .push(value.trim().to_string());
    }
    parsed
}

/// Pure builder: assembles a `ModelCall` for AI-provider traffic.
/// Returns `None` for non-AI domains, HEAD requests (connectivity
/// probes), and non-LLM API paths (e.g. `/api/.../metrics`,
/// `/v1/models`).
pub fn maybe_build_model_call(
    req_ctx: &TelemetryRequestContext,
    resp_stats: &TelemetryResponseStats,
    llm_events: &[crate::net::ai_traffic::events::LlmEvent],
    pricing: &PricingTable,
    trace_state: &Arc<Mutex<TraceState>>,
) -> Option<ModelCall> {
    let provider = req_ctx.ai_provider?;
    if req_ctx.method == "HEAD" || !is_llm_api_path(provider, &req_ctx.path) {
        return None;
    }
    let duration_ms = req_ctx.start_time.elapsed().as_millis() as u64;
    let (bytes_sent, req_body_bytes) = {
        let st = req_ctx
            .request_body_stats
            .lock()
            .expect("req body stats lock");
        (st.bytes, st.preview.clone())
    };

    // Parse request body for metadata (model, message count, tools, tool_results).
    let req_meta = request_parser::parse_request(provider, &req_body_bytes);

    let summary = if llm_events.is_empty() {
        None
    } else {
        Some(collect_summary(llm_events))
    };

    // Streaming detection: explicit body field OR URL path keyword.
    let stream = req_meta.stream || req_ctx.path.contains("stream");

    let stop_reason_str =
        summary
            .as_ref()
            .and_then(|s| s.stop_reason.as_ref())
            .map(|sr| match sr {
                StopReason::EndTurn => "end_turn".to_string(),
                StopReason::ToolUse => "tool_use".to_string(),
                StopReason::MaxTokens => "max_tokens".to_string(),
                StopReason::ContentFilter => "content_filter".to_string(),
                StopReason::Other(s) => s.clone(),
            });

    let tool_calls: Vec<ToolCallEntry> = summary
        .as_ref()
        .map(|s| {
            s.tool_calls
                .iter()
                .map(|tc| ToolCallEntry {
                    call_index: tc.index,
                    call_id: tc.call_id.clone(),
                    tool_name: tc.name.clone(),
                    arguments: if tc.arguments.is_empty() {
                        None
                    } else {
                        Some(tc.arguments.clone())
                    },
                    origin: tool_origin(&tc.name).to_string(),
                    trace_id: crate::telemetry::ambient_capsem_trace_id(),
                })
                .collect()
        })
        .unwrap_or_default();

    let tool_responses: Vec<ToolResponseEntry> = req_meta
        .tool_results
        .iter()
        .map(|tr| ToolResponseEntry {
            call_id: tr.call_id.clone(),
            content_preview: Some(tr.content_preview.clone()),
            is_error: tr.is_error,
            trace_id: crate::telemetry::ambient_capsem_trace_id(),
        })
        .collect();

    // Non-streaming usage fallback: when SSE stream produced no
    // input_tokens, parse the JSON response body.
    let (resp_model, resp_input, resp_output, resp_details) = if summary
        .as_ref()
        .map(|s| s.input_tokens.is_none())
        .unwrap_or(true)
    {
        if !resp_stats.preview.is_empty() && req_ctx.status_code == Some(200) {
            parse_non_streaming_usage(provider, &resp_stats.preview)
        } else {
            (None, None, None, BTreeMap::new())
        }
    } else {
        (None, None, None, BTreeMap::new())
    };

    // Resolve model: request body > SSE stream > response JSON > URL path.
    let effective_model = req_meta
        .model
        .clone()
        .or_else(|| summary.as_ref().and_then(|s| s.model.clone()))
        .or(resp_model)
        .or_else(|| extract_model_from_path(&req_ctx.path));

    let input_tokens = summary.as_ref().and_then(|s| s.input_tokens).or(resp_input);
    let output_tokens = summary
        .as_ref()
        .and_then(|s| s.output_tokens)
        .or(resp_output);
    let mut usage_details = summary
        .as_ref()
        .map(|s| s.usage_details.clone())
        .unwrap_or_default();
    if usage_details.is_empty() {
        usage_details = resp_details;
    }

    let estimated_cost_usd = pricing.estimate_cost(
        provider.as_str(),
        effective_model.as_deref(),
        input_tokens,
        output_tokens,
        &usage_details,
    );

    // Trace correlation: tool_response IDs index into the live
    // trace map; tool_call IDs register new pending entries; a
    // non-tool-use stop completes the trace.
    let tool_response_ids: Vec<String> = req_meta
        .tool_results
        .iter()
        .map(|tr| tr.call_id.clone())
        .collect();
    let tool_call_ids: Vec<String> = tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
    let trace_id = {
        let mut state = trace_state.lock().unwrap_or_else(|e| e.into_inner());
        let tid = state
            .lookup(&tool_response_ids)
            .or_else(crate::telemetry::ambient_capsem_trace_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let is_tool_use = !tool_call_ids.is_empty()
            || stop_reason_str
                .as_deref()
                .map(|r| r.contains("tool") || r == "tool_use")
                .unwrap_or(false);
        if is_tool_use && !tool_call_ids.is_empty() {
            state.register_tool_calls(&tid, &tool_call_ids);
        } else if !is_tool_use {
            state.complete_trace(&tid);
        }
        tid
    };

    let request_body_preview = if req_body_bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&req_body_bytes).into_owned())
    };
    let interaction_id = format!("model:{trace_id}:{}", uuid::Uuid::new_v4());
    let request_id = format!("request:{trace_id}:{}", uuid::Uuid::new_v4());
    let ai_evidence = Some(build_model_interaction_evidence(ModelEvidenceInput {
        interaction_id: &interaction_id,
        trace_id: &trace_id,
        request_id: &request_id,
        response_id: summary.as_ref().and_then(|s| s.message_id.as_deref()),
        provider,
        path: &req_ctx.path,
        request: &req_meta,
        response: summary.as_ref(),
        estimated_cost_micros: cost_micros(estimated_cost_usd),
        attribution_scope: AiAttributionScope::Vm,
        source_engine: SourceEngine::Network,
        origin_kind: AiOriginKind::GuestNetwork,
        accounting_owner: None,
        profile_id: req_ctx.identity.profile_id.as_deref(),
        vm_id: req_ctx.identity.vm_id.as_deref(),
        session_id: req_ctx.identity.session_id.as_deref(),
        user_id: req_ctx.identity.user_id.as_deref(),
    }));

    let model_call = ModelCall {
        timestamp: SystemTime::now(),
        provider: provider.as_str().to_string(),
        model: effective_model,
        process_name: req_ctx.process_name.clone(),
        pid: None,
        method: req_ctx.method.clone(),
        path: req_ctx.path.clone(),
        stream,
        system_prompt_preview: req_meta.system_prompt_preview,
        messages_count: req_meta.messages_count,
        tools_count: req_meta.tools_count,
        request_bytes: bytes_sent,
        request_body_preview,
        message_id: summary.as_ref().and_then(|s| s.message_id.clone()),
        status_code: req_ctx.status_code,
        text_content: summary
            .as_ref()
            .map(|s| s.text.clone())
            .filter(|s| !s.is_empty()),
        thinking_content: summary
            .as_ref()
            .map(|s| s.thinking.clone())
            .filter(|s| !s.is_empty()),
        stop_reason: stop_reason_str,
        input_tokens,
        output_tokens,
        usage_details,
        duration_ms,
        response_bytes: resp_stats.bytes,
        estimated_cost_usd,
        trace_id: Some(trace_id),
        ai_evidence,
        tool_calls,
        tool_responses,
    };

    if model_call.model.is_none() {
        warn!(
            provider = provider.as_str(),
            path = req_ctx.path,
            "MITM proxy: model_call has NULL model"
        );
    }

    Some(model_call)
}

/// Per-request log line, mirrors what `TelemetryEmitter::emit` does.
fn log_outcome(req_ctx: &TelemetryRequestContext) {
    match req_ctx.decision {
        Decision::Allowed => info!(
            domain = req_ctx.domain,
            method = req_ctx.method,
            path = req_ctx.path,
            status = ?req_ctx.status_code,
            "MITM proxy: completed"
        ),
        Decision::Denied => info!(
            domain = req_ctx.domain,
            method = req_ctx.method,
            path = req_ctx.path,
            "MITM proxy: denied"
        ),
        Decision::Error => warn!(
            domain = req_ctx.domain,
            method = req_ctx.method,
            "MITM proxy: error"
        ),
        // T3.d added Decision::Redirected for the DNS path. The MITM
        // proxy doesn't produce it today (no HTTP-level redirect rule
        // exists), but the variant is in scope here, so treat it as
        // an Allowed-shaped successful response to keep log shape
        // stable if a future MITM rewrite rule ever uses this code
        // path.
        Decision::Redirected => info!(
            domain = req_ctx.domain,
            method = req_ctx.method,
            path = req_ctx.path,
            status = ?req_ctx.status_code,
            "MITM proxy: redirected"
        ),
    }
}

#[cfg(test)]
mod tests;
