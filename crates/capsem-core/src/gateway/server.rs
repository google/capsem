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

    if is_tool_use && !tool_call_ids.is_empty() {
        state.register_tool_calls(&trace_id, tool_call_ids);
    } else {
        state.complete_trace(&trace_id);
    }

    trace_id
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
                })
                .collect();

            let estimated_cost_usd = config_ref.pricing.estimate_cost(
                &provider_str,
                model_clone.as_deref(),
                summary.input_tokens,
                summary.output_tokens,
            );

            let stop_reason_str = summary.stop_reason.map(|r| format!("{:?}", r));

            // Assign trace_id using shared TraceState
            let tool_call_ids: Vec<String> = tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
            let trace_id = assign_trace_id(
                &config_ref,
                &tool_response_call_ids,
                &tool_call_ids,
                stop_reason_str.as_deref(),
            );

            let model_call = ModelCall {
                timestamp: SystemTime::now(),
                provider: provider_str.clone(),
                model: model_clone.clone(),
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
                duration_ms,
                response_bytes,
                estimated_cost_usd,
                trace_id: Some(trace_id),
                tool_calls,
                tool_responses,
            };

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
            model.as_deref(),
            None,
            None,
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
            model: model.clone(),
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
            input_tokens: None,
            output_tokens: None,
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
}
