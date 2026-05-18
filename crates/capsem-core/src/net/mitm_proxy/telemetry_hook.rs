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
use tracing::{info, warn};

use super::body::BodyStats;
use super::hooks::{ChunkCtx, ChunkHook};
use super::interpreter_hook::LlmEventStream;
use super::util::is_llm_api_path;
use crate::net::ai_traffic::events::{collect_summary, parse_non_streaming_usage, StopReason};
use crate::net::ai_traffic::pricing::PricingTable;
use crate::net::ai_traffic::provider::{extract_model_from_path, tool_origin, ProviderKind};
use crate::net::ai_traffic::{request_parser, TraceState};

/// Per-request snapshot of the request-side fields that the response
/// completion handler needs in order to build a `NetEvent` /
/// `ModelCall`. `handle_request` seeds this into `HookState` after
/// the request head and upstream response head have been observed,
/// before the body wrapper begins iterating chunks.
pub struct TelemetryRequestContext {
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
    pub policy_mode: Option<String>,
    pub policy_action: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
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

        let net_event = build_net_event(&req_ctx, &resp_stats);
        let model_call = maybe_build_model_call(
            &req_ctx,
            &resp_stats,
            &llm_events,
            &self.deps.pricing,
            &self.deps.trace_state,
        );

        log_outcome(&req_ctx);

        // Spawn DB writes so the body completion path doesn't block
        // on backpressure.
        let db = Arc::clone(&self.deps.db);
        tokio::spawn(async move {
            db.write(WriteOp::NetEvent(net_event)).await;
            if let Some(mc) = model_call {
                db.write(WriteOp::ModelCall(mc)).await;
            }
        });
    }
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
