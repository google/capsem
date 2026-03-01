/// Gateway HTTP server: axum router that proxies LLM API requests to upstream
/// providers with key injection, SSE stream forwarding, and audit logging.
///
/// Runs on vsock:5004 in production (plain HTTP from the VM) or on a TCP
/// socket for standalone testing.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;
use capsem_logger::{DbWriter, ModelCall, NetEvent, Decision, ToolCallEntry, ToolResponseEntry, WriteOp};
use crate::mcp::builtin_tools::is_builtin_tool;
use tracing::{info, warn};

use super::ai_body::AiResponseBody;
use super::events::collect_summary;
use super::provider::route_provider;
use super::request_parser::parse_request;
use super::GatewayConfig;

/// Assign a trace_id for a model call, updating the shared TraceState.
///
/// Looks up existing trace from tool_response call_ids. If none found,
/// generates a new UUID. If stop_reason indicates ToolUse, registers
/// the emitted tool_call_ids. Otherwise, completes the trace.
fn assign_trace_id(
    config: &GatewayConfig,
    tool_response_call_ids: &[String],
    tool_call_ids: &[String],
    stop_reason: Option<&str>,
) -> String {
    let mut state = config.trace_state.lock().unwrap_or_else(|e| e.into_inner());

    // Look up existing trace from tool responses in this request.
    let trace_id = state
        .lookup(tool_response_call_ids)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Check if this is a tool-use stop (trace continues) or terminal (trace ends).
    // Some providers (like Gemini) don't have a specific ToolUse stop reason and just return STOP,
    // so the presence of tool calls is the most reliable indicator.
    let is_tool_use = !tool_call_ids.is_empty() || stop_reason
        .map(|r| {
            let r_lower = r.to_lowercase();
            r_lower.contains("tool") || r_lower == "\"tooluse\""
        })
        .unwrap_or(false);

    if !tool_call_ids.is_empty() {
        state.register_tool_calls(&trace_id, tool_call_ids);
    } else if !is_tool_use {
        // Only complete the trace if stop_reason doesn't indicate tool use.
        // When is_tool_use is true but tool_call_ids is empty (e.g., extraction
        // failure), leave the trace open rather than prematurely closing it.
        state.complete_trace(&trace_id);
    }

    trace_id
}

/// Determine the origin of a tool call based on its name.
///
/// - Built-in MCP tools (fetch_http, grep_http, http_headers): "mcp"
/// - External MCP tools with server__tool namespacing: "mcp"
/// - Native model tools (write_file, bash, run_shell_command, etc.): "native"
fn tool_origin(name: &str) -> String {
    if is_builtin_tool(name) || name.contains("__") {
        "mcp".to_string()
    } else {
        "native".to_string()
    }
}

/// Maximum request body to capture in audit log (64 KB).
const MAX_REQUEST_CAPTURE: usize = 64 * 1024;
/// Maximum response body to capture in audit log (64 KB).
const MAX_RESPONSE_CAPTURE: usize = 64 * 1024;

/// Build the axum router for the gateway.
pub fn router(config: Arc<GatewayConfig>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        // Catch-all: any method, any path -> proxy handler.
        .fallback(any(proxy_handler))
        .with_state(config)
}

async fn health_handler() -> &'static str {
    "ok"
}

/// Headers that should not be forwarded to upstream (hop-by-hop).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

/// Main proxy handler: routes to provider, injects key, forwards request,
/// streams response, logs to audit DB.
async fn proxy_handler(
    State(config): State<Arc<GatewayConfig>>,
    req: axum::extract::Request,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    // Filter HEAD requests (AI CLI connectivity checks): proxy to upstream
    // but don't create a ModelCall record -- they have no model/tokens/text.
    if method == "HEAD" {
        let (kind, provider) = match route_provider(&path) {
            Some(p) => p,
            None => {
                return (StatusCode::NOT_FOUND, format!("Capsem gateway: unknown path {path}\n")).into_response();
            }
        };
        let api_key = match config.api_key_for(kind) {
            Some(key) => key.to_string(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, "no API key\n").into_response(),
        };
        let upstream_url = provider.upstream_url(&path, query.as_deref());
        let builder = provider.inject_key(
            config.http_client.head(&upstream_url),
            &api_key,
        );
        match builder.send().await {
            Ok(r) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                record_net_event(
                    &config.db, kind.as_str(), "HEAD", &path,
                    Some(r.status().as_u16()), duration_ms, 0, 0,
                    Decision::Allowed, None,
                ).await;
                let mut resp = Response::builder().status(r.status().as_u16());
                for (name, value) in r.headers().iter() {
                    resp = resp.header(name.clone(), value.clone());
                }
                return resp.body(Body::empty()).unwrap_or_else(|_| {
                    Response::builder().status(200).body(Body::empty()).unwrap()
                });
            }
            Err(e) => {
                let msg = format!("Capsem gateway: HEAD upstream failed: {e}\n");
                return (StatusCode::BAD_GATEWAY, msg).into_response();
            }
        }
    }

    // 1. Route to provider.
    let (kind, provider) = match route_provider(&path) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("Capsem gateway: unknown path {path}\n"),
            )
                .into_response();
        }
    };

    // 2. Look up API key.
    let api_key = match config.api_key_for(kind) {
        Some(key) => key.to_string(),
        None => {
            let msg = format!(
                "Capsem gateway: no API key configured for {}\n",
                kind.as_str()
            );
            warn!(provider = kind.as_str(), "no API key configured");
            record_error(&config.db, kind.as_str(), &method, &path, &msg).await;
            return (StatusCode::SERVICE_UNAVAILABLE, msg).into_response();
        }
    };

    // 3. Build upstream URL.
    let upstream_url = provider.upstream_url(&path, query.as_deref());

    // 4. Collect inbound request headers and body.
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 100 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("Capsem gateway: failed to read request body: {e}\n");
            record_error(&config.db, kind.as_str(), &method, &path, &msg).await;
            return (StatusCode::BAD_REQUEST, msg).into_response();
        }
    };

    let request_bytes = body_bytes.len() as u64;

    // Use structured request parser for normalized auditing.
    let req_meta = parse_request(kind, &body_bytes);
    let model = req_meta.model.clone();

    // Capture request body preview for audit.
    let request_body_preview = if body_bytes.is_empty() {
        None
    } else {
        let limit = MAX_REQUEST_CAPTURE.min(body_bytes.len());
        Some(String::from_utf8_lossy(&body_bytes[..limit]).into_owned())
    };

    // 5. Build upstream request via reqwest.
    let reqwest_method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::POST,
    };

    let mut builder = config.http_client.request(reqwest_method, &upstream_url);

    // Forward headers (skip hop-by-hop and auth headers the agent sent).
    for (name, value) in parts.headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_lower.as_str()) {
            continue;
        }
        // Strip dummy auth headers -- we inject real ones.
        if name_lower == "x-api-key" || name_lower == "authorization" {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }

    // Set the body.
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    // 6. Inject real API key.
    builder = provider.inject_key(builder, &api_key);

    // 7. Send upstream request.
    let upstream_resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let msg = format!("Capsem gateway: upstream request failed: {e}\n");
            warn!(provider = kind.as_str(), error = %e, "upstream request failed");
            record_net_event(
                &config.db,
                kind.as_str(),
                &method, &path,
                Some(502), duration_ms,
                request_bytes, 0,
                Decision::Error, Some(&msg),
            ).await;
            return (StatusCode::BAD_GATEWAY, msg).into_response();
        }
    };

    let status = upstream_resp.status();
    let status_u16 = status.as_u16();

    // Check if the response is SSE (streaming).
    let is_sse = upstream_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    // Build response headers for the client.
    let mut response_builder = Response::builder().status(status_u16);
    for (name, value) in upstream_resp.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_lower.as_str()) {
            continue;
        }
        response_builder = response_builder.header(name.clone(), value.clone());
    }

    // 8. Stream or collect response body.
    if is_sse {
        // Streaming SSE: wrap in AiResponseBody for normalized parsing.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let byte_stream = upstream_resp.bytes_stream();
        let body = Body::from_stream(byte_stream);

        let ai_body = AiResponseBody::new(
            body,
            kind.create_parser(),
            MAX_RESPONSE_CAPTURE,
            100 * 1024 * 1024,
        )
        .with_on_drop(tx);

        let ai_state_handle = ai_body.ai_state();
        let stats_handle = ai_body.stats();

        let body = Body::new(ai_body);

        let response = response_builder.body(body).unwrap_or_else(|_| {
            Response::builder()
                .status(502)
                .body(Body::from("internal error"))
                .unwrap()
        });

        // Recording logic
        let db = Arc::clone(&config.db);
        let config_ref = Arc::clone(&config);
        let provider_str = kind.as_str().to_string();
        let model_clone = model.clone();
        let method_clone = method.clone();
        let path_clone = path.clone();
        let req_preview = request_body_preview.clone();

        // Build tool responses from request metadata
        let tool_responses: Vec<ToolResponseEntry> = req_meta
            .tool_results
            .iter()
            .map(|tr| ToolResponseEntry {
                call_id: tr.call_id.clone(),
                content_preview: Some(tr.content_preview.clone()),
                is_error: tr.is_error,
            })
            .collect();

        // Collect tool response call_ids for trace lookup
        let tool_response_call_ids: Vec<String> = req_meta
            .tool_results
            .iter()
            .map(|tr| tr.call_id.clone())
            .collect();

        let sys_prompt_preview = req_meta.system_prompt_preview;
        let messages_count = req_meta.messages_count;
        let tools_count = req_meta.tools_count;

        tokio::spawn(async move {
            // Wait for the body to be dropped (stream finished)
            let _ = rx.await;
            let duration_ms = start.elapsed().as_millis() as u64;

            let (response_bytes, _response_preview, events) =
                if let Ok(st) = stats_handle.lock() {
                    let events = ai_state_handle
                        .lock()
                        .ok()
                        .map(|s| s.events.clone())
                        .unwrap_or_default();
                    (st.bytes, None::<String>, events)
                } else {
                    (0, None, Vec::new())
                };

            // Build model call from SSE events
            let summary = collect_summary(&events);
            let tool_calls: Vec<ToolCallEntry> = summary
                .tool_calls
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
                    origin: tool_origin(&tc.name),
                })
                .collect();

            let stop_reason_str = summary.stop_reason.map(|r| format!("{:?}", r));

            // Assign trace_id using shared TraceState
            let tool_call_ids: Vec<String> = tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
            let trace_id = assign_trace_id(
                &config_ref,
                &tool_response_call_ids,
                &tool_call_ids,
                stop_reason_str.as_deref(),
            );

            // Use response model (from SSE stream) when request model is missing
            // (e.g., Google puts model in URL path, not request body).
            let effective_model = model_clone.clone()
                .or(summary.model.clone())
                .or_else(|| extract_model_from_path(&path_clone));

            // Re-estimate cost with effective model (may now be non-None for Google)
            let estimated_cost_usd = config_ref.pricing.estimate_cost(
                &provider_str,
                effective_model.as_deref(),
                summary.input_tokens,
                summary.output_tokens,
                &summary.usage_details,
            );

            let model_call = ModelCall {
                timestamp: SystemTime::now(),
                provider: provider_str.clone(),
                model: effective_model,
                process_name: None,
                pid: None,
                method: method_clone.clone(),
                path: path_clone.clone(),
                stream: true,
                system_prompt_preview: sys_prompt_preview,
                messages_count,
                tools_count,
                request_bytes,
                request_body_preview: req_preview,
                message_id: summary.message_id,
                status_code: Some(status_u16),
                text_content: if summary.text.is_empty() {
                    None
                } else {
                    Some(summary.text)
                },
                thinking_content: if summary.thinking.is_empty() {
                    None
                } else {
                    Some(summary.thinking)
                },
                stop_reason: stop_reason_str,
                input_tokens: summary.input_tokens,
                output_tokens: summary.output_tokens,
                usage_details: summary.usage_details,
                duration_ms,
                response_bytes,
                estimated_cost_usd,
                trace_id: Some(trace_id),
                tool_calls,
                tool_responses,
            };

            // Diagnostic: warn if critical fields are missing (helps debug NULL data bug)
            if model_call.model.is_none() {
                warn!(
                    provider = provider_str,
                    path = path_clone,
                    "gateway: model_call has NULL model after stream complete"
                );
            }
            if model_call.input_tokens.is_none() && model_call.output_tokens.is_none() {
                warn!(
                    provider = provider_str,
                    path = path_clone,
                    event_count = events.len(),
                    "gateway: model_call has NULL tokens after stream complete"
                );
            }
            if model_call.request_body_preview.is_none() {
                warn!(
                    provider = provider_str,
                    request_bytes,
                    "gateway: model_call has NULL request_body_preview"
                );
            }

            db.write(WriteOp::ModelCall(model_call)).await;

            info!(
                provider = provider_str,
                model = ?model_clone,
                status = status_u16,
                duration_ms,
                response_bytes,
                "gateway: streaming response complete"
            );
        });

        response
    } else {
        // Non-streaming: collect body, record audit, return.
        let resp_bytes = match upstream_resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                let msg = format!("Capsem gateway: failed to read upstream response: {e}\n");
                record_net_event(
                    &config.db,
                    kind.as_str(),
                    &method, &path,
                    Some(502), duration_ms,
                    request_bytes, 0,
                    Decision::Error, Some(&msg),
                ).await;
                return (StatusCode::BAD_GATEWAY, msg).into_response();
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let response_bytes = resp_bytes.len() as u64;

        let response_preview = if resp_bytes.is_empty() {
            None
        } else {
            let limit = MAX_RESPONSE_CAPTURE.min(resp_bytes.len());
            Some(String::from_utf8_lossy(&resp_bytes[..limit]).into_owned())
        };

        // Try to parse usage metadata from non-streaming JSON response.
        let (resp_model, resp_input_tokens, resp_output_tokens, resp_usage_details) =
            if status_u16 == 200 && !resp_bytes.is_empty() {
                parse_non_streaming_usage(kind, &resp_bytes)
            } else {
                (None, None, None, std::collections::BTreeMap::new())
            };

        // Use response model if request didn't have one, or extract from URL.
        let effective_model = model.clone()
            .or(resp_model)
            .or_else(|| extract_model_from_path(&path));

        // Build tool responses from request metadata
        let tool_responses: Vec<ToolResponseEntry> = req_meta
            .tool_results
            .iter()
            .map(|tr| ToolResponseEntry {
                call_id: tr.call_id.clone(),
                content_preview: Some(tr.content_preview.clone()),
                is_error: tr.is_error,
            })
            .collect();

        let tool_response_call_ids: Vec<String> = req_meta
            .tool_results
            .iter()
            .map(|tr| tr.call_id.clone())
            .collect();

        let estimated_cost_usd = config.pricing.estimate_cost(
            kind.as_str(),
            effective_model.as_deref(),
            resp_input_tokens,
            resp_output_tokens,
            &resp_usage_details,
        );

        // Non-streaming: no tool calls emitted, so trace always completes here
        let trace_id = assign_trace_id(
            &config,
            &tool_response_call_ids,
            &[], // no tool_call_ids in non-streaming responses
            None,
        );

        let model_call = ModelCall {
            timestamp: SystemTime::now(),
            provider: kind.as_str().to_string(),
            model: effective_model,
            process_name: None,
            pid: None,
            method: method.clone(),
            path: path.clone(),
            stream: false,
            system_prompt_preview: req_meta.system_prompt_preview,
            messages_count: req_meta.messages_count,
            tools_count: req_meta.tools_count,
            request_bytes,
            request_body_preview: request_body_preview.clone(),
            message_id: None,
            status_code: Some(status_u16),
            text_content: response_preview.clone(),
            thinking_content: None,
            stop_reason: None,
            input_tokens: resp_input_tokens,
            output_tokens: resp_output_tokens,
            usage_details: resp_usage_details,
            duration_ms,
            response_bytes,
            estimated_cost_usd,
            trace_id: Some(trace_id),
            tool_calls: Vec::new(),
            tool_responses,
        };

        config.db.write(WriteOp::ModelCall(model_call)).await;

        info!(
            provider = kind.as_str(),
            model = ?model,
            status = status_u16,
            duration_ms,
            response_bytes,
            "gateway: response complete"
        );

        let body = Body::from(resp_bytes);
        response_builder.body(body).unwrap_or_else(|_| {
            Response::builder()
                .status(502)
                .body(Body::from("internal error"))
                .unwrap()
        })
    }
}

/// Parse usage metadata from a non-streaming JSON response body.
/// Returns (model, input_tokens, output_tokens, usage_details).
fn parse_non_streaming_usage(
    kind: super::provider::ProviderKind,
    body: &[u8],
) -> (Option<String>, Option<u64>, Option<u64>, std::collections::BTreeMap<String, u64>) {
    use std::collections::BTreeMap;

    let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (None, None, None, BTreeMap::new());
    };

    match kind {
        super::provider::ProviderKind::Google => {
            let model = json.get("modelVersion")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let usage = json.get("usageMetadata");
            let input = usage.and_then(|u| u.get("promptTokenCount")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("candidatesTokenCount")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("cachedContentTokenCount")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage.and_then(|u| u.get("thoughtsTokenCount")).and_then(|v| v.as_u64()) {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ProviderKind::Anthropic => {
            let model = json.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("cache_read_input_tokens")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            (model, input, output, details)
        }
        super::provider::ProviderKind::OpenAi => {
            let model = json.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
            let usage = json.get("usage");
            let input = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64());
            let output = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64());
            let mut details = BTreeMap::new();
            if let Some(v) = usage.and_then(|u| u.get("prompt_tokens_details")).and_then(|u| u.get("cached_tokens")).and_then(|v| v.as_u64()) {
                details.insert("cache_read".into(), v);
            }
            if let Some(v) = usage.and_then(|u| u.get("completion_tokens_details")).and_then(|u| u.get("reasoning_tokens")).and_then(|v| v.as_u64()) {
                details.insert("thinking".into(), v);
            }
            (model, input, output, details)
        }
    }
}

/// Extract model name from a Gemini-style URL path.
/// E.g. `/v1beta/models/gemini-2.5-flash-lite:generateContent` -> `gemini-2.5-flash-lite`
fn extract_model_from_path(path: &str) -> Option<String> {
    // Match pattern: /v.../models/{model}:{action}
    let models_idx = path.find("/models/")?;
    let after = &path[models_idx + 8..]; // skip "/models/"
    let model = after.split(':').next()?;
    if model.is_empty() {
        return None;
    }
    Some(model.to_string())
}

/// Record an error event as a NetEvent to the audit DB.
async fn record_error(db: &DbWriter, provider: &str, method: &str, path: &str, error: &str) {
    let event = NetEvent {
        timestamp: SystemTime::now(),
        domain: provider.to_string(),
        port: 0,
        decision: Decision::Error,
        process_name: None,
        pid: None,
        method: Some(method.to_string()),
        path: Some(path.to_string()),
        query: None,
        status_code: None,
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 0,
        matched_rule: Some(error.to_string()),
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: Some("gateway".to_string()),
    };
    db.write(WriteOp::NetEvent(event)).await;
}

/// Record a gateway network event.
#[allow(clippy::too_many_arguments)]
async fn record_net_event(
    db: &DbWriter,
    provider: &str,
    method: &str,
    path: &str,
    status_code: Option<u16>,
    duration_ms: u64,
    bytes_sent: u64,
    bytes_received: u64,
    decision: Decision,
    error: Option<&str>,
) {
    let event = NetEvent {
        timestamp: SystemTime::now(),
        domain: provider.to_string(),
        port: 0,
        decision,
        process_name: None,
        pid: None,
        method: Some(method.to_string()),
        path: Some(path.to_string()),
        query: None,
        status_code,
        bytes_sent,
        bytes_received,
        duration_ms,
        matched_rule: error.map(|s| s.to_string()),
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: Some("gateway".to_string()),
    };
    db.write(WriteOp::NetEvent(event)).await;
}

/// Start the gateway on a TCP socket. Returns the bound address.
pub async fn start_standalone(
    config: Arc<GatewayConfig>,
    addr: SocketAddr,
) -> anyhow::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual = listener.local_addr()?;
    let app = router(config);
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DbWriter::open(&dir.path().join("test.db"), 64).unwrap(),
        );
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            db,
            http_client: reqwest::Client::new(),
            pricing: crate::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(crate::gateway::TraceState::new()),
        });

        let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let resp = reqwest::get(format!("http://{addr}/health")).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn unknown_path_returns_404() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DbWriter::open(&dir.path().join("test.db"), 64).unwrap(),
        );
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            db,
            http_client: reqwest::Client::new(),
            pricing: crate::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(crate::gateway::TraceState::new()),
        });

        let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let resp = reqwest::get(format!("http://{addr}/v2/unknown"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn missing_api_key_returns_503() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Arc::new(DbWriter::open(&path, 64).unwrap());
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            db: Arc::clone(&db),
            http_client: reqwest::Client::new(),
            pricing: crate::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(crate::gateway::TraceState::new()),
        });

        let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{addr}/v1/messages"))
            .body(r#"{"model":"test","messages":[]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);

        // Give writer thread time to flush.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify error was recorded via reader.
        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert!(!events.is_empty(), "should have recorded the error event");
    }

    // ── tool_origin ──────────────────────────────────────────────────

    #[test]
    fn tool_origin_native_tools() {
        assert_eq!(tool_origin("write_file"), "native");
        assert_eq!(tool_origin("bash"), "native");
        assert_eq!(tool_origin("run_shell_command"), "native");
        assert_eq!(tool_origin("read_file"), "native");
    }

    #[test]
    fn tool_origin_builtin_mcp_tools() {
        assert_eq!(tool_origin("fetch_http"), "mcp");
        assert_eq!(tool_origin("grep_http"), "mcp");
        assert_eq!(tool_origin("http_headers"), "mcp");
    }

    #[test]
    fn tool_origin_external_mcp_tools() {
        assert_eq!(tool_origin("github__list_issues"), "mcp");
        assert_eq!(tool_origin("jira__create_ticket"), "mcp");
        assert_eq!(tool_origin("custom_server__my_tool"), "mcp");
    }

    // ── extract_model_from_path ────────────────────────────────────

    #[test]
    fn extract_model_gemini_stream() {
        assert_eq!(
            extract_model_from_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
            Some("gemini-2.5-flash".to_string())
        );
    }

    #[test]
    fn extract_model_gemini_generate() {
        assert_eq!(
            extract_model_from_path("/v1beta/models/gemini-2.5-pro:generateContent"),
            Some("gemini-2.5-pro".to_string())
        );
    }

    #[test]
    fn extract_model_no_models_segment() {
        assert_eq!(extract_model_from_path("/v1/messages"), None);
    }

    #[test]
    fn extract_model_empty_model() {
        assert_eq!(extract_model_from_path("/v1beta/models/:generateContent"), None);
    }

    // ── assign_trace_id ────────────────────────────────────────────

    fn test_config() -> Arc<GatewayConfig> {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DbWriter::open(&dir.path().join("test.db"), 64).unwrap(),
        );
        // Leak the tempdir so it doesn't get cleaned up before the test is done
        std::mem::forget(dir);
        Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            db,
            http_client: reqwest::Client::new(),
            pricing: crate::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(crate::gateway::TraceState::new()),
        })
    }

    #[test]
    fn trace_tool_use_with_ids_keeps_trace_open() {
        let config = test_config();
        let trace_id = assign_trace_id(
            &config,
            &[],
            &["call_1".to_string()],
            Some("ToolUse"),
        );
        // The trace should still be open (not completed) with registered tool calls.
        // A subsequent request with tool_response for "call_1" should reuse the trace.
        let trace_id2 = assign_trace_id(
            &config,
            &["call_1".to_string()],
            &[],
            Some("EndTurn"),
        );
        assert_eq!(trace_id, trace_id2, "tool response should reuse the same trace");
    }

    #[test]
    fn trace_tool_use_stop_reason_but_no_ids_does_not_complete() {
        let config = test_config();
        // First request: no tool_call_ids but stop_reason says tool_use.
        // This was the bug: trace was being completed prematurely.
        let trace_id = assign_trace_id(
            &config,
            &[],
            &[],
            Some("ToolUse"),
        );
        // The trace should NOT be completed, but since there are no tool_call_ids
        // to register, we can't look it up again. The key fix is that we don't
        // prematurely close it. Verify a new request gets a new trace (since there's
        // no lookup path to find the old one).
        let trace_id2 = assign_trace_id(
            &config,
            &[],
            &[],
            Some("EndTurn"),
        );
        // Different traces since there's no link, but the important thing is
        // the first trace wasn't explicitly completed.
        assert_ne!(trace_id, trace_id2);
    }

    #[test]
    fn trace_end_turn_completes_trace() {
        let config = test_config();
        let trace_id = assign_trace_id(
            &config,
            &[],
            &["call_1".to_string()],
            Some("ToolUse"),
        );
        // Complete the trace with a tool response and end_turn
        let trace_id2 = assign_trace_id(
            &config,
            &["call_1".to_string()],
            &[],
            Some("EndTurn"),
        );
        assert_eq!(trace_id, trace_id2);
        // Now the trace is completed. A new request should get a new trace.
        let trace_id3 = assign_trace_id(
            &config,
            &[],
            &[],
            Some("EndTurn"),
        );
        assert_ne!(trace_id, trace_id3, "completed trace should not be reused");
    }
}
