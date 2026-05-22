#![allow(dead_code)]
/// MITM transparent proxy: terminates TLS from the guest, inspects HTTP traffic,
/// bridges to the real upstream server.
///
/// Connection flow:
/// 1. Read initial bytes from vsock fd (TLS ClientHello)
/// 2. TLS handshake (MitmCertResolver captures domain from SNI)
/// 3. Read HTTP request via hyper
/// 4. Upstream TLS to real server
/// 5. Forward request, stream response back
/// 6. Emit per-request telemetry (one NetEvent per HTTP request, not per connection)
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
mod pipeline_factory;
pub mod protocol;
mod response;
pub mod sse_parser_hook;
pub mod telemetry_hook;
mod upstream;
mod util;

use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime};

use capsem_logger::{DbWriter, Decision, NetEvent, WriteOp};
use capsem_security_engine::{
    SecurityAction, SecurityDecisionAction, SecurityEngineError, SecurityEvent, SecurityResult,
};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

use super::cert_authority::{CertAuthority, MitmCertResolver};
use crate::net::ai_traffic::provider::ProviderKind;
use body::{BodyStats, ProxyBoxBody, TrackedBody};
use fd_stream::{set_nonblocking, AsyncFdStream, ReplayReader};
use protocol::Protocol;
use telemetry_hook::{TelemetryIdentityContext, TelemetryRequestContext};
use util::{format_headers, parse_http_host_target, split_path_query};

pub use mcp_endpoint::{McpEndpointState, McpTimeouts};
pub use pipeline_factory::{make_default_pipeline, make_production_pipeline};
use response::response_uses_gzip_content_encoding;
use upstream::upstream_connect_target;
#[cfg(test)]
use upstream::UpstreamConnectTarget;
pub use upstream::{make_upstream_tls_config, UpstreamTlsConfig};

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;
const DEFAULT_BODY_PREVIEW_BYTES: usize = 4096;
const LOG_BODY_PREVIEWS: bool = true;
const SECURITY_BLOCK_STATUS: u16 = 403;

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
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
    /// Hook pipeline. `make_production_pipeline` registers the sync ChunkHook
    /// chain (decompression → SSE parse →
    /// provider interpreters → telemetry). `handle_request` dispatches L1
    /// events through this pipeline and seeds per-request context into the
    /// `ChunkDispatchBody`'s `HookState` before serving.
    pub pipeline: Arc<pipeline::Pipeline>,
    /// Optional runtime Security Engine used by transport code to project
    /// normalized request events into allow/block/ask/rewrite outcomes before
    /// touching upstream. The engine boundary is intentionally typed: MITM
    /// does not know about registries, profile storage, or service routes.
    pub security_engine: Arc<RuntimeSecurityEngineSlot>,
    /// T3 framed MCP endpoint on the MITM listener. Dispatch state lives
    /// here so the low-privilege aggregator remains DB-free while MITM
    /// owns policy, timeouts, and `mcp_calls` telemetry.
    pub mcp_endpoint: Option<Arc<McpEndpointState>>,
}

pub trait RuntimeSecurityEngine: Send + Sync {
    fn evaluate(&self, event: SecurityEvent) -> Result<SecurityResult, SecurityEngineError>;
}

#[derive(Default)]
pub struct RuntimeSecurityEngineSlot {
    inner: RwLock<Option<Arc<dyn RuntimeSecurityEngine>>>,
}

impl RuntimeSecurityEngineSlot {
    pub fn new(engine: Option<Arc<dyn RuntimeSecurityEngine>>) -> Self {
        Self {
            inner: RwLock::new(engine),
        }
    }

    pub fn set(&self, engine: Option<Arc<dyn RuntimeSecurityEngine>>) {
        *self
            .inner
            .write()
            .expect("runtime security engine slot lock poisoned") = engine;
    }

    pub fn has_engine(&self) -> bool {
        self.inner
            .read()
            .expect("runtime security engine slot lock poisoned")
            .is_some()
    }
}

impl RuntimeSecurityEngine for RuntimeSecurityEngineSlot {
    fn evaluate(&self, event: SecurityEvent) -> Result<SecurityResult, SecurityEngineError> {
        let engine = self
            .inner
            .read()
            .map_err(|error| SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Enforcement,
                message: format!("runtime security engine slot lock poisoned: {error}"),
            })?
            .clone()
            .ok_or_else(|| SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Enforcement,
                message: "runtime security engine is not installed".into(),
            })?;
        engine.evaluate(event)
    }
}

impl RuntimeSecurityEngine for std::sync::Mutex<capsem_security_engine::SecurityEngine> {
    fn evaluate(&self, event: SecurityEvent) -> Result<SecurityResult, SecurityEngineError> {
        let mut engine = self
            .lock()
            .map_err(|error| SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Enforcement,
                message: format!("runtime security engine lock poisoned: {error}"),
            })?;
        engine.evaluate(event)
    }
}

struct RuntimeHttpRequestInput {
    domain: String,
    process_name: Option<String>,
    ai_provider: Option<ProviderKind>,
    method: String,
    path: String,
    query: Option<String>,
    request_headers: String,
    start_time: Instant,
    request_body_stats: Arc<Mutex<BodyStats>>,
    max_response_preview: usize,
    port: u16,
    conn_type: &'static str,
}

enum RuntimeHttpDecision {
    Allow,
    Reject(TelemetryRequestContext, String),
}

fn evaluate_runtime_http_request(
    config: &MitmProxyConfig,
    input: RuntimeHttpRequestInput,
) -> Option<Result<RuntimeHttpDecision, SecurityEngineError>> {
    if !config.security_engine.has_engine() {
        return None;
    }
    Some(evaluate_runtime_http_request_inner(
        config.security_engine.as_ref(),
        input,
    ))
}

fn evaluate_runtime_http_request_inner(
    engine: &dyn RuntimeSecurityEngine,
    input: RuntimeHttpRequestInput,
) -> Result<RuntimeHttpDecision, SecurityEngineError> {
    let req_ctx = TelemetryRequestContext {
        domain: input.domain,
        process_name: input.process_name,
        ai_provider: input.ai_provider,
        method: input.method,
        path: input.path,
        query: input.query,
        status_code: None,
        decision: Decision::Allowed,
        matched_rule: None,
        request_headers: Some(input.request_headers),
        response_headers: None,
        start_time: input.start_time,
        request_body_stats: input.request_body_stats,
        max_response_preview: input.max_response_preview,
        port: input.port,
        conn_type: input.conn_type,
        identity: TelemetryIdentityContext::from_env(),
        policy_mode: Some("runtime".into()),
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
    };
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let event = telemetry_hook::build_http_security_event(
        &req_ctx,
        timestamp_unix_ms,
        crate::telemetry::ambient_capsem_trace_id(),
        None,
        None,
    );
    let result = engine.evaluate(event)?;

    if runtime_action_allows_transport(&result.action) {
        return Ok(RuntimeHttpDecision::Allow);
    }

    let decision = result.resolved_event.event.decision.as_ref();
    let policy_rule = decision.and_then(|decision| decision.rule.clone());
    let policy_reason = runtime_security_reason(&result);
    let policy_action = decision
        .map(|decision| security_decision_action_label(decision.action).to_string())
        .unwrap_or_else(|| security_action_label(&result.action).to_string());
    let mut denied_ctx = req_ctx;
    denied_ctx.status_code = Some(SECURITY_BLOCK_STATUS);
    denied_ctx.decision = Decision::Denied;
    denied_ctx.matched_rule = policy_rule.clone().or_else(|| Some(policy_reason.clone()));
    denied_ctx.policy_action = Some(policy_action);
    denied_ctx.policy_rule = policy_rule;
    denied_ctx.policy_reason = Some(policy_reason.clone());

    Ok(RuntimeHttpDecision::Reject(
        denied_ctx,
        format!("Capsem: request blocked by security engine ({policy_reason})\n"),
    ))
}

fn runtime_action_allows_transport(action: &SecurityAction) -> bool {
    matches!(
        action,
        SecurityAction::Continue | SecurityAction::ObserveOnly
    )
}

fn runtime_security_reason(result: &SecurityResult) -> String {
    if let Some(reason) = result
        .resolved_event
        .event
        .decision
        .as_ref()
        .and_then(|decision| decision.reason.clone())
    {
        return reason;
    }
    match &result.action {
        SecurityAction::Block(block) => block.reason_code.clone(),
        SecurityAction::Ask(ask) => ask.reason_code.clone(),
        SecurityAction::Throttle(throttle) => throttle.reason_code.clone(),
        SecurityAction::DropConnection(drop) => drop.reason_code.clone(),
        SecurityAction::Error(error) => error.message.clone(),
        SecurityAction::Rewrite(_) => "rewrite_not_applied".into(),
        SecurityAction::Quarantine(_) => "quarantine_not_supported_for_http".into(),
        SecurityAction::Restore(_) => "restore_not_supported_for_http".into(),
        SecurityAction::Continue | SecurityAction::ObserveOnly => "allowed".into(),
    }
}

fn security_decision_action_label(action: SecurityDecisionAction) -> &'static str {
    match action {
        SecurityDecisionAction::Allow => "allow",
        SecurityDecisionAction::Ask => "ask",
        SecurityDecisionAction::Block => "block",
        SecurityDecisionAction::Rewrite => "rewrite",
        SecurityDecisionAction::Throttle => "throttle",
    }
}

fn security_action_label(action: &SecurityAction) -> &'static str {
    match action {
        SecurityAction::Continue => "continue",
        SecurityAction::Ask(_) => "ask",
        SecurityAction::Rewrite(_) => "rewrite",
        SecurityAction::Block(_) => "block",
        SecurityAction::Throttle(_) => "throttle",
        SecurityAction::Quarantine(_) => "quarantine",
        SecurityAction::Restore(_) => "restore",
        SecurityAction::DropConnection(_) => "drop_connection",
        SecurityAction::ObserveOnly => "observe_only",
        SecurityAction::Error(_) => "error",
    }
}

async fn collect_request_body_for_security(
    body: hyper::body::Incoming,
    stats: &Arc<Mutex<BodyStats>>,
    max_size: usize,
) -> Result<Bytes, anyhow::Error> {
    use http_body_util::{BodyExt, Limited};

    let bytes = Limited::new(body, max_size)
        .collect()
        .await
        .map_err(|error| anyhow::anyhow!("request body read failed: {error}"))?
        .to_bytes();
    let mut stats = stats.lock().expect("req body stats lock");
    stats.bytes = bytes.len() as u64;
    stats.preview.clear();
    let preview_len = stats.max_preview.min(bytes.len());
    stats.preview.extend_from_slice(&bytes[..preview_len]);
    Ok(bytes)
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

/// Detect AI provider from domain name.
fn detect_ai_provider(domain: &str) -> Option<ProviderKind> {
    match domain {
        "api.anthropic.com" => Some(ProviderKind::Anthropic),
        "api.openai.com" => Some(ProviderKind::OpenAi),
        "generativelanguage.googleapis.com" => Some(ProviderKind::Google),
        _ => None,
    }
}

/// Handle a single MITM proxy connection from the guest.
///
/// This is the async entry point for each vsock:5002 connection.
/// Per-request telemetry is emitted by `TelemetryHook` (a sync
/// ChunkHook) when each HTTP response body completes. This function
/// only emits connection-level error events (TLS failures, no SNI,
/// etc.).
#[tracing::instrument(skip_all, target = "mitm.connection", fields(vsock_fd, domain = tracing::field::Empty))]
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
            };

            config.db.write(WriteOp::NetEvent(event)).await;
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

    // 1. Read initial bytes (TLS ClientHello + potential metadata).
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut initial_buf)
        .await
        .map_err(|e| {
            (
                String::new(),
                Decision::Error,
                format!("read ClientHello: {e}"),
            )
        })?;
    if n == 0 {
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
                .await
                .map_err(|e| {
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
                .await
                .map_err(|e| {
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
            .await
            .map_err(|e| {
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
    let tls_stream = acceptor.accept(replay).await.map_err(|e| {
        ::metrics::histogram!(metrics::TLS_HANDSHAKE_MS)
            .record(handshake_start.elapsed().as_secs_f64() * 1000.0);
        let domain = resolver.domain().unwrap_or_default();
        (domain, Decision::Error, format!("TLS handshake: {e}"))
    })?;
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
            let ai_provider = detect_ai_provider(&request_domain);
            handle_request(
                req,
                &request_domain,
                protocol,
                upstream_port,
                &upstream_tls,
                &config_arc,
                &process_name,
                ai_provider,
                &cached_upstream,
            )
            .await
        }
    });

    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await
    {
        // Connection errors are expected when the guest closes.
        let err_str = e.to_string();
        if !e.is_incomplete_message() && !err_str.contains("error shutting down connection") {
            warn!(error = %e, "hyper serve error");
        }
    }
}

/// Handle a single HTTP request within a MITM-proxied connection
/// (TLS or plain HTTP).
///
/// Reads the live Policy config per-request so settings changes (e.g.
/// disabling a provider) take effect immediately, even for in-flight keep-alive
/// connections.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    target = "mitm.request",
    fields(
        domain = %domain,
        protocol = protocol.label(),
        port = upstream_port,
        method = tracing::field::Empty,
        path = tracing::field::Empty,
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
    )
)]
async fn handle_request(
    req: hyper::Request<hyper::body::Incoming>,
    domain: &str,
    protocol: Protocol,
    upstream_port: u16,
    upstream_tls: &Arc<rustls::ClientConfig>,
    config: &Arc<MitmProxyConfig>,
    process_name: &Option<String>,
    ai_provider: Option<ProviderKind>,
    cached_upstream: &tokio::sync::Mutex<
        Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>,
    >,
) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error> {
    use http_body_util::BodyExt;

    let log_bodies = LOG_BODY_PREVIEWS;
    let max_body = DEFAULT_BODY_PREVIEW_BYTES;

    // `conn_type` for telemetry. Derived from protocol; landed in
    // every TelemetryRequestContext below.
    let conn_type: &'static str = match protocol {
        Protocol::Tls => "https-mitm",
        Protocol::Http => "http-mitm",
        Protocol::McpFrame => "mcp-frame",
        Protocol::Unknown => "unknown-mitm",
    };
    let telemetry_identity = TelemetryIdentityContext::from_env();

    let start_time = Instant::now();
    let (parts, req_body) = req.into_parts();
    let mut req_body = Some(req_body);
    let initial_method = parts.method.to_string();
    let (initial_path, _) = split_path_query(&parts.uri);

    // Span fields for the #[instrument] decoration -- sets method
    // + path on the span so every log line in this request carries
    // them. decision + status are filled later as we learn them.
    {
        let span = tracing::Span::current();
        span.record("method", initial_method.as_str());
        span.record("path", initial_path.as_str());
    }

    // Check for WebSocket upgrade.
    let is_upgrade = parts
        .headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let method = parts.method.to_string();
    let (path, query) = split_path_query(&parts.uri);
    let req_hdrs = format_headers(&parts.headers);

    // T1 slice 4: per-request counter, partitioned by decision.
    // upstream_error increments are handled at the dial site below.
    let req_decision_label = "allow";
    tracing::Span::current().record("decision", req_decision_label);
    ::metrics::counter!(metrics::REQUESTS_TOTAL,
        "protocol" => protocol.label(), "decision" => req_decision_label)
    .increment(1);

    // Helper: wrap an already-built response body in
    // `ChunkDispatchBody` seeded with the per-request
    // `TelemetryRequestContext`, so the registered `TelemetryHook`
    // fires `NetEvent` (+ `ModelCall`) on body completion. Used by
    // every response path that doesn't reach upstream (deny,
    // websocket-deny, 502).
    let seal_with_telemetry =
        |inner: ProxyBoxBody, req_ctx: TelemetryRequestContext| -> ProxyBoxBody {
            let dispatched = body::ChunkDispatchBody::new(
                inner,
                Arc::clone(&config.pipeline),
                hooks::ConnMeta {
                    domain: domain.to_string(),
                    process_name: process_name.clone(),
                    port: upstream_port,
                    protocol,
                    ai_provider,
                },
                crate::telemetry::ambient_capsem_trace_id(),
            )
            .seed::<Option<TelemetryRequestContext>>(Some(req_ctx));
            dispatched.boxed()
        };

    // Reject WebSocket upgrades (not supported through MITM proxy).
    if is_upgrade {
        let body_text = format!(
            "Capsem: WebSocket upgrades are not supported ({} {})\n",
            method, path
        );

        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(400),
            decision: Decision::Denied,
            matched_rule: Some("websocket-not-supported".to_string()),
            request_headers: Some(req_hdrs),
            response_headers: None,
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            identity: telemetry_identity.clone(),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        };

        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();

        return Ok(hyper::Response::builder()
            .status(400)
            .body(seal_with_telemetry(deny_body, req_ctx))
            .unwrap());
    }

    // Save original request headers.
    let original_headers = parts.headers.clone();
    let original_method = parts.method.clone();

    // Helper: build a 502 Bad Gateway response with telemetry so upstream
    // errors don't kill keep-alive connections (returns Ok, not Err).
    let make_502 = |error: &dyn std::fmt::Display,
                    method: &str,
                    path: &str,
                    query: &Option<String>,
                    req_hdrs: &str,
                    start: Instant|
     -> hyper::Response<ProxyBoxBody> {
        warn!(domain, method, path, error = %error, "MITM proxy: upstream error");
        let body_text = format!("Capsem: upstream error ({error})\n");
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
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
            identity: telemetry_identity.clone(),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::builder()
            .status(502)
            .body(seal_with_telemetry(deny_body, req_ctx))
            .unwrap()
    };

    // Track request body (boxed for consistent sender type across requests).
    // Always capture AI provider request bodies for telemetry parsing
    // (model name, tool results, etc.) regardless of log_bodies setting.
    const AI_BODY_PREVIEW: usize = 64 * 1024;
    let req_max_preview = if ai_provider.is_some() {
        AI_BODY_PREVIEW.max(if log_bodies { max_body } else { 0 })
    } else if log_bodies {
        max_body
    } else {
        0
    };
    let req_stats = Arc::new(Mutex::new(BodyStats {
        bytes: 0,
        preview: Vec::new(),
        max_preview: req_max_preview,
    }));
    let buffered_request_body = if config.security_engine.has_engine() {
        Some(
            collect_request_body_for_security(
                req_body
                    .take()
                    .expect("request body should be present before security collection"),
                &req_stats,
                100 * 1024 * 1024,
            )
            .await?,
        )
    } else {
        None
    };

    if let Some(runtime_decision) = evaluate_runtime_http_request(
        config,
        RuntimeHttpRequestInput {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            request_headers: req_hdrs.clone(),
            start_time,
            request_body_stats: Arc::clone(&req_stats),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
        },
    ) {
        match runtime_decision {
            Ok(RuntimeHttpDecision::Allow) => {}
            Ok(RuntimeHttpDecision::Reject(req_ctx, body_text)) => {
                let deny_body = Full::new(Bytes::from(body_text))
                    .map_err(|never| match never {})
                    .boxed();
                return Ok(hyper::Response::builder()
                    .status(SECURITY_BLOCK_STATUS)
                    .body(seal_with_telemetry(deny_body, req_ctx))
                    .unwrap());
            }
            Err(error) => {
                let reason = format!("security engine error: {error}");
                let req_ctx = TelemetryRequestContext {
                    domain: domain.to_string(),
                    process_name: process_name.clone(),
                    ai_provider,
                    method: method.clone(),
                    path: path.clone(),
                    query: query.clone(),
                    status_code: Some(SECURITY_BLOCK_STATUS),
                    decision: Decision::Error,
                    matched_rule: Some(reason.clone()),
                    request_headers: Some(req_hdrs.clone()),
                    response_headers: None,
                    start_time,
                    request_body_stats: Arc::clone(&req_stats),
                    max_response_preview: 0,
                    port: upstream_port,
                    conn_type,
                    identity: telemetry_identity.clone(),
                    policy_mode: Some("runtime".into()),
                    policy_action: Some("error".into()),
                    policy_rule: None,
                    policy_reason: Some(reason.clone()),
                };
                let deny_body = Full::new(Bytes::from(format!("Capsem: {reason}\n")))
                    .map_err(|never| match never {})
                    .boxed();
                return Ok(hyper::Response::builder()
                    .status(SECURITY_BLOCK_STATUS)
                    .body(seal_with_telemetry(deny_body, req_ctx))
                    .unwrap());
            }
        }
    }

    let upstream_req_body: ProxyBoxBody = if let Some(body) = buffered_request_body {
        Full::new(body).map_err(|never| match never {}).boxed()
    } else {
        TrackedBody::new(
            req_body
                .take()
                .expect("request body should be present for streaming upstream body"),
            Arc::clone(&req_stats),
            100 * 1024 * 1024,
        )
        .boxed()
    };

    // Try to reuse a cached upstream sender, or create a new
    // connection. Each MITM connection serves one upstream via
    // keep-alive, so per-connection caching avoids re-establishing
    // TCP[+TLS] for every request.
    let upstream_lock_start = Instant::now();
    let mut reusable = cached_upstream.lock().await.take();
    let upstream_lock_us = upstream_lock_start.elapsed().as_micros() as u64;

    // If we have a cached sender, check it's still alive.
    let ready_us = if let Some(ref mut s) = reusable {
        let ready_start = Instant::now();
        if s.ready().await.is_err() {
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

    // Create a fresh upstream connection if needed. TLS path goes
    // TCP -> TLS handshake -> HTTP/1.1 handshake; HTTP path skips
    // the TLS step.
    let mut sender = if let Some(s) = reusable {
        s
    } else {
        let dial_start = Instant::now();
        let tcp_start = Instant::now();
        let connect_target = upstream_connect_target(domain, upstream_port);
        let upstream_tcp =
            match tokio::net::TcpStream::connect(connect_target.address.as_str()).await {
                Ok(tcp) => {
                    let _ = tcp.set_nodelay(true);
                    tcp
                }
                Err(e) => {
                    tcp_us = tcp_start.elapsed().as_micros() as u64;
                    tracing::debug!(
                        target: "mitm.transport.upstream",
                        domain, port = upstream_port, reused = false,
                        upstream_lock_us, ready_us, tcp_us,
                        error = %e, "upstream TCP connect failed"
                    );
                    ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                        .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                    ::metrics::counter!(metrics::REQUESTS_TOTAL,
                    "protocol" => protocol.label(), "decision" => "upstream_error")
                    .increment(1);
                    return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
                }
            };
        tcp_us = tcp_start.elapsed().as_micros() as u64;

        // TLS path: wrap TCP in a TLS stream, time the handshake.
        // HTTP path: skip TLS, hand the bare TCP stream to hyper.
        let (sender, hs_us) = match protocol {
            Protocol::Tls if connect_target.plaintext_tls => {
                ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                    .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                let upstream_io = TokioIo::new(upstream_tcp);
                let handshake_start = Instant::now();
                let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io).await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        ::metrics::counter!(metrics::REQUESTS_TOTAL,
                            "protocol" => protocol.label(), "decision" => "upstream_error")
                        .increment(1);
                        return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
                    }
                };
                let hs = handshake_start.elapsed().as_micros() as u64;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                (sender, hs)
            }
            Protocol::Tls => {
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));
                let server_name = match rustls::pki_types::ServerName::try_from(domain.to_string())
                {
                    Ok(sn) => sn,
                    Err(e) => {
                        return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
                    }
                };
                let tls_start = Instant::now();
                let upstream_tls_stream = match connector.connect(server_name, upstream_tcp).await {
                    Ok(tls) => {
                        ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                            .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                        tls
                    }
                    Err(e) => {
                        ::metrics::histogram!(metrics::UPSTREAM_DIAL_MS)
                            .record(dial_start.elapsed().as_secs_f64() * 1000.0);
                        ::metrics::counter!(metrics::REQUESTS_TOTAL,
                            "protocol" => protocol.label(), "decision" => "upstream_error")
                        .increment(1);
                        return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
                    }
                };
                tls_us = tls_start.elapsed().as_micros() as u64;
                let upstream_io = TokioIo::new(upstream_tls_stream);
                let handshake_start = Instant::now();
                let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io).await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
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
                let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io).await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        ::metrics::counter!(metrics::REQUESTS_TOTAL,
                            "protocol" => protocol.label(), "decision" => "upstream_error")
                        .increment(1);
                        return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
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

    tracing::debug!(
        target: "mitm.transport.upstream",
        domain, port = upstream_port, reused, upstream_lock_us, ready_us,
        tcp_us, tls_us, handshake_us,
        "upstream sender prepared"
    );

    // Build upstream request with original headers.
    let full_path = match &query {
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

    let resp = match sender.send_request(upstream_req).await {
        Ok(r) => r,
        Err(e) => {
            return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
        }
    };

    // Put the sender back in the cache for the next request on this connection.
    // The next request's ready().await will naturally wait until this response
    // body completes (hyper 1.x keep-alive semantics).
    cached_upstream.lock().await.replace(sender);
    let (mut resp_parts, resp_body) = resp.into_parts();

    let resp_status = resp_parts.status.as_u16();
    tracing::Span::current().record("status", resp_status);

    // Capture response headers BEFORE stripping Content-Encoding.
    // Telemetry logs still record the original headers (useful for debugging).
    let resp_hdrs = format_headers(&resp_parts.headers);

    // Strip Content-Encoding / Content-Length when the body is gzip --
    // the DecompressionHook (sync ChunkHook) handles the actual byte
    // transformation downstream. The guest receives uncompressed data
    // (vsock is local; compression is unnecessary). This header strip
    // is just three field accesses on the parts struct and stays
    // inline here -- moving it to an async Hook would re-introduce
    // the kind of plumbing the slice removed.
    let is_gzip = response_uses_gzip_content_encoding(&resp_parts.headers);
    if is_gzip {
        resp_parts.headers.remove("content-encoding");
        resp_parts.headers.remove("content-length");
    }

    // Pick the response-side preview cap. AI provider bodies always
    // capture at least AI_BODY_PREVIEW so non-streaming usage parsing
    // works even when log_bodies is off. Non-AI bodies follow the
    // log_bodies / max_body_capture policy.
    let resp_max_preview = if ai_provider.is_some() {
        AI_BODY_PREVIEW.max(if log_bodies { max_body } else { 0 })
    } else if log_bodies {
        max_body
    } else {
        0
    };

    let resp_body: ProxyBoxBody = resp_body.map_err(|e| -> anyhow::Error { e.into() }).boxed();

    let req_ctx = TelemetryRequestContext {
        domain: domain.to_string(),
        process_name: process_name.clone(),
        ai_provider,
        method,
        path,
        query,
        status_code: Some(resp_status),
        decision: Decision::Allowed,
        matched_rule: None,
        request_headers: Some(req_hdrs),
        response_headers: Some(resp_hdrs),
        start_time,
        request_body_stats: Arc::clone(&req_stats),
        max_response_preview: resp_max_preview,
        port: upstream_port,
        conn_type,
        identity: telemetry_identity,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
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
            ai_provider,
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
mod tests;
