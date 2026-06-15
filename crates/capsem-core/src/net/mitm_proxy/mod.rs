#![allow(dead_code)]
/// MITM transparent proxy: terminates TLS from the guest, inspects HTTP traffic,
/// applies per-domain read/write policy, and bridges to the real upstream server.
///
/// Connection flow:
/// 1. Read initial bytes from vsock fd (TLS ClientHello)
/// 2. TLS handshake (MitmCertResolver captures domain from SNI)
/// 3. Read HTTP request via hyper
/// 4. Policy check (domain + method -> read/write)
/// 5. If denied: return 403
/// 6. Upstream TLS to real server
/// 7. Forward request, stream response back
/// 8. Emit per-request telemetry (one NetEvent per HTTP request, not per connection)
pub mod body;
pub mod decompression_hook;
pub mod events;
mod fd_stream;
pub mod hooks;
pub mod interpreter_hook;
mod mcp_endpoint;
mod mcp_frame;
pub mod metrics;
pub mod pipeline;
pub mod protocol;
pub mod spans;
pub mod sse_parser_hook;
pub mod telemetry_hook;
mod util;

use std::io::Read;
use std::mem::ManuallyDrop;
use std::net::IpAddr;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use capsem_logger::{DbWriter, Decision, McpCall, NetEvent, WriteOp};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn, Instrument};

use crate::security_engine::{
    emit_matching_security_rules, emit_security_write, McpSecurityEvent, RuntimeSecurityEventType,
};

trait TokioReadWrite: AsyncRead + AsyncWrite {}

impl<T> TokioReadWrite for T where T: AsyncRead + AsyncWrite {}

use super::cert_authority::{CertAuthority, MitmCertResolver};
use super::policy::NetworkPolicy;
use crate::net::ai_traffic::provider::{route_provider, ModelProtocol, ProviderKind};
use crate::security_engine::{
    HttpSecurityEvent, IpSecurityEvent, ModelSecurityEvent, SecurityEvent, TcpSecurityEvent,
};
use body::{BodyStats, ProxyBoxBody, TrackedBody};
use fd_stream::{set_nonblocking, AsyncFdStream, ReplayReader};
use protocol::Protocol;
use telemetry_hook::TelemetryRequestContext;
use util::{
    format_headers, format_headers_for_domain, is_llm_api_path, parse_http_host_target,
    split_path_query,
};

pub use mcp_endpoint::{McpEndpointState, McpTimeouts};
pub use mcp_frame::dispatch_logged_mcp_request;

/// Re-exported so capsem-app can reference the type without depending on rustls.
pub type UpstreamTlsConfig = rustls::ClientConfig;

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;
const AI_BODY_PREVIEW: usize = 1024 * 1024;
const MCP_BODY_PREVIEW: usize = 64 * 1024;
const CREDENTIAL_BODY_PREVIEW: usize = 16 * 1024;

static FIRST_NETWORK_READY_EMITTED: AtomicBool = AtomicBool::new(false);

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    /// Live policy, swappable via RwLock so settings changes take effect
    /// without restarting the VM. Each HTTP request snapshots the Arc so
    /// that disabling a provider blocks the next request even on an
    /// existing keep-alive connection.
    pub policy: Arc<std::sync::RwLock<Arc<NetworkPolicy>>>,
    /// Live model endpoint registry from settings/profile provider blocks.
    /// MITM resolves host -> model protocol once per request and then passes
    /// that typed metadata to enforcement, hooks, broker substitution, and
    /// telemetry. Provider hooks must not infer protocol from domains.
    pub model_endpoints:
        Arc<std::sync::RwLock<Arc<crate::net::policy_config::ModelEndpointRegistry>>>,
    pub db: Arc<DbWriter>,
    /// Cached upstream TLS config (shared across all connections).
    pub upstream_tls: Arc<rustls::ClientConfig>,
    /// Telemetry deps shared with the `TelemetryHook` registered in
    /// `pipeline`. Held here as the same `Arc` so the hook and any
    /// remaining direct callers (rare; should fold into the hook) read
    /// the same `pricing` table + `trace_state` mutex. The Arc breaks
    /// the would-be cycle (config → pipeline → hook → config); the
    /// hook only points at this `TelemetryDeps`, not the surrounding
    /// `MitmProxyConfig`.
    pub telemetry: Arc<telemetry_hook::TelemetryDeps>,
    /// Hook pipeline. `make_production_pipeline` registers the sync
    /// ChunkHook chain (decompression → SSE parse →
    /// provider interpreters → telemetry). `handle_request` dispatches
    /// L1 events through this pipeline and seeds per-request context
    /// into the `ChunkDispatchBody`'s `HookState` before serving.
    pub pipeline: Arc<pipeline::Pipeline>,
    /// T3 framed MCP endpoint on the MITM listener. Dispatch state lives
    /// here so the low-privilege aggregator remains DB-free while MITM
    /// owns policy, timeouts, and `mcp_calls` telemetry.
    pub mcp_endpoint: Option<Arc<McpEndpointState>>,
}

/// Build the default (empty) hook pipeline. T1 slices 2 + 3 will
/// extend this to register the production hook set; until then the
/// pipeline is wired through `MitmProxyConfig` but no dispatch
/// happens from `handle_request`.
pub fn make_default_pipeline() -> Arc<pipeline::Pipeline> {
    Arc::new(pipeline::Pipeline::builder().build())
}

/// RAII helper: decrements the `mitm.active_connections` gauge when
/// `handle_connection` returns (success, error, or panic-via-unwind).
/// Held in a `let _gauge_guard = ConnectionGauge;` binding for the
/// connection's lifetime.
struct ConnectionGauge;

impl Drop for ConnectionGauge {
    fn drop(&mut self) {
        ::metrics::gauge!(metrics::ACTIVE_CONNECTIONS).decrement(1.0);
    }
}

/// Build the production hook pipeline. Registers the full sync ChunkHook chain
/// (decompression → SSE parse → provider interpreters → telemetry).
///
/// All four ChunkHook stages are pure-sync: per-chunk work runs
/// inline from `poll_frame` with no `.await`, no channel hop, no
/// async wrapper. Header mutations needed for decompression
/// (Content-Encoding / Content-Length strip) happen inline in
/// `handle_request` before chunk dispatch begins -- the chunk hooks
/// themselves never see the head.
pub fn make_production_pipeline(
    policy: Arc<std::sync::RwLock<Arc<NetworkPolicy>>>,
    telemetry: Arc<telemetry_hook::TelemetryDeps>,
) -> Arc<pipeline::Pipeline> {
    let _ = policy;
    let p = pipeline::Pipeline::builder()
        // Chunk-hook order is load-bearing:
        //   1. DecompressionHook -- gzip detection on first chunk's
        //      magic; subsequent chunks fed through flate2::Decompress.
        //   2. SseParserHook -- needs decompressed bytes for AI
        //      domains.
        //   3. Interpreter hooks -- drain SseParserHook's queue and
        //      build LlmEvents. Three providers; only the matching
        //      one runs.
        //   4. TelemetryHook -- counts response bytes, captures
        //      preview, fires NetEvent + optional ModelCall on
        //      on_response_end.
        .register_chunk(Arc::new(decompression_hook::DecompressionHook::new()))
        .register_chunk(Arc::new(sse_parser_hook::SseParserHook::new()))
        .register_chunk(Arc::new(interpreter_hook::AnthropicInterpreterHook::new()))
        .register_chunk(Arc::new(interpreter_hook::OpenAiInterpreterHook::new()))
        .register_chunk(Arc::new(interpreter_hook::GoogleInterpreterHook::new()))
        .register_chunk(Arc::new(telemetry_hook::TelemetryHook::new(telemetry)))
        .build();
    Arc::new(p)
}

fn ai_provider_for_domain(config: &MitmProxyConfig, domain: &str) -> Option<ProviderKind> {
    config
        .model_endpoints
        .read()
        .unwrap()
        .provider_for_host(domain)
}

fn ai_provider_for_target(
    config: &MitmProxyConfig,
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> Option<ProviderKind> {
    let registry = config.model_endpoints.read().unwrap();
    ai_identity_for_target_or_path(&registry, domain, upstream_port, path).provider
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModelTrafficIdentity {
    /// Endpoint owner used for policy/logging. Example: `ollama` for
    /// `127.0.0.1:11434`, even when the request path is OpenAI/Anthropic
    /// compatible.
    provider: Option<ProviderKind>,
    /// Wire protocol used to parse request/response payloads.
    protocol: Option<ModelProtocol>,
}

fn ai_identity_for_target_or_path(
    registry: &crate::net::policy_config::ModelEndpointRegistry,
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> ModelTrafficIdentity {
    let path_protocol = route_provider(path).map(|(protocol, _)| protocol);
    let endpoint_provider = registry.provider_for_target(domain, upstream_port);
    let endpoint_protocol = registry.protocol_for_target(domain, upstream_port);
    ModelTrafficIdentity {
        provider: endpoint_provider.or_else(|| path_protocol.map(|_| ProviderKind::Unknown)),
        protocol: path_protocol.or(endpoint_protocol),
    }
}

fn ai_provider_for_target_or_path(
    registry: &crate::net::policy_config::ModelEndpointRegistry,
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> Option<ProviderKind> {
    ai_identity_for_target_or_path(registry, domain, upstream_port, path).provider
}

fn ai_protocol_for_body_preview(body: &[u8]) -> Option<ModelProtocol> {
    if body.len() > AI_BODY_PREVIEW {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = json.as_object()?;
    let model = obj.get("model").and_then(|value| value.as_str());
    let has_messages = obj
        .get("messages")
        .and_then(|value| value.as_array())
        .is_some();
    let has_google_contents = obj
        .get("contents")
        .and_then(|value| value.as_array())
        .is_some()
        || obj.contains_key("generationConfig")
        || obj.contains_key("safetySettings");

    if has_google_contents || model.is_some_and(is_google_model_name) {
        return Some(ModelProtocol::Google);
    }
    if model.is_some_and(is_anthropic_model_name)
        || (has_messages && obj.contains_key("max_tokens"))
    {
        return Some(ModelProtocol::Anthropic);
    }
    if model.is_some_and(is_openai_model_name)
        || obj.contains_key("input")
        || obj.contains_key("response_format")
        || obj.contains_key("stream_options")
        || (has_messages && obj.contains_key("tools"))
    {
        return Some(ModelProtocol::OpenAi);
    }
    None
}

fn should_sniff_unknown_model_body(
    ai_provider: Option<ProviderKind>,
    method: &http::Method,
    headers: &http::HeaderMap,
) -> bool {
    if ai_provider.is_some() {
        return false;
    }
    if !matches!(
        *method,
        http::Method::POST | http::Method::PUT | http::Method::PATCH
    ) {
        return false;
    }
    let is_json = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false);
    if !is_json {
        return false;
    }
    let Some(len) = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return false;
    };
    len <= AI_BODY_PREVIEW
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedMcpHttpRequest {
    method: String,
    server_name: String,
    tool_name: Option<String>,
    request_id: Option<String>,
    request_preview: Option<String>,
    bytes_sent: u64,
}

impl ObservedMcpHttpRequest {
    fn event_type(&self) -> RuntimeSecurityEventType {
        runtime_mcp_event_type(&self.method)
    }

    fn security_event(&self, tool_list: Option<String>) -> SecurityEvent {
        SecurityEvent::new(self.event_type()).with_mcp(McpSecurityEvent {
            method: Some(self.method.clone()),
            server_name: Some(self.server_name.clone()),
            tool_call_name: self.tool_name.clone(),
            tool_list,
        })
    }
}

fn should_sniff_mcp_http_body(method: &http::Method, headers: &http::HeaderMap) -> bool {
    if !matches!(
        *method,
        http::Method::POST | http::Method::PUT | http::Method::PATCH
    ) {
        return false;
    }
    let is_json = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false);
    if !is_json {
        return false;
    }
    let Some(len) = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return false;
    };
    len <= MCP_BODY_PREVIEW
}

fn observed_mcp_http_request_for_body(
    body: &[u8],
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> Option<ObservedMcpHttpRequest> {
    if body.len() > MCP_BODY_PREVIEW {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = json.as_object()?;
    if obj.get("jsonrpc").and_then(|value| value.as_str()) != Some("2.0") {
        return None;
    }
    let method = obj.get("method").and_then(|value| value.as_str())?;
    if !is_mcp_json_rpc_method(method) {
        return None;
    }
    let request_id = obj.get("id").and_then(json_rpc_id_to_log_string);
    let params = obj.get("params").and_then(|value| value.as_object());
    let tool_name = if method == "tools/call" {
        params
            .and_then(|params| params.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    } else {
        None
    };
    Some(ObservedMcpHttpRequest {
        method: method.to_string(),
        server_name: observed_mcp_server_name(domain, upstream_port, path),
        tool_name,
        request_id,
        request_preview: Some(String::from_utf8_lossy(body).to_string()),
        bytes_sent: body.len() as u64,
    })
}

fn is_mcp_json_rpc_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "notifications/initialized"
            | "tools/list"
            | "tools/call"
            | "resources/list"
            | "resources/read"
            | "prompts/list"
            | "prompts/get"
    )
}

fn runtime_mcp_event_type(method: &str) -> RuntimeSecurityEventType {
    match method {
        "tools/call" => RuntimeSecurityEventType::McpToolCall,
        "tools/list" => RuntimeSecurityEventType::McpToolList,
        _ => RuntimeSecurityEventType::McpEvent,
    }
}

fn observed_mcp_server_name(domain: &str, upstream_port: u16, path: &str) -> String {
    format!("observed:{domain}:{upstream_port}{path}")
}

fn json_rpc_id_to_log_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

fn is_openai_model_name(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("chatgpt-")
}

fn is_anthropic_model_name(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("claude-")
}

fn is_google_model_name(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("gemini-") || model.starts_with("models/gemini-")
}

fn provider_label(provider: Option<ProviderKind>) -> &'static str {
    provider.map(|provider| provider.as_str()).unwrap_or("none")
}

fn body_preview_cap(
    ai_provider: Option<ProviderKind>,
    domain: &str,
    path: &str,
    log_bodies: bool,
    max_body: usize,
) -> usize {
    if ai_provider.is_some() {
        return AI_BODY_PREVIEW.max(max_body);
    }
    if log_bodies {
        return max_body;
    }
    if crate::credential_broker::is_http_body_credential_candidate(domain, path) {
        return CREDENTIAL_BODY_PREVIEW;
    }
    0
}

fn response_body_preview_cap(
    ai_provider: Option<ProviderKind>,
    domain: &str,
    path: &str,
    log_bodies: bool,
    max_body: usize,
    credential_ref: Option<&str>,
) -> usize {
    let cap = body_preview_cap(ai_provider, domain, path, log_bodies, max_body);
    if credential_ref.is_some() {
        cap.max(CREDENTIAL_BODY_PREVIEW)
    } else {
        cap
    }
}

#[derive(Clone, Debug, Default)]
struct SecurityBoundaryDecisionFields {
    policy_mode: Option<String>,
    policy_action: Option<String>,
    policy_rule: Option<String>,
    policy_reason: Option<String>,
}

impl SecurityBoundaryDecisionFields {
    fn from_enforcement(decision: &crate::security_engine::SecurityEnforcementDecision) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(decision.action.as_str().to_string()),
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
        }
    }

    fn matched_rule(&self, fallback: String) -> String {
        self.policy_rule.clone().unwrap_or(fallback)
    }
}

fn model_security_event(
    event_type: RuntimeSecurityEventType,
    provider: ProviderKind,
    model: Option<String>,
    request_body: Option<&[u8]>,
    response_body: Option<&[u8]>,
) -> SecurityEvent {
    SecurityEvent::new(event_type).with_model(ModelSecurityEvent {
        provider: Some(provider.as_str().to_string()),
        name: model,
        request_body: request_body.map(|body| String::from_utf8_lossy(body).to_string()),
        response_body: response_body.map(|body| String::from_utf8_lossy(body).to_string()),
        tool_calls: None,
    })
}

fn maybe_decompress_gzip_body(body: Bytes, is_gzip: bool) -> anyhow::Result<Bytes> {
    if !is_gzip {
        return Ok(body);
    }
    let mut decoder = flate2::read::GzDecoder::new(&body[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(Bytes::from(decompressed))
}

fn materialize_collected_response_headers(
    headers: &mut http::HeaderMap,
    body_len: usize,
    is_gzip: bool,
) {
    if is_gzip {
        headers.remove(http::header::CONTENT_ENCODING);
    }
    headers.remove(http::header::CONTENT_LENGTH);
    headers.remove(http::header::TRANSFER_ENCODING);
    if let Ok(value) = http::HeaderValue::from_str(&body_len.to_string()) {
        headers.insert(http::header::CONTENT_LENGTH, value);
    }
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Build the upstream TLS client config (trusts standard webpki roots).
pub fn make_upstream_tls_config() -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("TLS config")
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Arc::new(config)
}

/// Handle a single MITM proxy connection from the guest.
///
/// This is the async entry point for each vsock:5002 connection.
/// Per-request telemetry is emitted by `TelemetryHook` (a sync
/// ChunkHook) when each HTTP response body completes. This function
/// only emits connection-level error events (TLS failures, no SNI,
/// etc.).
#[tracing::instrument(
    skip_all,
    name = "capsem.mitm.connection",
    target = "capsem.mitm",
    fields(vsock_fd)
)]
pub async fn handle_connection(vsock_fd: RawFd, config: Arc<MitmProxyConfig>) {
    // The `protocol="…"` partition for `mitm.connections_total` is
    // incremented inside `handle_inner` once the first-byte sniff has
    // classified the wire payload (T2.1). Errors before classification
    // count as `protocol="unknown"`.
    ::metrics::gauge!(metrics::ACTIVE_CONNECTIONS).increment(1.0);
    let _gauge_guard = ConnectionGauge;

    let result = handle_inner(vsock_fd, &config).await;

    match result {
        Ok(domain) => {
            debug!(domain, "MITM proxy: connection closed");
        }
        Err((domain, decision, reason)) => {
            let display_domain = if domain.is_empty() {
                "<unknown>".to_string()
            } else {
                domain
            };

            let event = NetEvent {
                event_id: None,
                timestamp: SystemTime::now(),
                domain: display_domain.clone(),
                port: 443,
                decision,
                process_name: None,
                pid: None,
                bytes_sent: 0,
                bytes_received: 0,
                duration_ms: 0,
                method: None,
                path: None,
                query: None,
                status_code: None,
                matched_rule: Some(reason.clone()),
                request_headers: None,
                response_headers: None,
                request_body_preview: None,
                response_body_preview: None,
                conn_type: Some("https-mitm".to_string()),
                policy_mode: None,
                policy_action: None,
                policy_rule: None,
                policy_reason: None,
                trace_id: crate::telemetry::ambient_capsem_trace_id(),
                credential_ref: None,
            };

            crate::security_engine::emit_security_write(&config.db, WriteOp::NetEvent(event)).await;
            warn!(
                domain = display_domain,
                reason, "MITM proxy: connection error"
            );
        }
    }
}

/// Inner handler. Returns Ok(domain) on success, Err((domain, decision, reason))
/// on connection-level failure. Per-request telemetry is emitted by `TelemetryHook`.
async fn handle_inner(
    vsock_fd: RawFd,
    config: &Arc<MitmProxyConfig>,
) -> Result<String, (String, Decision, String)> {
    // Wrap vsock fd in a non-owning async stream.
    let vsock_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(vsock_fd) });
    let std_fd = vsock_file
        .try_clone()
        .map_err(|e| (String::new(), Decision::Error, format!("dup vsock fd: {e}")))?;
    set_nonblocking(vsock_fd).map_err(|e| {
        (
            String::new(),
            Decision::Error,
            format!("set nonblocking: {e}"),
        )
    })?;
    let async_fd = tokio::io::unix::AsyncFd::new(std_fd)
        .map_err(|e| (String::new(), Decision::Error, format!("async fd: {e}")))?;
    let mut vsock_stream = AsyncFdStream(async_fd);
    let classify_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_VSOCK_CLASSIFY,
        protocol = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );

    // 1. Read initial bytes (TLS ClientHello + potential metadata).
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut initial_buf)
        .instrument(classify_span.clone())
        .await
        .map_err(|e| {
            classify_span.record("status", "error");
            classify_span.record("error_kind", "read_client_hello");
            (
                String::new(),
                Decision::Error,
                format!("read ClientHello: {e}"),
            )
        })?;
    if n == 0 {
        classify_span.record("status", "error");
        classify_span.record("error_kind", "empty_connection");
        return Err((String::new(), Decision::Error, "empty connection".into()));
    }
    initial_buf.truncate(n);

    let mut process_name: Option<String> = None;
    if initial_buf.starts_with(b"\0CAPSEM_META:") {
        // Metadata may arrive fragmented across multiple reads.
        // Keep reading until we find the terminating '\n' or hit the 4KB limit.
        const MAX_META_SIZE: usize = 4096;
        loop {
            if let Some(nl_idx) = initial_buf.iter().position(|&b| b == b'\n') {
                let proc_bytes = &initial_buf[13..nl_idx];
                process_name = String::from_utf8(proc_bytes.to_vec()).ok();
                initial_buf.drain(0..=nl_idx);
                break;
            }
            if initial_buf.len() >= MAX_META_SIZE {
                return Err((
                    String::new(),
                    Decision::Error,
                    "metadata exceeded 4KB limit".into(),
                ));
            }
            let mut more = vec![0u8; 1024];
            let n2 = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut more)
                .instrument(classify_span.clone())
                .await
                .map_err(|e| {
                    classify_span.record("status", "error");
                    classify_span.record("error_kind", "read_metadata");
                    (
                        String::new(),
                        Decision::Error,
                        format!("read metadata: {e}"),
                    )
                })?;
            if n2 == 0 {
                return Err((
                    String::new(),
                    Decision::Error,
                    "EOF during metadata read".into(),
                ));
            }
            initial_buf.extend_from_slice(&more[..n2]);
        }

        // If initial_buf is empty after draining meta, we need to read
        // the wire payload (TLS ClientHello or HTTP request line).
        if initial_buf.is_empty() {
            let mut hello_buf = vec![0u8; MAX_HELLO_SIZE];
            let n2 = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut hello_buf)
                .instrument(classify_span.clone())
                .await
                .map_err(|e| {
                    classify_span.record("status", "error");
                    classify_span.record("error_kind", "read_payload_after_meta");
                    (
                        String::new(),
                        Decision::Error,
                        format!("read payload after meta: {e}"),
                    )
                })?;
            if n2 == 0 {
                return Err((
                    String::new(),
                    Decision::Error,
                    "empty connection after meta".into(),
                ));
            }
            hello_buf.truncate(n2);
            initial_buf = hello_buf;
        }
    }

    // Framed MCP starts with a four-byte length prefix followed by a
    // two-byte magic. If the guest's first write is split across the
    // hypervisor boundary, pull just enough bytes for the classifier.
    while initial_buf.first() == Some(&0) && initial_buf.len() < 6 {
        let mut more = vec![0u8; 6 - initial_buf.len()];
        let n2 = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut more)
            .instrument(classify_span.clone())
            .await
            .map_err(|e| {
                classify_span.record("status", "error");
                classify_span.record("error_kind", "read_protocol_prefix");
                (
                    String::new(),
                    Decision::Error,
                    format!("read protocol prefix: {e}"),
                )
            })?;
        if n2 == 0 {
            break;
        }
        initial_buf.extend_from_slice(&more[..n2]);
    }

    // 2. First-byte protocol sniff (T2.1). Classify the post-meta
    //    payload as TLS (0x16) or plain HTTP (uppercase ASCII method).
    //    Unrecognized first byte → connection-level error event.
    let detected = match protocol::detect(&initial_buf) {
        Some(p) => p,
        None => {
            classify_span.record("protocol", Protocol::Unknown.label());
            classify_span.record("status", "error");
            classify_span.record("error_kind", "unknown_protocol");
            ::metrics::counter!(metrics::CONNECTIONS_TOTAL,
                "protocol" => Protocol::Unknown.label())
            .increment(1);
            let first = initial_buf.first().copied().unwrap_or(0);
            return Err((
                String::new(),
                Decision::Error,
                format!("unknown protocol byte 0x{first:02x}"),
            ));
        }
    };
    ::metrics::counter!(metrics::CONNECTIONS_TOTAL,
        "protocol" => detected.label())
    .increment(1);
    classify_span.record("protocol", detected.label());
    classify_span.record("status", "ok");

    let process_name = Arc::new(process_name);

    match detected {
        Protocol::Tls => serve_tls(initial_buf, vsock_stream, config, process_name).await,
        Protocol::Http => serve_plain_http(initial_buf, vsock_stream, config, process_name).await,
        Protocol::McpFrame => {
            let Some(endpoint) = &config.mcp_endpoint else {
                return Err((
                    "mcp.capsem.internal".to_string(),
                    Decision::Error,
                    "framed MCP endpoint disabled".into(),
                ));
            };
            mcp_frame::serve(
                initial_buf,
                vsock_stream,
                Arc::clone(endpoint),
                Arc::clone(&config.db),
            )
            .await
        }
        Protocol::Unknown => unreachable!("Protocol::Unknown returned Err earlier"),
    }
}

/// TLS-terminating MITM path. Runs the rustls acceptor on the vsock
/// stream (chained with the buffered ClientHello bytes), pulls the
/// SNI domain off the resolver, and serves a hyper HTTP/1.1 server
/// over the resulting TLS stream.
async fn serve_tls(
    initial_buf: Vec<u8>,
    vsock_stream: AsyncFdStream,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) -> Result<String, (String, Decision, String)> {
    let resolver = Arc::new(MitmCertResolver::new(Arc::clone(&config.ca)));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls_config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| (String::new(), Decision::Error, format!("TLS config: {e}")))?
        .with_no_client_auth()
        .with_cert_resolver(Arc::clone(&resolver) as _);
    tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Chain buffered ClientHello bytes with the remaining vsock stream.
    let replay = ReplayReader::new(initial_buf, vsock_stream);
    let handshake_start = Instant::now();
    let tls_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_TLS_GUEST_HANDSHAKE,
        protocol = "https",
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let tls_stream = acceptor
        .accept(replay)
        .instrument(tls_span.clone())
        .await
        .map_err(|e| {
            tls_span.record("status", "error");
            tls_span.record("error_kind", "guest_tls_handshake");
            ::metrics::histogram!(metrics::TLS_HANDSHAKE_MS)
                .record(handshake_start.elapsed().as_secs_f64() * 1000.0);
            let domain = resolver.domain().unwrap_or_default();
            (domain, Decision::Error, format!("TLS handshake: {e}"))
        })?;
    tls_span.record("status", "ok");
    ::metrics::histogram!(metrics::TLS_HANDSHAKE_MS)
        .record(handshake_start.elapsed().as_secs_f64() * 1000.0);

    let domain = resolver.domain().ok_or_else(|| {
        (
            String::new(),
            Decision::Denied,
            "no SNI in ClientHello".into(),
        )
    })?;

    let io = TokioIo::new(tls_stream);
    serve_pipeline(io, domain.clone(), Protocol::Tls, config, process_name).await;
    Ok(domain)
}

/// Plain-HTTP MITM path (T2.2). Skips TLS termination entirely and
/// runs the hyper HTTP/1.1 server directly on the vsock stream
/// (`ReplayReader` carries the buffered first bytes plus the rest of
/// the stream). Per-request domain + upstream port are derived from
/// the inbound `Host` header inside the service closure.
async fn serve_plain_http(
    initial_buf: Vec<u8>,
    vsock_stream: AsyncFdStream,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) -> Result<String, (String, Decision, String)> {
    let replay = ReplayReader::new(initial_buf, vsock_stream);
    let io = TokioIo::new(replay);
    serve_pipeline(io, String::new(), Protocol::Http, config, process_name).await;
    // Per-request telemetry is emitted by `TelemetryHook`. The
    // connection-level `NetEvent` `handle_connection` would write on
    // an Err-return is intentionally skipped on this path -- there
    // is no connection-level domain to attribute (each request can
    // carry a different Host header).
    Ok(String::new())
}

/// Drive the hyper HTTP/1.1 server over the supplied IO. The service
/// closure resolves the per-request `(domain, upstream_port)`:
/// * TLS: connection-level SNI domain + 443 (constant per connection).
/// * HTTP: parsed from the inbound `Host` header per request; falls
///   back to `("", 80)` when the header is missing or malformed,
///   producing a 502 downstream once `handle_request` runs.
async fn serve_pipeline<IO>(
    io: IO,
    connection_domain: String,
    protocol: Protocol,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) where
    IO: hyper::rt::Read + hyper::rt::Write + Send + Unpin + 'static,
{
    let upstream_tls = Arc::clone(&config.upstream_tls);
    let config_arc = Arc::clone(config);

    // Per-connection upstream sender cache: each MITM connection
    // serves one upstream via keep-alive, so caching the sender
    // avoids re-establishing TCP[+TLS] for every request on the
    // same connection.
    let cached_upstream: Arc<
        tokio::sync::Mutex<Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>>,
    > = Arc::new(tokio::sync::Mutex::new(None));

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let upstream_tls = Arc::clone(&upstream_tls);
        let connection_domain = connection_domain.clone();
        let config_arc = Arc::clone(&config_arc);
        let process_name = Arc::clone(&process_name);
        let cached_upstream = Arc::clone(&cached_upstream);

        async move {
            // Resolve the per-request `(domain, upstream_port)`. TLS
            // already knows from SNI; HTTP must read the Host header.
            let (request_domain, upstream_port) = match protocol {
                Protocol::Tls => (connection_domain, 443u16),
                Protocol::Http => parse_http_host_target(req.headers().get("host"))
                    .unwrap_or_else(|| (String::new(), 80)),
                Protocol::McpFrame => unreachable!("framed MCP bypasses HTTP pipeline"),
                Protocol::Unknown => (String::new(), 0),
            };
            let ai_identity = {
                let registry = config_arc.model_endpoints.read().unwrap();
                ai_identity_for_target_or_path(
                    &registry,
                    &request_domain,
                    upstream_port,
                    req.uri().path(),
                )
            };
            handle_request(
                req,
                &request_domain,
                protocol,
                upstream_port,
                &upstream_tls,
                &config_arc,
                &process_name,
                ai_identity.provider,
                ai_identity.protocol,
                &cached_upstream,
            )
            .await
        }
    });

    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .with_upgrades()
        .await
    {
        // Connection errors are expected when the guest closes.
        let err_str = e.to_string();
        if !e.is_incomplete_message() && !err_str.contains("error shutting down connection") {
            warn!(error = %e, "hyper serve error");
        }
    }
}

fn http_request_security_event(
    domain: &str,
    upstream_port: u16,
    method: &str,
    path: &str,
    query: Option<String>,
    ai_provider: Option<ProviderKind>,
    headers: http::HeaderMap,
    body: Option<&Bytes>,
) -> SecurityEvent {
    let body = body.and_then(|body| std::str::from_utf8(body).ok().map(ToOwned::to_owned));
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some(domain.to_string()),
            method: Some(method.to_string()),
            path: Some(path.to_string()),
            query: query.clone(),
            status: None,
            body,
        })
        .with_http_request(crate::security_engine::HttpRequestSecurityEvent::new(
            domain,
            ai_provider,
            headers,
            query,
        ));
    security_event_with_transport(event, domain, upstream_port)
}

fn security_event_with_transport(
    mut event: SecurityEvent,
    domain: &str,
    upstream_port: u16,
) -> SecurityEvent {
    event = event.with_tcp(TcpSecurityEvent {
        port: Some(upstream_port.to_string()),
    });
    if let Ok(ip) = domain.parse::<IpAddr>() {
        event = event.with_ip(IpSecurityEvent {
            value: Some(ip.to_string()),
            version: Some(match ip {
                IpAddr::V4(_) => "4".to_string(),
                IpAddr::V6(_) => "6".to_string(),
            }),
        });
    }

    event
}

/// Handle a single HTTP request within a MITM-proxied connection
/// (TLS or plain HTTP).
///
/// Reads the live policy from `config.policy` RwLock per-request so that
/// settings changes (e.g. disabling a provider) take effect immediately,
/// even for in-flight keep-alive connections.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    name = "capsem.mitm.request",
    target = "capsem.mitm",
    fields(
        protocol = protocol.label(),
        provider = provider_label(ai_provider),
        method = tracing::field::Empty,
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
    )
)]
async fn handle_request(
    mut req: hyper::Request<hyper::body::Incoming>,
    domain: &str,
    protocol: Protocol,
    upstream_port: u16,
    upstream_tls: &Arc<rustls::ClientConfig>,
    config: &Arc<MitmProxyConfig>,
    process_name: &Option<String>,
    ai_provider: Option<ProviderKind>,
    ai_protocol: Option<ModelProtocol>,
    cached_upstream: &tokio::sync::Mutex<
        Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>,
    >,
) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error> {
    use http_body_util::BodyExt;

    let is_upgrade = req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let client_upgrade = if is_upgrade {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };

    // Snapshot the live policy for this request (not per-connection) so that
    // hot-reloaded settings take effect for subsequent requests on the same
    // keep-alive connection.
    let policy: Arc<NetworkPolicy> = config.policy.read().unwrap().clone();
    let log_bodies = policy.log_bodies;
    let max_body = policy.max_body_capture;

    // `conn_type` for telemetry. Derived from protocol; landed in
    // every TelemetryRequestContext below.
    let conn_type: &'static str = match protocol {
        Protocol::Tls => "https-mitm",
        Protocol::Http => "http-mitm",
        Protocol::McpFrame => "mcp-frame",
        Protocol::Unknown => "unknown-mitm",
    };

    let start_time = Instant::now();
    let (parts, req_body) = req.into_parts();
    let initial_method = parts.method.to_string();

    // Span fields for the #[instrument] decoration -- sets method
    // on the span. decision + status are filled later as we learn them.
    {
        let span = tracing::Span::current();
        span.record("method", initial_method.as_str());
    }
    if FIRST_NETWORK_READY_EMITTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let first_network_span = tracing::info_span!(
            target: "capsem.launch",
            crate::telemetry::LAUNCH_FIRST_NETWORK_READY_SPAN,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
            status = "ok",
        );
        first_network_span.in_scope(|| {
            tracing::info!(
                target: "capsem.launch",
                protocol = protocol.label(),
                provider = provider_label(ai_provider),
                "first network request reached MITM"
            );
        });
    }

    let method = parts.method.to_string();
    let (path, query) = split_path_query(&parts.uri);
    let formatted_req_headers = format_headers_for_domain(domain, ai_provider, &parts.headers);
    let req_hdrs = formatted_req_headers.formatted;
    let credential_observations = formatted_req_headers.observations;
    let credential_ref = formatted_req_headers.credential_ref;
    let mut credential_injections = Vec::new();
    let mut request_security_decision = SecurityBoundaryDecisionFields::default();
    let matched_rule = "security.http.default".to_string();

    tracing::Span::current().record("decision", "allow");
    ::metrics::counter!(metrics::REQUESTS_TOTAL,
        "protocol" => protocol.label(), "decision" => "allow")
    .increment(1);

    // Helper: wrap an already-built response body in
    // `ChunkDispatchBody` seeded with the per-request
    // `TelemetryRequestContext`, so the registered `TelemetryHook`
    // fires `NetEvent` (+ `ModelCall`) on body completion. Used by
    // every response path that doesn't reach upstream (deny,
    // websocket-deny, 502).
    let seal_with_telemetry = |inner: ProxyBoxBody,
                               req_ctx: TelemetryRequestContext,
                               conn_ai_provider: Option<ProviderKind>,
                               conn_ai_protocol: Option<ModelProtocol>|
     -> ProxyBoxBody {
        let dispatched = body::ChunkDispatchBody::new(
            inner,
            Arc::clone(&config.pipeline),
            hooks::ConnMeta {
                domain: domain.to_string(),
                process_name: process_name.clone(),
                port: upstream_port,
                protocol,
                ai_provider: conn_ai_provider,
                ai_protocol: conn_ai_protocol,
            },
            crate::telemetry::ambient_capsem_trace_id(),
        )
        .seed::<Option<TelemetryRequestContext>>(Some(req_ctx));
        dispatched.boxed()
    };

    if is_upgrade {
        let original_headers = parts.headers.clone();
        let original_method = parts.method.clone();
        let client_upgrade = client_upgrade.expect("websocket upgrade captured before split");

        let ws_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_WEBSOCKET,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
            decision = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let make_ws_error = |error: &dyn std::fmt::Display| -> hyper::Response<ProxyBoxBody> {
            let body_text = format!("Capsem: websocket upstream error ({error})\n");
            let req_ctx = TelemetryRequestContext {
                domain: domain.to_string(),
                process_name: process_name.clone(),
                ai_provider,
                ai_protocol,
                model_traffic: false,
                method: method.clone(),
                path: path.clone(),
                query: query.clone(),
                status_code: Some(502),
                decision: Decision::Denied,
                matched_rule: Some(matched_rule.clone()),
                request_headers: Some(req_hdrs.clone()),
                response_headers: None,
                start_time,
                request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
                max_response_preview: 0,
                port: upstream_port,
                conn_type,
                policy_mode: request_security_decision.policy_mode.clone(),
                policy_action: request_security_decision.policy_action.clone(),
                policy_rule: request_security_decision.policy_rule.clone(),
                policy_reason: request_security_decision.policy_reason.clone(),
                credential_ref: credential_ref.clone(),
                credential_observations: credential_observations.clone(),
                credential_injections: Vec::new(),
            };
            let body = Full::new(Bytes::from(body_text))
                .map_err(|never| match never {})
                .boxed();
            hyper::Response::builder()
                .status(http::StatusCode::BAD_GATEWAY)
                .body(seal_with_telemetry(body, req_ctx, ai_provider, ai_protocol))
                .unwrap()
        };

        let dial_target = format!("{domain}:{upstream_port}");
        let upstream_tcp = match tokio::net::TcpStream::connect(&dial_target)
            .instrument(ws_span.clone())
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                ws_span.record("decision", "error");
                ws_span.record("status", "error");
                ws_span.record("error_kind", "upstream_tcp_connect");
                return Ok(make_ws_error(&error));
            }
        };

        let upstream_io: TokioIo<Box<dyn TokioReadWrite + Unpin + Send>> = match protocol {
            Protocol::Tls => {
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));
                let server_name = match rustls::pki_types::ServerName::try_from(domain.to_string())
                {
                    Ok(sn) => sn,
                    Err(error) => {
                        ws_span.record("decision", "error");
                        ws_span.record("status", "error");
                        ws_span.record("error_kind", "upstream_server_name");
                        return Ok(make_ws_error(&error));
                    }
                };
                match connector.connect(server_name, upstream_tcp).await {
                    Ok(tls) => {
                        TokioIo::new(Box::new(tls) as Box<dyn TokioReadWrite + Unpin + Send>)
                    }
                    Err(error) => {
                        ws_span.record("decision", "error");
                        ws_span.record("status", "error");
                        ws_span.record("error_kind", "upstream_tls_handshake");
                        return Ok(make_ws_error(&error));
                    }
                }
            }
            Protocol::Http => {
                TokioIo::new(Box::new(upstream_tcp) as Box<dyn TokioReadWrite + Unpin + Send>)
            }
            Protocol::McpFrame => unreachable!("framed MCP bypasses HTTP upstream dial"),
            Protocol::Unknown => unreachable!("handle_inner gates Unknown earlier"),
        };

        let (mut sender, conn) = match hyper::client::conn::http1::handshake(upstream_io)
            .instrument(ws_span.clone())
            .await
        {
            Ok(pair) => pair,
            Err(error) => {
                ws_span.record("decision", "error");
                ws_span.record("status", "error");
                ws_span.record("error_kind", "upstream_http_handshake");
                return Ok(make_ws_error(&error));
            }
        };
        tokio::spawn(async move {
            let _ = conn.with_upgrades().await;
        });

        let full_path = match &query {
            Some(q) => format!("{path}?{q}"),
            None => path.clone(),
        };
        let mut builder = hyper::Request::builder()
            .method(original_method)
            .uri(&full_path);
        for (name, value) in original_headers.iter() {
            let drop_host = matches!(protocol, Protocol::Tls) && name == "host";
            if drop_host {
                continue;
            }
            builder = builder.header(name.clone(), value.clone());
        }
        if matches!(protocol, Protocol::Tls) {
            builder = builder.header("host", domain);
        }
        let upstream_req = builder.body(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )?;

        let mut upstream_resp = match sender
            .send_request(upstream_req)
            .instrument(ws_span.clone())
            .await
        {
            Ok(response) => response,
            Err(error) => {
                ws_span.record("decision", "error");
                ws_span.record("status", "error");
                ws_span.record("error_kind", "upstream_send_request");
                return Ok(make_ws_error(&error));
            }
        };
        let status_code = upstream_resp.status().as_u16();
        let upstream_upgrade = if upstream_resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
            Some(hyper::upgrade::on(&mut upstream_resp))
        } else {
            None
        };
        let (resp_parts, _resp_body) = upstream_resp.into_parts();
        if let Some(upstream_upgrade) = upstream_upgrade {
            let tunnel_span = ws_span.clone();
            tokio::spawn(async move {
                let result = async move {
                    let mut client = TokioIo::new(client_upgrade.await?);
                    let mut upstream = TokioIo::new(upstream_upgrade.await?);
                    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
                    Ok::<(), anyhow::Error>(())
                }
                .instrument(tunnel_span.clone())
                .await;
                match result {
                    Ok(()) => {
                        tunnel_span.record("decision", "allow");
                        tunnel_span.record("status", "ok");
                    }
                    Err(error) => {
                        tunnel_span.record("decision", "error");
                        tunnel_span.record("status", "error");
                        tunnel_span.record("error_kind", "websocket_tunnel");
                        warn!(error = %error, "websocket tunnel ended with error");
                    }
                }
            });
        }

        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(status_code),
            decision: Decision::Allowed,
            matched_rule: Some(matched_rule.clone()),
            request_headers: Some(req_hdrs),
            response_headers: Some(format_headers(&resp_parts.headers)),
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            policy_mode: request_security_decision.policy_mode.clone(),
            policy_action: request_security_decision.policy_action.clone(),
            policy_rule: request_security_decision.policy_rule.clone(),
            policy_reason: request_security_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: credential_injections.clone(),
        };

        let empty_body = Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed();

        return Ok(hyper::Response::from_parts(
            resp_parts,
            seal_with_telemetry(empty_body, req_ctx, ai_provider, ai_protocol),
        ));
    }

    // Save original request headers.
    let mut original_headers = parts.headers.clone();
    let original_method = parts.method.clone();

    // Helper: build a 502 Bad Gateway response with telemetry so upstream
    // errors don't kill keep-alive connections (returns Ok, not Err).
    let make_502 = |error: &dyn std::fmt::Display,
                    method: &str,
                    path: &str,
                    query: &Option<String>,
                    req_hdrs: &str,
                    start: Instant,
                    policy_fields: &SecurityBoundaryDecisionFields|
     -> hyper::Response<ProxyBoxBody> {
        warn!(domain, method, path, error = %error, "MITM proxy: upstream error");
        let body_text = format!("Capsem: upstream error ({error})\n");
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.to_string(),
            path: path.to_string(),
            query: query.clone(),
            status_code: Some(502),
            decision: Decision::Error,
            matched_rule: Some(error.to_string()),
            request_headers: Some(req_hdrs.to_string()),
            response_headers: None,
            start_time: start,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            policy_mode: policy_fields.policy_mode.clone(),
            policy_action: policy_fields.policy_action.clone(),
            policy_rule: policy_fields.policy_rule.clone(),
            policy_reason: policy_fields.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: Vec::new(),
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::builder()
            .status(502)
            .body(seal_with_telemetry(
                deny_body,
                req_ctx,
                ai_provider,
                ai_protocol,
            ))
            .unwrap()
    };

    enum RequestBodySource {
        Incoming(hyper::body::Incoming),
        Collected(Bytes),
    }

    fn collected_request_body_stats(
        request_body_source: &RequestBodySource,
        max_preview: usize,
    ) -> Arc<Mutex<BodyStats>> {
        let mut stats = BodyStats::new(max_preview);
        if let RequestBodySource::Collected(body) = request_body_source {
            stats.bytes = body.len() as u64;
            let to_copy = max_preview.min(body.len());
            stats.preview.extend_from_slice(&body[..to_copy]);
        }
        Arc::new(Mutex::new(stats))
    }

    let mut effective_ai_provider = ai_provider;
    let mut effective_ai_protocol = ai_protocol;
    let mut sniffed_model_request = false;
    let mut observed_mcp_request: Option<ObservedMcpHttpRequest> = None;
    let mut mcp_request_security_decision = SecurityBoundaryDecisionFields::default();
    let mut request_body_source = RequestBodySource::Incoming(req_body);
    let should_sniff_model =
        should_sniff_unknown_model_body(effective_ai_provider, &original_method, &original_headers);
    let should_sniff_mcp = should_sniff_mcp_http_body(&original_method, &original_headers);
    if should_sniff_model || should_sniff_mcp {
        let sniff_span = tracing::debug_span!(
            target: "capsem.mitm",
            "mitm_unknown_semantic_body_sniff",
            protocol = protocol.label(),
            host = domain,
            path = path.as_str(),
            provider = tracing::field::Empty,
            mcp_method = tracing::field::Empty,
            status = tracing::field::Empty,
        );
        if let RequestBodySource::Incoming(body) = request_body_source {
            let preview_limit = if should_sniff_model {
                AI_BODY_PREVIEW.max(MCP_BODY_PREVIEW)
            } else {
                MCP_BODY_PREVIEW
            };
            let collected = match http_body_util::Limited::new(body, preview_limit)
                .collect()
                .instrument(sniff_span.clone())
                .await
            {
                Ok(collected) => collected,
                Err(error) => {
                    sniff_span.record("status", "error");
                    return Ok(make_502(
                        &error,
                        &method,
                        &path,
                        &query,
                        &req_hdrs,
                        start_time,
                        &request_security_decision,
                    ));
                }
            };
            let body_bytes = collected.to_bytes();
            let mut sniff_matched = false;
            if should_sniff_model {
                if let Some(protocol) = ai_protocol_for_body_preview(&body_bytes) {
                    if effective_ai_provider.is_none() {
                        effective_ai_provider = Some(ProviderKind::Unknown);
                    }
                    effective_ai_protocol = Some(protocol);
                    sniffed_model_request = true;
                    sniff_matched = true;
                    sniff_span.record("provider", provider_label(effective_ai_provider));
                    tracing::info!(
                        target: "capsem.mitm",
                        host = domain,
                        path,
                        provider = provider_label(effective_ai_provider),
                        protocol = protocol.as_str(),
                        body_bytes = body_bytes.len(),
                        "unknown model endpoint promoted from bounded body shape"
                    );
                }
            }
            if should_sniff_mcp {
                if let Some(observed) =
                    observed_mcp_http_request_for_body(&body_bytes, domain, upstream_port, &path)
                {
                    sniff_matched = true;
                    sniff_span.record("mcp_method", observed.method.as_str());
                    tracing::info!(
                        target: "capsem.mitm",
                        host = domain,
                        path,
                        mcp_method = observed.method.as_str(),
                        mcp_server = observed.server_name.as_str(),
                        mcp_tool = observed.tool_name.as_deref(),
                        body_bytes = body_bytes.len(),
                        "unknown MCP-over-HTTP endpoint promoted from bounded JSON-RPC shape"
                    );
                    observed_mcp_request = Some(observed);
                }
            }
            if sniff_matched {
                sniff_span.record("status", "ok");
            } else {
                sniff_span.record("status", "no_match");
            }
            request_body_source = RequestBodySource::Collected(body_bytes);
        }
    }

    let mut http_security_event = http_request_security_event(
        domain,
        upstream_port,
        &method,
        &path,
        query.clone(),
        effective_ai_provider,
        original_headers.clone(),
        match &request_body_source {
            RequestBodySource::Collected(body) => Some(body),
            RequestBodySource::Incoming(_) => None,
        },
    );
    if let Some(trace_id) = crate::telemetry::ambient_capsem_trace_id() {
        http_security_event = http_security_event.with_trace_id(trace_id);
    }
    let rules = config.telemetry.security_rules.read().unwrap().clone();
    let actions_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_SECURITY_ACTIONS,
        protocol = protocol.label(),
        provider = provider_label(ai_provider),
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let http_evaluation = match actions_span.in_scope(|| {
        crate::security_engine::evaluate_security_boundary(
            &rules,
            config.telemetry.plugin_policy.read().unwrap().clone(),
            http_security_event,
        )
    }) {
        Ok(evaluation) => evaluation,
        Err(error) => {
            actions_span.record("decision", "error");
            actions_span.record("status", "error");
            actions_span.record("error_kind", "security_actions");
            return Ok(make_502(
                &error,
                &method,
                &path,
                &query,
                &req_hdrs,
                start_time,
                &request_security_decision,
            ));
        }
    };
    let credential_observations = {
        let mut observations = credential_observations.clone();
        observations.extend(http_evaluation.event.credential_observations.clone());
        observations
    };
    credential_injections = http_evaluation.event.credential_injections.clone();
    request_security_decision =
        SecurityBoundaryDecisionFields::from_enforcement(&http_evaluation.enforcement);
    if !http_evaluation.enforcement.is_allowed() {
        actions_span.record("decision", http_evaluation.enforcement.action.as_str());
        actions_span.record("status", "ok");
        let rule_id = http_evaluation
            .enforcement
            .rule_id
            .as_deref()
            .unwrap_or("unknown");
        let body_text = if matches!(
            http_evaluation.enforcement.action,
            crate::security_engine::SecurityEnforcementAction::Ask
        ) {
            format!("capsem: HTTP request requires approval by security rule: {rule_id}\n")
        } else {
            format!("capsem: HTTP request blocked by security rule: {rule_id}\n")
        };
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(403),
            decision: Decision::Denied,
            matched_rule: http_evaluation.enforcement.rule_id.clone(),
            request_headers: Some(req_hdrs.clone()),
            response_headers: None,
            start_time,
            request_body_stats: collected_request_body_stats(&request_body_source, max_body),
            max_response_preview: max_body,
            port: upstream_port,
            conn_type,
            policy_mode: request_security_decision.policy_mode.clone(),
            policy_action: request_security_decision.policy_action.clone(),
            policy_rule: request_security_decision.policy_rule.clone(),
            policy_reason: request_security_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: credential_injections.clone(),
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        return Ok(hyper::Response::builder()
            .status(403)
            .body(seal_with_telemetry(
                deny_body,
                req_ctx,
                ai_provider,
                ai_protocol,
            ))
            .unwrap());
    }
    actions_span.record("decision", "allow");
    actions_span.record("status", "ok");
    let upstream_materialized = match actions_span.in_scope(|| {
        crate::security_engine::materialize_http_request_for_upstream(&http_evaluation.event)
    }) {
        Ok(materialized) => materialized,
        Err(error) => {
            actions_span.record("decision", "error");
            actions_span.record("status", "error");
            actions_span.record("error_kind", "materialize_http_request");
            return Ok(make_502(
                &anyhow::anyhow!(error),
                &method,
                &path,
                &query,
                &req_hdrs,
                start_time,
                &request_security_decision,
            ));
        }
    };
    original_headers = upstream_materialized.headers;
    let credential_ref = credential_ref
        .clone()
        .or_else(|| upstream_materialized.credential_ref.clone());
    let upstream_query = upstream_materialized.query.as_ref().or(query.as_ref());

    // T2.2: enforce the HTTP upstream-port allowlist. The policy
    // hook ran above with `domain` already set; the port comes from
    // the inbound `Host` header (or default 80) and is not yet
    // policy-checked. The default allowlist mirrors guest iptables:
    // 80, 3128, 3713, 8080, and 11434. The TLS path always uses
    // 443, which is implicit and not gated here.
    if protocol == Protocol::Http && !policy.http_upstream_ports.contains(&upstream_port) {
        ::metrics::counter!(metrics::REQUESTS_TOTAL,
            "protocol" => protocol.label(), "decision" => "deny")
        .increment(1);
        let body_text =
            format!("Capsem: HTTP upstream port {upstream_port} not in allowlist for {domain}\n");
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(403),
            decision: Decision::Denied,
            matched_rule: Some(format!("http-port-not-allowlisted({upstream_port})")),
            request_headers: Some(req_hdrs.clone()),
            response_headers: None,
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            policy_mode: request_security_decision.policy_mode.clone(),
            policy_action: request_security_decision.policy_action.clone(),
            policy_rule: request_security_decision.policy_rule.clone(),
            policy_reason: request_security_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: credential_injections.clone(),
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        return Ok(hyper::Response::builder()
            .status(403)
            .body(seal_with_telemetry(
                deny_body,
                req_ctx,
                ai_provider,
                ai_protocol,
            ))
            .unwrap());
    }

    if let Some(observed) = observed_mcp_request.as_ref() {
        let mcp_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_SECURITY_ACTIONS,
            protocol = protocol.label(),
            mcp_method = observed.method.as_str(),
            mcp_server = observed.server_name.as_str(),
            decision = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let mcp_event = observed.security_event(None).with_http(HttpSecurityEvent {
            host: Some(domain.to_string()),
            method: Some(method.clone()),
            path: Some(path.clone()),
            query: query.clone(),
            status: None,
            body: observed.request_preview.clone(),
        });
        let mcp_evaluation = match mcp_span.in_scope(|| {
            crate::security_engine::evaluate_security_boundary(
                &rules,
                config.telemetry.plugin_policy.read().unwrap().clone(),
                mcp_event,
            )
        }) {
            Ok(evaluation) => evaluation,
            Err(error) => {
                mcp_span.record("decision", "error");
                mcp_span.record("status", "error");
                mcp_span.record("error_kind", "security_actions");
                return Ok(make_502(
                    &error,
                    &method,
                    &path,
                    &query,
                    &req_hdrs,
                    start_time,
                    &request_security_decision,
                ));
            }
        };
        mcp_request_security_decision =
            SecurityBoundaryDecisionFields::from_enforcement(&mcp_evaluation.enforcement);
        if !mcp_evaluation.enforcement.is_allowed() {
            mcp_span.record("decision", mcp_evaluation.enforcement.action.as_str());
            mcp_span.record("status", "ok");
            request_security_decision = mcp_request_security_decision.clone();
            let body_text = format!(
                "capsem: MCP request blocked by security rule: {}\n",
                mcp_evaluation
                    .enforcement
                    .rule_id
                    .as_deref()
                    .unwrap_or("unknown")
            );
            let security_event = observed.security_event(None);
            let denied_call = McpCall {
                event_id: None,
                timestamp: SystemTime::now(),
                server_name: observed.server_name.clone(),
                method: observed.method.clone(),
                tool_name: observed.tool_name.clone(),
                request_id: observed.request_id.clone(),
                request_preview: observed.request_preview.clone(),
                response_preview: Some(body_text.clone()),
                decision: "denied".to_string(),
                duration_ms: start_time.elapsed().as_millis() as u64,
                error_message: Some(body_text.trim().to_string()),
                process_name: process_name.clone(),
                bytes_sent: observed.bytes_sent,
                bytes_received: body_text.len() as u64,
                policy_mode: request_security_decision.policy_mode.clone(),
                policy_action: request_security_decision.policy_action.clone(),
                policy_rule: request_security_decision.policy_rule.clone(),
                policy_reason: request_security_decision.policy_reason.clone(),
                trace_id: crate::telemetry::ambient_capsem_trace_id(),
                credential_ref: credential_ref.clone(),
            };
            if let Some(event_id) =
                emit_security_write(&config.db, WriteOp::McpCall(denied_call)).await
            {
                if let Err(error) = emit_matching_security_rules(
                    &config.db,
                    event_id,
                    observed.event_type(),
                    &rules,
                    &security_event,
                    current_unix_ms(),
                )
                .await
                {
                    warn!(error = %error, "failed to emit denied observed MCP-over-HTTP security rule ledger rows");
                }
            }
            let mut scrubbed_stats = BodyStats::new(0);
            scrubbed_stats.bytes = observed.bytes_sent;
            let req_ctx = TelemetryRequestContext {
                domain: domain.to_string(),
                process_name: process_name.clone(),
                ai_provider: effective_ai_provider,
                ai_protocol: effective_ai_protocol,
                model_traffic: sniffed_model_request,
                method: method.clone(),
                path: path.clone(),
                query: query.clone(),
                status_code: Some(403),
                decision: Decision::Denied,
                matched_rule: mcp_evaluation.enforcement.rule_id.clone(),
                request_headers: Some(req_hdrs.clone()),
                response_headers: None,
                start_time,
                request_body_stats: Arc::new(Mutex::new(scrubbed_stats)),
                max_response_preview: 0,
                port: upstream_port,
                conn_type,
                policy_mode: request_security_decision.policy_mode.clone(),
                policy_action: request_security_decision.policy_action.clone(),
                policy_rule: request_security_decision.policy_rule.clone(),
                policy_reason: request_security_decision.policy_reason.clone(),
                credential_ref: credential_ref.clone(),
                credential_observations: credential_observations.clone(),
                credential_injections: credential_injections.clone(),
            };
            let deny_body = Full::new(Bytes::from(body_text))
                .map_err(|never| match never {})
                .boxed();
            return Ok(hyper::Response::builder()
                .status(403)
                .body(seal_with_telemetry(
                    deny_body,
                    req_ctx,
                    effective_ai_provider,
                    effective_ai_protocol,
                ))
                .unwrap());
        }
        mcp_span.record("decision", "allow");
        mcp_span.record("status", "ok");
    }

    // Track request body (boxed for consistent sender type across requests).
    // Always capture AI provider request bodies for telemetry parsing
    // (model name, tool results, etc.) regardless of log_bodies setting.
    let req_max_preview =
        body_preview_cap(effective_ai_provider, domain, &path, log_bodies, max_body);
    let req_stats = Arc::new(Mutex::new(BodyStats {
        bytes: 0,
        preview: Vec::new(),
        max_preview: req_max_preview,
    }));

    let should_evaluate_model_request = sniffed_model_request
        || effective_ai_protocol.is_some_and(|protocol| is_llm_api_path(protocol, &path));
    let upstream_req_body: ProxyBoxBody = if should_evaluate_model_request {
        let model_request_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_SECURITY_ACTIONS,
            protocol = protocol.label(),
            provider = provider_label(effective_ai_provider),
            decision = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let body_bytes = match request_body_source {
            RequestBodySource::Collected(body_bytes) => body_bytes,
            RequestBodySource::Incoming(body) => {
                let collected = match http_body_util::Limited::new(body, 100 * 1024 * 1024)
                    .collect()
                    .instrument(model_request_span.clone())
                    .await
                {
                    Ok(collected) => collected,
                    Err(error) => {
                        model_request_span.record("decision", "error");
                        model_request_span.record("status", "error");
                        model_request_span.record("error_kind", "collect_model_request_body");
                        return Ok(make_502(
                            &error,
                            &method,
                            &path,
                            &query,
                            &req_hdrs,
                            start_time,
                            &request_security_decision,
                        ));
                    }
                };
                collected.to_bytes()
            }
        };
        let mut body_for_upstream = body_bytes.clone();
        {
            let mut st = req_stats.lock().expect("req body stats lock");
            st.bytes = body_bytes.len() as u64;
            let to_copy = st.max_preview.min(body_bytes.len());
            st.preview.extend_from_slice(&body_bytes[..to_copy]);
        }

        if let (Some(provider), Some(model_protocol)) =
            (effective_ai_provider, effective_ai_protocol)
        {
            let request_meta =
                crate::net::ai_traffic::request_parser::parse_request(model_protocol, &body_bytes);
            let model_event = model_security_event(
                RuntimeSecurityEventType::ModelCall,
                provider,
                request_meta.model.clone(),
                Some(&body_bytes),
                None,
            )
            .with_http(HttpSecurityEvent {
                host: Some(domain.to_string()),
                method: Some(method.clone()),
                path: Some(path.clone()),
                query: query.clone(),
                status: None,
                body: Some(String::from_utf8_lossy(&body_bytes).to_string()),
            });
            let model_event = security_event_with_transport(model_event, domain, upstream_port);
            let model_evaluation = match crate::security_engine::evaluate_security_boundary(
                &rules,
                config.telemetry.plugin_policy.read().unwrap().clone(),
                model_event,
            ) {
                Ok(evaluation) => evaluation,
                Err(error) => {
                    model_request_span.record("decision", "error");
                    model_request_span.record("status", "error");
                    model_request_span.record("error_kind", "security_actions");
                    return Ok(make_502(
                        &error,
                        &method,
                        &path,
                        &query,
                        &req_hdrs,
                        start_time,
                        &request_security_decision,
                    ));
                }
            };
            request_security_decision =
                SecurityBoundaryDecisionFields::from_enforcement(&model_evaluation.enforcement);
            if !model_evaluation.enforcement.is_allowed() {
                model_request_span.record("decision", model_evaluation.enforcement.action.as_str());
                model_request_span.record("status", "ok");
                let body_text = format!(
                    "capsem: model request blocked by security rule: {}\n",
                    model_evaluation
                        .enforcement
                        .rule_id
                        .as_deref()
                        .unwrap_or("unknown")
                );
                let mut scrubbed_stats = BodyStats::new(0);
                scrubbed_stats.bytes = body_bytes.len() as u64;
                let req_ctx = TelemetryRequestContext {
                    domain: domain.to_string(),
                    process_name: process_name.clone(),
                    ai_provider: effective_ai_provider,
                    ai_protocol: effective_ai_protocol,
                    model_traffic: true,
                    method: method.clone(),
                    path: path.clone(),
                    query: query.clone(),
                    status_code: Some(403),
                    decision: Decision::Denied,
                    matched_rule: model_evaluation.enforcement.rule_id.clone(),
                    request_headers: Some(req_hdrs.clone()),
                    response_headers: None,
                    start_time,
                    request_body_stats: Arc::new(Mutex::new(scrubbed_stats)),
                    max_response_preview: 0,
                    port: upstream_port,
                    conn_type,
                    policy_mode: request_security_decision.policy_mode.clone(),
                    policy_action: request_security_decision.policy_action.clone(),
                    policy_rule: request_security_decision.policy_rule.clone(),
                    policy_reason: request_security_decision.policy_reason.clone(),
                    credential_ref: credential_ref.clone(),
                    credential_observations: credential_observations.clone(),
                    credential_injections: credential_injections.clone(),
                };
                let deny_body = Full::new(Bytes::from(body_text))
                    .map_err(|never| match never {})
                    .boxed();
                return Ok(hyper::Response::builder()
                    .status(403)
                    .body(seal_with_telemetry(
                        deny_body,
                        req_ctx,
                        effective_ai_provider,
                        effective_ai_protocol,
                    ))
                    .unwrap());
            }
            model_request_span.record("decision", "allow");
            model_request_span.record("status", "ok");
            if let Some(model) = model_evaluation.event.model.as_ref() {
                if let Some(updated_body) = model.request_body.as_ref() {
                    if updated_body.as_bytes() != body_bytes.as_ref() {
                        body_for_upstream = Bytes::from(updated_body.clone());
                        {
                            let mut st = req_stats.lock().expect("req body stats lock");
                            st.bytes = body_for_upstream.len() as u64;
                            st.preview.clear();
                            let to_copy = st.max_preview.min(body_for_upstream.len());
                            st.preview.extend_from_slice(&body_for_upstream[..to_copy]);
                        }
                        original_headers.remove(http::header::CONTENT_LENGTH);
                        if let Ok(value) =
                            http::HeaderValue::from_str(&body_for_upstream.len().to_string())
                        {
                            original_headers.insert(http::header::CONTENT_LENGTH, value);
                        }
                    }
                }
            }
        }

        Full::new(body_for_upstream)
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed()
    } else {
        match request_body_source {
            RequestBodySource::Collected(body_bytes) => {
                {
                    let mut st = req_stats.lock().expect("req body stats lock");
                    st.bytes = body_bytes.len() as u64;
                    let to_copy = st.max_preview.min(body_bytes.len());
                    st.preview.extend_from_slice(&body_bytes[..to_copy]);
                }
                Full::new(body_bytes)
                    .map_err(|never| -> anyhow::Error { match never {} })
                    .boxed()
            }
            RequestBodySource::Incoming(body) => {
                TrackedBody::new(body, Arc::clone(&req_stats), 100 * 1024 * 1024).boxed()
            }
        }
    };

    // Try to reuse a cached upstream sender, or create a new
    // connection. Each MITM connection serves one upstream via
    // keep-alive, so per-connection caching avoids re-establishing
    // TCP[+TLS] for every request.
    let upstream_prepare_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_UPSTREAM_PREPARE,
        protocol = protocol.label(),
        provider = provider_label(effective_ai_provider),
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let upstream_lock_start = Instant::now();
    let mut reusable = cached_upstream
        .lock()
        .instrument(upstream_prepare_span.clone())
        .await
        .take();
    let upstream_lock_us = upstream_lock_start.elapsed().as_micros() as u64;

    // If we have a cached sender, check it's still alive.
    let ready_us = if let Some(ref mut s) = reusable {
        let ready_start = Instant::now();
        if s.ready()
            .instrument(upstream_prepare_span.clone())
            .await
            .is_err()
        {
            reusable = None;
        }
        ready_start.elapsed().as_micros() as u64
    } else {
        0
    };

    let reused = reusable.is_some();
    let mut tcp_us = 0u64;
    let mut tls_us = 0u64;
    let mut handshake_us = 0u64;
    let upstream_override = policy
        .find_upstream_override(domain, upstream_port)
        .cloned();
    let dial_target = upstream_override
        .as_ref()
        .map(|route| route.dial.clone())
        .unwrap_or_else(|| format!("{domain}:{upstream_port}"));
    let upstream_protocol = upstream_override
        .as_ref()
        .map(|route| match route.protocol {
            crate::net::policy::UpstreamOverrideProtocol::Http => Protocol::Http,
            crate::net::policy::UpstreamOverrideProtocol::Tls => Protocol::Tls,
        })
        .unwrap_or(protocol);

    // Create a fresh upstream connection if needed. TLS path goes
    // TCP -> TLS handshake -> HTTP/1.1 handshake; HTTP path skips
    // the TLS step.
    let mut sender = if let Some(s) = reusable {
        s
    } else {
        let dial_start = Instant::now();
        let tcp_start = Instant::now();
        let upstream_tcp = match tokio::net::TcpStream::connect(&dial_target)
            .instrument(upstream_prepare_span.clone())
            .await
        {
            Ok(tcp) => {
                let _ = tcp.set_nodelay(true);
                tcp
            }
            Err(e) => {
                upstream_prepare_span.record("decision", "error");
                upstream_prepare_span.record("status", "error");
                upstream_prepare_span.record("error_kind", "tcp_connect");
                tcp_us = tcp_start.elapsed().as_micros() as u64;
                tracing::debug!(
                    target: "mitm.transport.upstream",
                    domain, port = upstream_port, reused = false,
                    dial_target = %dial_target,
                    upstream_override = upstream_override.is_some(),
                    upstream_lock_us, ready_us, tcp_us,
                    error = %e, "upstream TCP connect failed"
                );
                ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                    .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                ::metrics::counter!(metrics::REQUESTS_TOTAL,
                    "protocol" => protocol.label(), "decision" => "upstream_error")
                .increment(1);
                return Ok(make_502(
                    &e,
                    &method,
                    &path,
                    &query,
                    &req_hdrs,
                    start_time,
                    &request_security_decision,
                ));
            }
        };
        tcp_us = tcp_start.elapsed().as_micros() as u64;

        // TLS path: wrap TCP in a TLS stream, time the handshake.
        // HTTP path: skip TLS, hand the bare TCP stream to hyper.
        let (sender, hs_us) = match upstream_protocol {
            Protocol::Tls => {
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));
                let server_name = match rustls::pki_types::ServerName::try_from(domain.to_string())
                {
                    Ok(sn) => sn,
                    Err(e) => {
                        return Ok(make_502(
                            &e,
                            &method,
                            &path,
                            &query,
                            &req_hdrs,
                            start_time,
                            &request_security_decision,
                        ));
                    }
                };
                let tls_start = Instant::now();
                let upstream_tls_stream = match connector
                    .connect(server_name, upstream_tcp)
                    .instrument(upstream_prepare_span.clone())
                    .await
                {
                    Ok(tls) => {
                        ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                            .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                        tls
                    }
                    Err(e) => {
                        upstream_prepare_span.record("decision", "error");
                        upstream_prepare_span.record("status", "error");
                        upstream_prepare_span.record("error_kind", "upstream_tls_handshake");
                        ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                            .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                        ::metrics::counter!(metrics::REQUESTS_TOTAL,
                            "protocol" => protocol.label(), "decision" => "upstream_error")
                        .increment(1);
                        return Ok(make_502(
                            &e,
                            &method,
                            &path,
                            &query,
                            &req_hdrs,
                            start_time,
                            &request_security_decision,
                        ));
                    }
                };
                tls_us = tls_start.elapsed().as_micros() as u64;
                let upstream_io = TokioIo::new(upstream_tls_stream);
                let handshake_start = Instant::now();
                let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io)
                    .instrument(upstream_prepare_span.clone())
                    .await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        upstream_prepare_span.record("decision", "error");
                        upstream_prepare_span.record("status", "error");
                        upstream_prepare_span.record("error_kind", "upstream_http_handshake");
                        return Ok(make_502(
                            &e,
                            &method,
                            &path,
                            &query,
                            &req_hdrs,
                            start_time,
                            &request_security_decision,
                        ));
                    }
                };
                let hs = handshake_start.elapsed().as_micros() as u64;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                (sender, hs)
            }
            Protocol::Http => {
                ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                    .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                let upstream_io = TokioIo::new(upstream_tcp);
                let handshake_start = Instant::now();
                let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io)
                    .instrument(upstream_prepare_span.clone())
                    .await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        upstream_prepare_span.record("decision", "error");
                        upstream_prepare_span.record("status", "error");
                        upstream_prepare_span.record("error_kind", "upstream_http_handshake");
                        ::metrics::counter!(metrics::REQUESTS_TOTAL,
                            "protocol" => protocol.label(), "decision" => "upstream_error")
                        .increment(1);
                        return Ok(make_502(
                            &e,
                            &method,
                            &path,
                            &query,
                            &req_hdrs,
                            start_time,
                            &request_security_decision,
                        ));
                    }
                };
                let hs = handshake_start.elapsed().as_micros() as u64;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                (sender, hs)
            }
            Protocol::McpFrame => unreachable!("framed MCP bypasses HTTP upstream dial"),
            Protocol::Unknown => unreachable!("handle_inner gates Unknown earlier"),
        };
        handshake_us = hs_us;
        sender
    };
    upstream_prepare_span.record("decision", if reused { "reuse" } else { "connect" });
    upstream_prepare_span.record("status", "ok");

    tracing::debug!(
        target: "mitm.transport.upstream",
        domain, port = upstream_port, reused, upstream_lock_us, ready_us,
        dial_target = %dial_target,
        upstream_override = upstream_override.is_some(),
        tcp_us, tls_us, handshake_us,
        "upstream sender prepared"
    );

    // Build upstream request with original headers.
    let full_path = match upstream_query {
        Some(q) => format!("{path}?{q}"),
        None => path.clone(),
    };
    let mut builder = hyper::Request::builder()
        .method(original_method)
        .uri(&full_path);
    for (name, value) in original_headers.iter() {
        // TLS: drop inbound `host` -- the SNI-derived `domain` is
        //      authoritative and we re-add it below.
        // HTTP: preserve inbound `host` -- the guest sent it,
        //       and parse_http_host_target already drove our
        //       upstream selection from it.
        let drop_host = matches!(protocol, Protocol::Tls) && name == "host";
        if drop_host || name == "accept-encoding" {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }
    if matches!(protocol, Protocol::Tls) {
        builder = builder.header("host", domain);
    }
    // Only accept gzip -- we can decompress it; brotli/zstd we cannot.
    builder = builder.header("accept-encoding", "gzip");

    let upstream_req = builder.body(upstream_req_body)?;

    let upstream_send_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_UPSTREAM_SEND,
        protocol = protocol.label(),
        provider = provider_label(effective_ai_provider),
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let resp = match sender
        .send_request(upstream_req)
        .instrument(upstream_send_span.clone())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            upstream_send_span.record("decision", "error");
            upstream_send_span.record("status", "error");
            upstream_send_span.record("error_kind", "send_request");
            return Ok(make_502(
                &e,
                &method,
                &path,
                &query,
                &req_hdrs,
                start_time,
                &request_security_decision,
            ));
        }
    };
    upstream_send_span.record("decision", "allow");
    upstream_send_span.record("status", "ok");

    // Put the sender back in the cache for the next request on this connection.
    // The next request's ready().await will naturally wait until this response
    // body completes (hyper 1.x keep-alive semantics).
    cached_upstream.lock().await.replace(sender);
    let (mut resp_parts, resp_body) = resp.into_parts();

    let mut effective_security_decision = request_security_decision.clone();
    let mut effective_matched_rule = effective_security_decision.matched_rule(matched_rule.clone());

    let resp_status = resp_parts.status.as_u16();
    tracing::Span::current().record("status", resp_status);

    // Strip Content-Encoding / Content-Length when the body is gzip --
    // the DecompressionHook (sync ChunkHook) handles the actual byte
    // transformation downstream. The guest receives uncompressed data
    // (vsock is local; compression is unnecessary). This header strip
    // is just three field accesses on the parts struct and stays
    // inline here -- moving it to an async Hook would re-introduce
    // the kind of plumbing the slice removed.
    let is_gzip = resp_parts
        .headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("gzip"))
        .unwrap_or(false);
    if is_gzip {
        resp_parts.headers.remove("content-encoding");
        resp_parts.headers.remove("content-length");
    }
    let mut resp_hdrs = format_headers(&resp_parts.headers);

    // Pick the response-side preview cap. AI provider bodies always
    // capture at least AI_BODY_PREVIEW so non-streaming usage parsing
    // works even when log_bodies is off. Credential broker exchange
    // candidates get a smaller bounded preview for capture/redaction.
    // Other non-AI bodies follow the log_bodies / max_body_capture policy.
    let mut resp_max_preview = response_body_preview_cap(
        effective_ai_provider,
        domain,
        &path,
        log_bodies,
        max_body,
        credential_ref.as_deref(),
    );
    if observed_mcp_request.is_some() {
        resp_max_preview = resp_max_preview.max(MCP_BODY_PREVIEW);
    }

    let should_evaluate_model_response = sniffed_model_request
        || effective_ai_protocol.is_some_and(|protocol| is_llm_api_path(protocol, &path));
    let should_collect_semantic_response =
        should_evaluate_model_response || observed_mcp_request.is_some();

    let resp_body: ProxyBoxBody = if should_collect_semantic_response {
        let model_response_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_SECURITY_ACTIONS,
            protocol = protocol.label(),
            provider = provider_label(effective_ai_provider),
            decision = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let collected = match http_body_util::Limited::new(resp_body, 100 * 1024 * 1024)
            .collect()
            .instrument(model_response_span.clone())
            .await
        {
            Ok(collected) => collected,
            Err(error) => {
                model_response_span.record("decision", "error");
                model_response_span.record("status", "error");
                model_response_span.record("error_kind", "collect_model_response_body");
                return Ok(make_502(
                    &error,
                    &method,
                    &path,
                    &query,
                    &req_hdrs,
                    start_time,
                    &effective_security_decision,
                ));
            }
        };
        let mut response_body = match maybe_decompress_gzip_body(collected.to_bytes(), is_gzip) {
            Ok(body) => body,
            Err(error) => {
                model_response_span.record("decision", "error");
                model_response_span.record("status", "error");
                model_response_span.record("error_kind", "decompress_model_response_body");
                return Ok(make_502(
                    &error,
                    &method,
                    &path,
                    &query,
                    &req_hdrs,
                    start_time,
                    &effective_security_decision,
                ));
            }
        };

        if let (Some(provider), Some(model_protocol)) =
            (effective_ai_provider, effective_ai_protocol)
        {
            let request_preview = {
                let st = req_stats.lock().expect("req body stats lock");
                st.preview.clone()
            };
            let request_meta = crate::net::ai_traffic::request_parser::parse_request(
                model_protocol,
                &request_preview,
            );
            let model_event = model_security_event(
                RuntimeSecurityEventType::ModelCall,
                provider,
                request_meta.model,
                Some(&request_preview),
                Some(&response_body),
            )
            .with_http(HttpSecurityEvent {
                host: Some(domain.to_string()),
                method: Some(method.clone()),
                path: Some(path.clone()),
                query: query.clone(),
                status: Some(resp_status.to_string()),
                body: Some(String::from_utf8_lossy(&response_body).to_string()),
            });
            let model_event = security_event_with_transport(model_event, domain, upstream_port);
            let model_evaluation = match crate::security_engine::evaluate_security_boundary(
                &rules,
                config.telemetry.plugin_policy.read().unwrap().clone(),
                model_event,
            ) {
                Ok(evaluation) => evaluation,
                Err(error) => {
                    model_response_span.record("decision", "error");
                    model_response_span.record("status", "error");
                    model_response_span.record("error_kind", "security_actions");
                    return Ok(make_502(
                        &error,
                        &method,
                        &path,
                        &query,
                        &req_hdrs,
                        start_time,
                        &effective_security_decision,
                    ));
                }
            };
            effective_security_decision =
                SecurityBoundaryDecisionFields::from_enforcement(&model_evaluation.enforcement);
            effective_matched_rule = effective_security_decision.matched_rule(matched_rule.clone());
            if !model_evaluation.enforcement.is_allowed() {
                model_response_span
                    .record("decision", model_evaluation.enforcement.action.as_str());
                model_response_span.record("status", "ok");
                let body_text = format!(
                    "capsem: model response blocked by security rule: {}\n",
                    model_evaluation
                        .enforcement
                        .rule_id
                        .as_deref()
                        .unwrap_or("unknown")
                );
                let req_ctx = TelemetryRequestContext {
                    domain: domain.to_string(),
                    process_name: process_name.clone(),
                    ai_provider: effective_ai_provider,
                    ai_protocol: effective_ai_protocol,
                    model_traffic: true,
                    method,
                    path,
                    query,
                    status_code: Some(403),
                    decision: Decision::Denied,
                    matched_rule: model_evaluation.enforcement.rule_id.clone(),
                    request_headers: Some(req_hdrs),
                    response_headers: None,
                    start_time,
                    request_body_stats: Arc::clone(&req_stats),
                    max_response_preview: 0,
                    port: upstream_port,
                    conn_type,
                    policy_mode: effective_security_decision.policy_mode.clone(),
                    policy_action: effective_security_decision.policy_action.clone(),
                    policy_rule: effective_security_decision.policy_rule.clone(),
                    policy_reason: effective_security_decision.policy_reason.clone(),
                    credential_ref: credential_ref.clone(),
                    credential_observations: credential_observations.clone(),
                    credential_injections: credential_injections.clone(),
                };
                let deny_body = Full::new(Bytes::from(body_text))
                    .map_err(|never| match never {})
                    .boxed();
                return Ok(hyper::Response::builder()
                    .status(403)
                    .body(seal_with_telemetry(
                        deny_body,
                        req_ctx,
                        effective_ai_provider,
                        effective_ai_protocol,
                    ))
                    .unwrap());
            }
            model_response_span.record("decision", "allow");
            model_response_span.record("status", "ok");
            if let Some(model) = model_evaluation.event.model.as_ref() {
                if let Some(updated_body) = model.response_body.as_ref() {
                    if updated_body.as_bytes() != response_body.as_ref() {
                        response_body = Bytes::from(updated_body.clone());
                    }
                }
            }
        }
        if let Some(observed) = observed_mcp_request.as_ref() {
            let response_preview = Some(String::from_utf8_lossy(&response_body).to_string());
            let tool_list = if observed.method == "tools/list" {
                response_preview.clone()
            } else {
                None
            };
            let security_event = observed.security_event(tool_list);
            let call = McpCall {
                event_id: None,
                timestamp: SystemTime::now(),
                server_name: observed.server_name.clone(),
                method: observed.method.clone(),
                tool_name: observed.tool_name.clone(),
                request_id: observed.request_id.clone(),
                request_preview: observed.request_preview.clone(),
                response_preview,
                decision: "allowed".to_string(),
                duration_ms: start_time.elapsed().as_millis() as u64,
                error_message: None,
                process_name: process_name.clone(),
                bytes_sent: observed.bytes_sent,
                bytes_received: response_body.len() as u64,
                policy_mode: mcp_request_security_decision.policy_mode.clone(),
                policy_action: mcp_request_security_decision.policy_action.clone(),
                policy_rule: mcp_request_security_decision.policy_rule.clone(),
                policy_reason: mcp_request_security_decision.policy_reason.clone(),
                trace_id: crate::telemetry::ambient_capsem_trace_id(),
                credential_ref: credential_ref.clone(),
            };
            if let Some(event_id) = emit_security_write(&config.db, WriteOp::McpCall(call)).await {
                if let Err(error) = emit_matching_security_rules(
                    &config.db,
                    event_id,
                    observed.event_type(),
                    &rules,
                    &security_event,
                    current_unix_ms(),
                )
                .await
                {
                    warn!(error = %error, "failed to emit observed MCP-over-HTTP security rule ledger rows");
                }
            }
        }
        materialize_collected_response_headers(
            &mut resp_parts.headers,
            response_body.len(),
            is_gzip,
        );
        resp_hdrs = format_headers(&resp_parts.headers);

        Full::new(response_body)
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed()
    } else {
        resp_body.map_err(|e| -> anyhow::Error { e.into() }).boxed()
    };

    let req_ctx = TelemetryRequestContext {
        domain: domain.to_string(),
        process_name: process_name.clone(),
        ai_provider: effective_ai_provider,
        ai_protocol: effective_ai_protocol,
        model_traffic: should_evaluate_model_response,
        method,
        path,
        query,
        status_code: Some(resp_status),
        decision: Decision::Allowed,
        matched_rule: Some(effective_security_decision.matched_rule(effective_matched_rule)),
        request_headers: Some(req_hdrs),
        response_headers: Some(resp_hdrs),
        start_time,
        request_body_stats: Arc::clone(&req_stats),
        max_response_preview: resp_max_preview,
        port: upstream_port,
        conn_type,
        policy_mode: effective_security_decision.policy_mode.clone(),
        policy_action: effective_security_decision.policy_action.clone(),
        policy_rule: effective_security_decision.policy_rule.clone(),
        policy_reason: effective_security_decision.policy_reason.clone(),
        credential_ref: credential_ref.clone(),
        credential_observations: credential_observations.clone(),
        credential_injections: credential_injections.clone(),
    };

    // Drive the sync ChunkHook chain on every response chunk:
    //   DecompressionHook (gzip) → SseParserHook (AI) →
    //   InterpreterHook* → TelemetryHook. The TelemetryHook reads the
    //   seeded TelemetryRequestContext at on_response_end, builds the
    //   NetEvent (+ ModelCall for AI), and spawns the DB writes.
    let chunk_dispatched = body::ChunkDispatchBody::new(
        resp_body,
        Arc::clone(&config.pipeline),
        hooks::ConnMeta {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            port: upstream_port,
            protocol,
            ai_provider: effective_ai_provider,
            ai_protocol: effective_ai_protocol,
        },
        crate::telemetry::ambient_capsem_trace_id(),
    )
    .seed::<decompression_hook::DecompressionConfig>(decompression_hook::DecompressionConfig {
        gzip: is_gzip,
    })
    .seed::<Option<TelemetryRequestContext>>(Some(req_ctx));
    let chunk_dispatched = if is_gzip {
        chunk_dispatched.without_size_hint()
    } else {
        chunk_dispatched
    };

    let response = hyper::Response::from_parts(resp_parts, chunk_dispatched.boxed());
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::policy_config::{SecurityRuleAction, SecurityRuleProfile, SecurityRuleSet};

    #[test]
    fn collected_gzip_chunked_response_headers_are_materialized() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_static("gzip"),
        );
        headers.insert(
            http::header::TRANSFER_ENCODING,
            http::HeaderValue::from_static("chunked"),
        );
        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("9999"),
        );

        materialize_collected_response_headers(&mut headers, 1234, true);

        assert!(!headers.contains_key(http::header::CONTENT_ENCODING));
        assert!(!headers.contains_key(http::header::TRANSFER_ENCODING));
        assert_eq!(
            headers.get(http::header::CONTENT_LENGTH),
            Some(&http::HeaderValue::from_static("1234"))
        );
    }

    #[test]
    fn provider_detection_marks_undeclared_model_path_as_unknown_provider() {
        let registry = crate::net::policy_config::ModelEndpointRegistry::default();

        assert_eq!(
            ai_identity_for_target_or_path(
                &registry,
                "rogue-openai-compatible.example",
                443,
                "/v1/chat/completions"
            ),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Unknown),
                protocol: Some(ModelProtocol::OpenAi),
            }
        );
        assert_eq!(
            ai_identity_for_target_or_path(&registry, "unknown.example", 443, "/v1/messages"),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Unknown),
                protocol: Some(ModelProtocol::Anthropic),
            }
        );
        assert_eq!(
            ai_identity_for_target_or_path(
                &registry,
                "unknown.example",
                443,
                "/v1beta/models/gemini-2.5-pro:generateContent"
            ),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Unknown),
                protocol: Some(ModelProtocol::Google),
            }
        );
        assert_eq!(
            ai_identity_for_target_or_path(&registry, "unknown.example", 443, "/api/chat"),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Unknown),
                protocol: Some(ModelProtocol::Ollama),
            }
        );
    }

    #[test]
    fn provider_identity_keeps_ollama_endpoint_owner_with_path_protocol() {
        let profile = crate::net::policy_config::ProviderRuleProfile::parse_toml(
            r#"
[ai.ollama]
name = "Ollama"
protocol = "ollama"
url = "http://127.0.0.1:11434"
listen_ports = [11434]

[ai.ollama.rules.local]
name = "ollama_local"
action = "allow"
match = 'http.host == "127.0.0.1"'
"#,
        )
        .expect("provider profile parses");
        let registry = profile.endpoint_registry().expect("registry builds");

        assert_eq!(
            ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/v1/messages"),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Ollama),
                protocol: Some(ModelProtocol::Anthropic),
            }
        );
        assert_eq!(
            ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/v1/responses"),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Ollama),
                protocol: Some(ModelProtocol::OpenAi),
            }
        );
        assert_eq!(
            ai_identity_for_target_or_path(&registry, "127.0.0.1", 11434, "/api/chat"),
            ModelTrafficIdentity {
                provider: Some(ProviderKind::Ollama),
                protocol: Some(ModelProtocol::Ollama),
            }
        );
    }

    #[test]
    fn provider_detection_promotes_unknown_host_by_bounded_body_shape() {
        assert_eq!(
            ai_protocol_for_body_preview(
                br#"{"model":"gpt-4.1","messages":[{"role":"user","content":"hi"}]}"#
            ),
            Some(ModelProtocol::OpenAi)
        );
        assert_eq!(
            ai_protocol_for_body_preview(
                br#"{"model":"claude-3-5-sonnet","max_tokens":128,"messages":[{"role":"user","content":"hi"}]}"#
            ),
            Some(ModelProtocol::Anthropic)
        );
        assert_eq!(
            ai_protocol_for_body_preview(
                br#"{"model":"gemini-2.5-pro","contents":[{"parts":[{"text":"hi"}]}]}"#
            ),
            Some(ModelProtocol::Google)
        );
    }

    #[test]
    fn provider_detection_body_shape_ignores_oversized_or_irrelevant_bodies() {
        let mut oversized = vec![b' '; AI_BODY_PREVIEW + 1];
        oversized.extend_from_slice(
            br#"{"model":"gpt-4.1","messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(ai_protocol_for_body_preview(&oversized), None);
        assert_eq!(ai_protocol_for_body_preview(br#"{"hello":"world"}"#), None);
    }

    #[test]
    fn http_request_security_event_exposes_transport_and_body_to_cel() {
        let profile = SecurityRuleProfile::parse_toml(
            r#"
[corp.rules.allow_local_fixture]
name = "allow_local_fixture"
action = "allow"
priority = -100
match = 'http.host == "127.0.0.1" && tcp.port == "3713" && ip.value == "127.0.0.1" && http.query == "case=plain-json" && http.body.contains("ironbank_http_plain_json")'
"#,
        )
        .expect("profile parses");
        let rules = SecurityRuleSet::compile_profile(
            &profile,
            crate::net::policy_config::SecurityRuleSource::Corp,
        )
        .expect("rules compile");

        let event = http_request_security_event(
            "127.0.0.1",
            3713,
            "POST",
            "/echo",
            Some("case=plain-json".to_string()),
            None,
            http::HeaderMap::new(),
            Some(&Bytes::from_static(
                br#"{"kind":"ironbank_http_plain_json"}"#,
            )),
        );
        let first = rules
            .evaluate(&event)
            .expect("event evaluates")
            .enforcement_rules()
            .into_iter()
            .next()
            .expect("transport/body rule matches");

        assert_eq!(first.rule_id, "corp.rules.allow_local_fixture");
        assert_eq!(first.action, SecurityRuleAction::Allow);
    }

    #[test]
    fn unknown_model_body_sniffing_is_json_and_length_bounded() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("128"),
        );
        assert!(should_sniff_unknown_model_body(
            None,
            &http::Method::POST,
            &headers
        ));
        assert!(!should_sniff_unknown_model_body(
            Some(ProviderKind::OpenAi),
            &http::Method::POST,
            &headers
        ));
        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&(AI_BODY_PREVIEW + 1).to_string()).unwrap(),
        );
        assert!(!should_sniff_unknown_model_body(
            None,
            &http::Method::POST,
            &headers
        ));
        headers.remove(http::header::CONTENT_LENGTH);
        assert!(!should_sniff_unknown_model_body(
            None,
            &http::Method::POST,
            &headers
        ));
    }

    #[test]
    fn unknown_mcp_http_body_sniffing_is_json_and_length_bounded() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("128"),
        );
        assert!(should_sniff_mcp_http_body(&http::Method::POST, &headers));

        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&(MCP_BODY_PREVIEW + 1).to_string()).unwrap(),
        );
        assert!(!should_sniff_mcp_http_body(&http::Method::POST, &headers));

        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("128"),
        );
        assert!(!should_sniff_mcp_http_body(&http::Method::GET, &headers));

        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain"),
        );
        assert!(!should_sniff_mcp_http_body(&http::Method::POST, &headers));
    }

    #[test]
    fn observed_mcp_http_request_requires_mcp_json_rpc_shape() {
        let body = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fetch_http","arguments":{"url":"https://example.com"}}}"#;
        let observed =
            observed_mcp_http_request_for_body(body, "mcp.example.test", 443, "/mcp").unwrap();
        assert_eq!(observed.method, "tools/call");
        assert_eq!(observed.tool_name.as_deref(), Some("fetch_http"));
        assert_eq!(observed.request_id.as_deref(), Some("7"));
        assert_eq!(observed.server_name, "observed:mcp.example.test:443/mcp");

        assert!(observed_mcp_http_request_for_body(
            br#"{"jsonrpc":"2.0","method":"eth_call"}"#,
            "rpc.example.test",
            443,
            "/"
        )
        .is_none());
        assert!(observed_mcp_http_request_for_body(
            br#"{"method":"tools/call","params":{"name":"fetch_http"}}"#,
            "mcp.example.test",
            443,
            "/mcp"
        )
        .is_none());
    }

    #[test]
    fn body_preview_cap_captures_oauth_broker_candidates_without_body_logging() {
        assert_eq!(
            body_preview_cap(None, "oauth2.googleapis.com", "/token", false, 0),
            CREDENTIAL_BODY_PREVIEW
        );
        assert_eq!(
            body_preview_cap(
                None,
                "api.github.com",
                "/login/oauth/access_token",
                false,
                0
            ),
            CREDENTIAL_BODY_PREVIEW
        );
    }

    #[test]
    fn body_preview_cap_keeps_unrelated_non_ai_bodies_off_without_body_logging() {
        assert_eq!(
            body_preview_cap(
                None,
                "daily-cloudcode-pa.googleapis.com",
                "/v1internal:streamGenerateContent",
                false,
                0
            ),
            0
        );
    }

    #[test]
    fn response_body_preview_cap_captures_broker_replay_proof_without_body_logging() {
        assert_eq!(
            response_body_preview_cap(
                None,
                "127.0.0.1",
                "/echo",
                false,
                0,
                Some("credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            ),
            CREDENTIAL_BODY_PREVIEW
        );
        assert_eq!(
            response_body_preview_cap(None, "127.0.0.1", "/echo", false, 0, None),
            0
        );
    }

    #[test]
    fn body_preview_cap_keeps_ai_capture_independent_from_body_logging() {
        assert_eq!(
            body_preview_cap(
                Some(ProviderKind::Google),
                "daily-cloudcode-pa.googleapis.com",
                "/v1internal:streamGenerateContent",
                false,
                0
            ),
            AI_BODY_PREVIEW
        );
        assert_eq!(
            body_preview_cap(
                Some(ProviderKind::Anthropic),
                "127.0.0.1",
                "/v1/messages",
                false,
                128 * 1024
            ),
            AI_BODY_PREVIEW
        );
    }
}
