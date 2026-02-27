/// Gateway HTTP server: axum router that proxies LLM API requests to upstream
/// providers with key injection, SSE stream forwarding, and audit logging.
///
/// Runs on vsock:5004 in production (plain HTTP from the VM) or on a TCP
/// socket for standalone testing.
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;
use futures::StreamExt;
use tracing::{info, warn};

use super::audit::{GatewayDb, GatewayEvent};
use super::provider::route_provider;
use super::streaming::{StreamAccumulator, drain_accumulated};
use super::GatewayConfig;

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
            record_error(Arc::clone(&config.audit_db), kind.as_str(), &method, &path, &msg);
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
            record_error(Arc::clone(&config.audit_db), kind.as_str(), &method, &path, &msg);
            return (StatusCode::BAD_REQUEST, msg).into_response();
        }
    };

    let request_bytes = body_bytes.len() as u64;

    // Capture request body preview for audit.
    let request_body_preview = if body_bytes.is_empty() {
        None
    } else {
        let limit = MAX_REQUEST_CAPTURE.min(body_bytes.len());
        Some(String::from_utf8_lossy(&body_bytes[..limit]).into_owned())
    };

    // Try to extract model name from request body JSON.
    let model = extract_model(&body_bytes);

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
            record_gateway_event(
                Arc::clone(&config.audit_db),
                kind.as_str(),
                model.as_deref(),
                &method,
                &path,
                502,
                duration_ms,
                request_bytes,
                0,
                false,
                request_body_preview.as_deref(),
                None,
                Some(&msg),
            );
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
        // Streaming SSE: wrap in accumulator and forward.
        let byte_stream = upstream_resp.bytes_stream();
        let accumulator = StreamAccumulator::new(byte_stream, MAX_RESPONSE_CAPTURE);
        let accumulated_handle = accumulator.accumulated();
        let bytes_handle = accumulator.bytes_count();

        // Map the stream to axum-compatible types.
        let mapped = accumulator.map(|result: Result<axum::body::Bytes, reqwest::Error>| {
            result.map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })
        });
        let body = Body::from_stream(mapped);

        let response = response_builder.body(body).unwrap_or_else(|_| {
            Response::builder()
                .status(502)
                .body(Body::from("internal error"))
                .unwrap()
        });

        // Spawn a task to record audit after response is fully streamed.
        // We need the accumulated data which is only available after the stream ends.
        let audit_db = Arc::clone(&config.audit_db);
        let provider_str = kind.as_str().to_string();
        let model_clone = model.clone();
        let method_clone = method.clone();
        let path_clone = path.clone();
        let req_preview = request_body_preview.clone();
        tokio::spawn(async move {
            // Wait a bit for the stream to likely finish, then check.
            // The actual recording happens when we can lock the accumulated data.
            // Since the stream is being consumed by axum/hyper, we just need to
            // wait until it's done. We poll periodically.
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            loop {
                // Check if the stream is done by seeing if the byte count stabilized.
                let current = bytes_handle.load(Ordering::Relaxed);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                let after = bytes_handle.load(Ordering::Relaxed);
                if current == after {
                    break;
                }
            }
            let duration_ms = start.elapsed().as_millis() as u64;
            let response_bytes = bytes_handle.load(Ordering::Relaxed);
            let response_preview = drain_accumulated(&accumulated_handle);
            record_gateway_event(
                Arc::clone(&audit_db),
                &provider_str,
                model_clone.as_deref(),
                &method_clone,
                &path_clone,
                status_u16,
                duration_ms,
                request_bytes,
                response_bytes,
                true,
                req_preview.as_deref(),
                response_preview.as_deref(),
                None,
            );
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
                record_gateway_event(
                    Arc::clone(&config.audit_db),
                    kind.as_str(),
                    model.as_deref(),
                    &method,
                    &path,
                    status_u16,
                    duration_ms,
                    request_bytes,
                    0,
                    false,
                    request_body_preview.as_deref(),
                    None,
                    Some(&msg),
                );
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

        record_gateway_event(
            Arc::clone(&config.audit_db),
            kind.as_str(),
            model.as_deref(),
            &method,
            &path,
            status_u16,
            duration_ms,
            request_bytes,
            response_bytes,
            false,
            request_body_preview.as_deref(),
            response_preview.as_deref(),
            None,
        );

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

#[derive(serde::Deserialize)]
struct ExtractModelPayload {
    model: Option<String>,
}

/// Extract the "model" field from a JSON request body.
/// Works for Anthropic (top-level "model") and OpenAI (top-level "model").
/// For Gemini, the model is in the URL path so this may return None.
fn extract_model(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    // Fast structural parse: Serde ignores unmapped fields (like huge image payloads)
    // without allocating memory for them, unlike `serde_json::Value`.
    serde_json::from_slice::<ExtractModelPayload>(body)
        .ok()
        .and_then(|p| p.model)
}

/// Record an error event to the audit DB.
fn record_error(db: Arc<Mutex<GatewayDb>>, provider: &str, method: &str, path: &str, error: &str) {
    let event = GatewayEvent {
        timestamp: SystemTime::now(),
        provider: provider.to_string(),
        model: None,
        method: method.to_string(),
        path: path.to_string(),
        status_code: 0,
        duration_ms: 0,
        request_bytes: 0,
        response_bytes: 0,
        streamed: false,
        request_body: None,
        response_body: None,
        error: Some(error.to_string()),
    };
    
    // SQLite writes are synchronous. Offload to a blocking thread pool
    // to prevent stalling the axum async worker thread.
    tokio::task::spawn_blocking(move || {
        if let Ok(db) = db.lock() {
            let _ = db.record(&event);
        }
    });
}

/// Record a gateway event to the audit DB.
#[allow(clippy::too_many_arguments)]
fn record_gateway_event(
    db: Arc<Mutex<GatewayDb>>,
    provider: &str,
    model: Option<&str>,
    method: &str,
    path: &str,
    status_code: u16,
    duration_ms: u64,
    request_bytes: u64,
    response_bytes: u64,
    streamed: bool,
    request_body: Option<&str>,
    response_body: Option<&str>,
    error: Option<&str>,
) {
    let event = GatewayEvent {
        timestamp: SystemTime::now(),
        provider: provider.to_string(),
        model: model.map(|s| s.to_string()),
        method: method.to_string(),
        path: path.to_string(),
        status_code,
        duration_ms,
        request_bytes,
        response_bytes,
        streamed,
        request_body: request_body.map(|s| s.to_string()),
        response_body: response_body.map(|s| s.to_string()),
        error: error.map(|s| s.to_string()),
    };
    
    // Offload synchronous SQLite write to a blocking thread.
    tokio::task::spawn_blocking(move || {
        if let Ok(db) = db.lock() {
            let _ = db.record(&event);
        }
    });
}

/// Start the gateway on a TCP socket. Returns the bound address.
/// Used for integration tests and standalone debugging.
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

    #[test]
    fn extract_model_anthropic() {
        let body = br#"{"model":"claude-sonnet-4-20250514","max_tokens":1024,"messages":[{"role":"user","content":"hi"}]}"#;
        assert_eq!(extract_model(body).as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn extract_model_openai() {
        let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
        assert_eq!(extract_model(body).as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn extract_model_empty_body() {
        assert_eq!(extract_model(b""), None);
    }

    #[test]
    fn extract_model_no_model_field() {
        let body = br#"{"messages":[{"role":"user","content":"hi"}]}"#;
        assert_eq!(extract_model(body), None);
    }

    #[test]
    fn extract_model_invalid_json() {
        assert_eq!(extract_model(b"not json"), None);
    }

    #[tokio::test]
    async fn health_endpoint() {
        let db = Arc::new(Mutex::new(GatewayDb::open_in_memory().unwrap()));
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            audit_db: db,
            http_client: reqwest::Client::new(),
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
        let db = Arc::new(Mutex::new(GatewayDb::open_in_memory().unwrap()));
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            audit_db: db,
            http_client: reqwest::Client::new(),
        });

        let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let resp = reqwest::get(format!("http://{addr}/v2/unknown")).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn missing_api_key_returns_503() {
        let db = Arc::new(Mutex::new(GatewayDb::open_in_memory().unwrap()));
        let config = Arc::new(GatewayConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            audit_db: db.clone(),
            http_client: reqwest::Client::new(),
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

        // Verify error was recorded in audit.
        let events = db.lock().unwrap().recent(10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].error.is_some());
    }
}
