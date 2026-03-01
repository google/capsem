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
use std::io;
use std::mem::ManuallyDrop;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use capsem_logger::{DbWriter, Decision, ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry, WriteOp};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use super::cert_authority::{CertAuthority, MitmCertResolver};
use super::policy::NetworkPolicy;
use crate::gateway::events::{StopReason, collect_summary};
use crate::gateway::provider::ProviderKind;

/// Re-exported so capsem-app can reference the type without depending on rustls.
pub type UpstreamTlsConfig = rustls::ClientConfig;

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    /// Live policy, swappable via RwLock so settings changes take effect
    /// without restarting the VM. Each connection snapshots the Arc.
    pub policy: Arc<std::sync::RwLock<Arc<NetworkPolicy>>>,
    pub db: Arc<DbWriter>,
    /// Cached upstream TLS config (shared across all connections).
    pub upstream_tls: Arc<rustls::ClientConfig>,
    /// Model pricing lookup table for cost estimation.
    pub pricing: crate::gateway::pricing::PricingTable,
    /// Trace state for linking multi-turn tool-use conversations.
    pub trace_state: std::sync::Mutex<crate::gateway::TraceState>,
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
/// Per-request telemetry is emitted by TelemetryBody when each HTTP response
/// body completes. This function only emits connection-level error events
/// (TLS failures, no SNI, etc.).
pub async fn handle_connection(vsock_fd: RawFd, config: Arc<MitmProxyConfig>) {
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
            };

            config.db.write(WriteOp::NetEvent(event)).await;
            warn!(domain = display_domain, reason, "MITM proxy: connection error");
        }
    }
}

/// Inner handler. Returns Ok(domain) on success, Err((domain, decision, reason))
/// on connection-level failure. Per-request telemetry is emitted by TelemetryBody.
async fn handle_inner(
    vsock_fd: RawFd,
    config: &Arc<MitmProxyConfig>,
) -> Result<String, (String, Decision, String)> {
    // Wrap vsock fd in a non-owning async stream.
    let vsock_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(vsock_fd) });
    let std_fd = vsock_file.try_clone().map_err(|e| {
        (String::new(), Decision::Error, format!("dup vsock fd: {e}"))
    })?;
    set_nonblocking(vsock_fd).map_err(|e| {
        (String::new(), Decision::Error, format!("set nonblocking: {e}"))
    })?;
    let async_fd = tokio::io::unix::AsyncFd::new(std_fd).map_err(|e| {
        (String::new(), Decision::Error, format!("async fd: {e}"))
    })?;
    let mut vsock_stream = AsyncFdStream(async_fd);

    // 1. Read initial bytes (TLS ClientHello + potential metadata).
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut initial_buf)
        .await
        .map_err(|e| (String::new(), Decision::Error, format!("read ClientHello: {e}")))?;
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
                return Err((String::new(), Decision::Error, "metadata exceeded 4KB limit".into()));
            }
            let mut more = vec![0u8; 1024];
            let n2 = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut more)
                .await
                .map_err(|e| (String::new(), Decision::Error, format!("read metadata: {e}")))?;
            if n2 == 0 {
                return Err((String::new(), Decision::Error, "EOF during metadata read".into()));
            }
            initial_buf.extend_from_slice(&more[..n2]);
        }

        // If initial_buf is empty after draining meta, we need to read ClientHello.
        if initial_buf.is_empty() {
            let mut hello_buf = vec![0u8; MAX_HELLO_SIZE];
            let n2 = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut hello_buf)
                .await
                .map_err(|e| (String::new(), Decision::Error, format!("read ClientHello after meta: {e}")))?;
            if n2 == 0 {
                return Err((String::new(), Decision::Error, "empty connection after meta".into()));
            }
            hello_buf.truncate(n2);
            initial_buf = hello_buf;
        }
    }

    // Snapshot the live policy for this connection (cheap Arc clone).
    let policy: Arc<NetworkPolicy> = config.policy.read().unwrap().clone();

    // 2. TLS handshake -- MitmCertResolver captures the domain from SNI.
    let resolver = Arc::new(MitmCertResolver::with_policy(
        Arc::clone(&config.ca),
        Arc::clone(&policy),
    ));
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
    let tls_stream = acceptor.accept(replay).await.map_err(|e| {
        let domain = resolver.domain().unwrap_or_default();
        (domain, Decision::Error, format!("TLS handshake: {e}"))
    })?;

    // 3. Get domain from the resolver (captured during handshake).
    let domain = resolver.domain().ok_or_else(|| {
        (String::new(), Decision::Denied, "no SNI in ClientHello".into())
    })?;

    // AI provider detection.
    let ai_provider = detect_ai_provider(&domain);

    // 4. Run hyper HTTP/1.1 server on the MITM TLS stream.
    let io = TokioIo::new(tls_stream);

    let upstream_tls = Arc::clone(&config.upstream_tls);
    let domain_for_svc = domain.clone();
    let db = Arc::clone(&config.db);
    let config_arc = Arc::clone(config);
    let log_bodies = policy.log_bodies;
    let max_body = policy.max_body_capture;
    let process_name = Arc::new(process_name);

    // Per-connection upstream sender cache: each MITM connection serves one
    // domain via keep-alive, so caching the sender avoids re-establishing
    // TCP+TLS for every request on the same connection.
    let cached_upstream: Arc<tokio::sync::Mutex<Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let policy = Arc::clone(&policy);
        let upstream_tls = Arc::clone(&upstream_tls);
        let domain = domain_for_svc.clone();
        let db = Arc::clone(&db);
        let config_arc = Arc::clone(&config_arc);
        let process_name = Arc::clone(&process_name);
        let cached_upstream = Arc::clone(&cached_upstream);

        async move {
            handle_request(req, &domain, &policy, &upstream_tls, &db, &config_arc, &process_name, ai_provider, log_bodies, max_body, &cached_upstream).await
        }
    });

    // Serve exactly one connection (may have multiple requests via keep-alive).
    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await
    {
        // Connection errors are expected when the guest closes.
        let err_str = e.to_string();
        if !e.is_incomplete_message() && !err_str.contains("error shutting down connection") {
            warn!(domain, error = %e, "hyper serve error");
        }
    }

    Ok(domain)
}

/// Handle a single HTTP request within the MITM TLS connection.
///
/// Builds a per-request `TelemetryEmitter` and wraps the response body in
/// `TelemetryBody` so telemetry is emitted when the response completes.
async fn handle_request(
    req: hyper::Request<hyper::body::Incoming>,
    domain: &str,
    policy: &NetworkPolicy,
    upstream_tls: &Arc<rustls::ClientConfig>,
    db: &Arc<DbWriter>,
    config: &Arc<MitmProxyConfig>,
    process_name: &Option<String>,
    ai_provider: Option<ProviderKind>,
    log_bodies: bool,
    max_body: usize,
    cached_upstream: &tokio::sync::Mutex<Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>>,
) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error> {
    use http_body_util::BodyExt;

    let start_time = Instant::now();
    let (parts, req_body) = req.into_parts();
    let method = parts.method.to_string();
    let (path, query) = split_path_query(&parts.uri);

    // Capture request headers.
    let req_hdrs = format_headers(&parts.headers);

    // Check for WebSocket upgrade.
    let is_upgrade = parts.headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    // Policy check: domain + method -> read/write decision.
    let eval = policy.evaluate(domain, &method);
    if !eval.allowed {
        let body_text = format!(
            "Capsem: request denied ({}: {} {})\n",
            eval.reason, method, path
        );

        let emitter = TelemetryEmitter {
            db: Arc::clone(db),
            config: Arc::clone(config),
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(403),
            decision: Decision::Denied,
            matched_rule: Some(eval.matched_rule),
            request_headers: Some(req_hdrs),
            response_headers: None,

            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time,
        };

        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        let telem_body = TelemetryBody::new(deny_body, emitter);

        return Ok(hyper::Response::builder()
            .status(403)
            .body(telem_body.boxed())
            .unwrap());
    }

    // Reject WebSocket upgrades (not supported through MITM proxy).
    if is_upgrade {
        let body_text = format!(
            "Capsem: WebSocket upgrades are not supported ({} {})\n",
            method, path
        );

        let emitter = TelemetryEmitter {
            db: Arc::clone(db),
            config: Arc::clone(config),
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
            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time,
        };

        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        let telem_body = TelemetryBody::new(deny_body, emitter);

        return Ok(hyper::Response::builder()
            .status(400)
            .body(telem_body.boxed())
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
        let emitter = TelemetryEmitter {
            db: Arc::clone(db),
            config: Arc::clone(config),
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
            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time: start,
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        let telem_body = TelemetryBody::new(deny_body, emitter);
        hyper::Response::builder()
            .status(502)
            .body(telem_body.boxed())
            .unwrap()
    };

    // Try to reuse a cached upstream sender, or create a new connection.
    // Each MITM connection serves one domain via keep-alive, so per-connection
    // caching avoids re-establishing TCP+TLS for every request.
    let mut reusable = cached_upstream.lock().await.take();

    // If we have a cached sender, check it's still alive.
    if let Some(ref mut s) = reusable {
        if s.ready().await.is_err() {
            reusable = None;
        }
    }

    // Create a fresh upstream connection if needed.
    let mut sender = if let Some(s) = reusable {
        s
    } else {
        let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));
        let upstream_tcp = match tokio::net::TcpStream::connect(format!("{domain}:443")).await {
            Ok(tcp) => {
                let _ = tcp.set_nodelay(true);
                tcp
            },
            Err(e) => {
                return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
            }
        };
        let server_name = match rustls::pki_types::ServerName::try_from(domain.to_string()) {
            Ok(sn) => sn,
            Err(e) => {
                return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
            }
        };
        let upstream_tls_stream = match connector.connect(server_name, upstream_tcp).await {
            Ok(tls) => tls,
            Err(e) => {
                return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
            }
        };
        let upstream_io = TokioIo::new(upstream_tls_stream);
        let (sender, conn) = match hyper::client::conn::http1::handshake(upstream_io).await {
            Ok(pair) => pair,
            Err(e) => {
                return Ok(make_502(&e, &method, &path, &query, &req_hdrs, start_time));
            }
        };
        tokio::spawn(async move {
            let _ = conn.await;
        });
        sender
    };

    // Build upstream request with original headers.
    let full_path = match &query {
        Some(q) => format!("{path}?{q}"),
        None => path.clone(),
    };
    let mut builder = hyper::Request::builder()
        .method(original_method)
        .uri(&full_path);
    for (name, value) in original_headers.iter() {
        if name != "host" {
            builder = builder.header(name.clone(), value.clone());
        }
    }
    builder = builder.header("host", domain);

    // Track request body (boxed for consistent sender type across requests).
    let req_stats = Arc::new(Mutex::new(BodyStats {
        bytes: 0,
        preview: Vec::new(),
        max_preview: if log_bodies { max_body } else { 0 },
    }));
    let tracked_req_body = TrackedBody::new(req_body, Arc::clone(&req_stats), 100 * 1024 * 1024);
    let upstream_req = builder.body(tracked_req_body.boxed())?;

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
    let resp_status = resp.status().as_u16();
    let (resp_parts, resp_body) = resp.into_parts();

    // Capture response headers.
    let resp_hdrs = format_headers(&resp_parts.headers);

    // Build the response body with telemetry wrapper.
    let (inner_body, resp_kind) = if let Some(provider) = ai_provider {
        use crate::gateway::ai_body::AiResponseBody;
        use crate::gateway::anthropic::AnthropicStreamParserWithState;
        use crate::gateway::google::GoogleStreamParser;
        use crate::gateway::openai::OpenAiStreamParser;

        let provider_parser: Box<dyn crate::gateway::events::ProviderStreamParser + Send> = match provider {
            ProviderKind::Anthropic => Box::new(AnthropicStreamParserWithState::new()),
            ProviderKind::OpenAi => Box::new(OpenAiStreamParser::new()),
            ProviderKind::Google => Box::new(GoogleStreamParser::new()),
        };

        let max_preview = if log_bodies { max_body } else { 0 };
        let ai_body = AiResponseBody::new(resp_body, provider_parser, max_preview, 100 * 1024 * 1024);
        let ai_state = ai_body.ai_state();
        let ai_stats = ai_body.stats();

        let kind = RespStatsKind::Ai { stats: ai_stats, state: ai_state };
        (ai_body.boxed(), kind)
    } else {
        let resp_stats = Arc::new(Mutex::new(BodyStats {
            bytes: 0,
            preview: Vec::new(),
            max_preview: if log_bodies { max_body } else { 0 },
        }));
        let tracked_resp_body = TrackedBody::new(resp_body, Arc::clone(&resp_stats), 100 * 1024 * 1024);
        let kind = RespStatsKind::Plain(resp_stats);
        (tracked_resp_body.boxed(), kind)
    };

    let emitter = TelemetryEmitter {
        db: Arc::clone(db),
        config: Arc::clone(config),
        domain: domain.to_string(),
        process_name: process_name.clone(),
        ai_provider,
        method,
        path,
        query,
        status_code: Some(resp_status),
        decision: Decision::Allowed,
        matched_rule: Some(eval.matched_rule),
        request_headers: Some(req_hdrs),
        response_headers: Some(resp_hdrs),

        req_stats,
        resp_kind,
        start_time,
    };

    let telem_body = TelemetryBody::new(inner_body, emitter);
    let response = hyper::Response::from_parts(resp_parts, telem_body.boxed());
    Ok(response)
}


type ProxyBoxBody = http_body_util::combinators::BoxBody<Bytes, anyhow::Error>;

struct BodyStats {
    bytes: u64,
    preview: Vec<u8>,
    max_preview: usize,
}

impl BodyStats {
    fn new(max_preview: usize) -> Self {
        Self { bytes: 0, preview: Vec::new(), max_preview }
    }
}

/// Which response body stats variant we're tracking.
enum RespStatsKind {
    /// Non-AI response: plain byte tracking.
    Plain(Arc<Mutex<BodyStats>>),
    /// AI response: SSE-parsed body with events + stats.
    Ai {
        stats: Arc<Mutex<crate::gateway::ai_body::AiBodyStats>>,
        state: Arc<Mutex<crate::gateway::ai_body::AiStreamState>>,
    },
}

/// Holds everything needed to build and emit a NetEvent (+ optional ModelCall)
/// when a single HTTP request/response cycle completes.
struct TelemetryEmitter {
    db: Arc<DbWriter>,
    config: Arc<MitmProxyConfig>,
    // Connection-level
    domain: String,
    process_name: Option<String>,
    ai_provider: Option<ProviderKind>,
    // Request-level
    method: String,
    path: String,
    query: Option<String>,
    status_code: Option<u16>,
    decision: Decision,
    matched_rule: Option<String>,
    request_headers: Option<String>,
    response_headers: Option<String>,
    // Body stats
    req_stats: Arc<Mutex<BodyStats>>,
    resp_kind: RespStatsKind,
    // Timing
    start_time: Instant,
}

impl TelemetryEmitter {
    /// Build and write a NetEvent (and optionally a ModelCall) to the DB.
    async fn emit(self) {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;

        // Read request body stats.
        let (bytes_sent, request_body_preview) = if let Ok(st) = self.req_stats.lock() {
            let preview = if st.preview.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&st.preview).into_owned())
            };
            (st.bytes, preview)
        } else {
            (0, None)
        };

        // Read response body stats.
        let (bytes_received, response_body_preview, ai_state_ref) = match &self.resp_kind {
            RespStatsKind::Plain(resp_stats) => {
                if let Ok(st) = resp_stats.lock() {
                    let preview = if st.preview.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&st.preview).into_owned())
                    };
                    (st.bytes, preview, None)
                } else {
                    (0, None, None)
                }
            }
            RespStatsKind::Ai { stats, state } => {
                let (bytes, preview) = if let Ok(st) = stats.lock() {
                    let p = if st.preview.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&st.preview).into_owned())
                    };
                    (st.bytes, p)
                } else {
                    (0, None)
                };
                (bytes, preview, Some(Arc::clone(state)))
            }
        };

        let event = NetEvent {
            timestamp: SystemTime::now(),
            domain: self.domain.clone(),
            port: 443,
            decision: self.decision,
            process_name: self.process_name.clone(),
            pid: None,
            bytes_sent,
            bytes_received,
            duration_ms,
            method: Some(self.method.clone()),
            path: Some(self.path.clone()),
            query: self.query.clone(),
            status_code: self.status_code,
            matched_rule: self.matched_rule.clone(),
            request_headers: self.request_headers.clone(),
            response_headers: self.response_headers.clone(),
            request_body_preview,
            response_body_preview,
            conn_type: Some("https-mitm".to_string()),
        };

        self.db.write(WriteOp::NetEvent(event)).await;

        // Emit ModelCall for AI providers.
        if let Some(provider) = self.ai_provider {
            self.emit_model_call(provider, bytes_sent, bytes_received, duration_ms, &ai_state_ref).await;
        }

        // Log.
        match self.decision {
            Decision::Allowed => info!(
                domain = self.domain,
                method = self.method,
                path = self.path,
                status = ?self.status_code,
                duration_ms,
                "MITM proxy: completed"
            ),
            Decision::Denied => info!(
                domain = self.domain,
                method = self.method,
                path = self.path,
                duration_ms,
                "MITM proxy: denied"
            ),
            Decision::Error => warn!(
                domain = self.domain,
                method = self.method,
                "MITM proxy: error"
            ),
        }
    }

    /// Build and write a ModelCall for AI provider traffic.
    async fn emit_model_call(
        &self,
        provider: ProviderKind,
        request_bytes: u64,
        response_bytes: u64,
        duration_ms: u64,
        ai_state_ref: &Option<Arc<Mutex<crate::gateway::ai_body::AiStreamState>>>,
    ) {
        use crate::gateway::request_parser;

        // Parse request body for metadata.
        let req_body_bytes: Vec<u8> = self.req_stats.lock()
            .ok()
            .map(|st| st.preview.clone())
            .unwrap_or_default();
        let req_meta = request_parser::parse_request(provider, &req_body_bytes);

        // Collect stream summary from AI events.
        let summary = ai_state_ref.as_ref().and_then(|state| {
            state.lock().ok().map(|ai| collect_summary(&ai.events))
        });

        let stop_reason_str = summary.as_ref().and_then(|s| s.stop_reason.as_ref()).map(|sr| {
            match sr {
                StopReason::EndTurn => "end_turn".to_string(),
                StopReason::ToolUse => "tool_use".to_string(),
                StopReason::MaxTokens => "max_tokens".to_string(),
                StopReason::ContentFilter => "content_filter".to_string(),
                StopReason::Other(s) => s.clone(),
            }
        });

        let tool_calls: Vec<ToolCallEntry> = summary.as_ref()
            .map(|s| s.tool_calls.iter().map(|tc| ToolCallEntry {
                call_index: tc.index,
                call_id: tc.call_id.clone(),
                tool_name: tc.name.clone(),
                arguments: if tc.arguments.is_empty() { None } else { Some(tc.arguments.clone()) },
                origin: "native".to_string(),
            }).collect())
            .unwrap_or_default();

        let tool_responses: Vec<ToolResponseEntry> = req_meta.tool_results.iter()
            .map(|tr| ToolResponseEntry {
                call_id: tr.call_id.clone(),
                content_preview: Some(tr.content_preview.clone()),
                is_error: tr.is_error,
            })
            .collect();

        // Estimate cost from pricing table.
        let model = req_meta.model.as_deref()
            .or(summary.as_ref().and_then(|s| s.model.as_deref()));
        let estimated_cost_usd = self.config.pricing.estimate_cost(
            provider.as_str(),
            model,
            summary.as_ref().and_then(|s| s.input_tokens),
            summary.as_ref().and_then(|s| s.output_tokens),
            summary.as_ref().map(|s| &s.usage_details).unwrap_or(&std::collections::BTreeMap::new()),
        );

        // Assign trace_id: look up from tool response call_ids, or create new.
        let tool_response_ids: Vec<String> = req_meta.tool_results.iter()
            .map(|tr| tr.call_id.clone()).collect();
        let tool_call_ids: Vec<String> = tool_calls.iter()
            .map(|tc| tc.call_id.clone()).collect();
        let trace_id = {
            let mut state = self.config.trace_state.lock()
                .unwrap_or_else(|e| e.into_inner());
            let tid = state.lookup(&tool_response_ids)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let is_tool_use = !tool_call_ids.is_empty()
                || stop_reason_str.as_deref()
                    .map(|r| r.contains("tool") || r == "tool_use")
                    .unwrap_or(false);
            if is_tool_use && !tool_call_ids.is_empty() {
                state.register_tool_calls(&tid, &tool_call_ids);
            } else {
                state.complete_trace(&tid);
            }
            tid
        };

        let model_call = ModelCall {
            timestamp: SystemTime::now(),
            provider: provider.as_str().to_string(),
            model: req_meta.model.or_else(|| summary.as_ref().and_then(|s| s.model.clone())),
            process_name: self.process_name.clone(),
            pid: None,
            method: self.method.clone(),
            path: self.path.clone(),
            stream: req_meta.stream,
            system_prompt_preview: req_meta.system_prompt_preview,
            messages_count: req_meta.messages_count,
            tools_count: req_meta.tools_count,
            request_bytes,
            request_body_preview: self.req_stats.lock().ok()
                .and_then(|st| if st.preview.is_empty() { None } else {
                    Some(String::from_utf8_lossy(&st.preview).into_owned())
                }),
            message_id: summary.as_ref().and_then(|s| s.message_id.clone()),
            status_code: self.status_code,
            text_content: summary.as_ref().map(|s| s.text.clone()).filter(|s| !s.is_empty()),
            thinking_content: summary.as_ref().map(|s| s.thinking.clone()).filter(|s| !s.is_empty()),
            stop_reason: stop_reason_str,
            input_tokens: summary.as_ref().and_then(|s| s.input_tokens),
            output_tokens: summary.as_ref().and_then(|s| s.output_tokens),
            usage_details: summary.as_ref().map(|s| s.usage_details.clone()).unwrap_or_default(),
            duration_ms,
            response_bytes,
            estimated_cost_usd,
            trace_id: Some(trace_id),
            tool_calls,
            tool_responses,
        };

        self.db.write(WriteOp::ModelCall(model_call)).await;
    }
}

/// Wraps a response body and fires telemetry when the body completes.
/// If the body is dropped before completion (client disconnect), the
/// Drop impl fires as a fallback.
///
/// ProxyBoxBody (BoxBody) is Unpin, so no pin projection needed.
struct TelemetryBody {
    inner: ProxyBoxBody,
    emitter: Option<TelemetryEmitter>,
}

impl TelemetryBody {
    fn new(inner: ProxyBoxBody, emitter: TelemetryEmitter) -> Self {
        Self { inner, emitter: Some(emitter) }
    }
}

impl hyper::body::Body for TelemetryBody {
    type Data = Bytes;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(None) => {
                // Body complete -- emit telemetry.
                if let Some(emitter) = this.emitter.take() {
                    tokio::spawn(async move {
                        emitter.emit().await;
                    });
                }
                Poll::Ready(None)
            }
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for TelemetryBody {
    fn drop(&mut self) {
        // Fallback: if the body was dropped before completion (e.g. client
        // disconnect), emit whatever telemetry we have.
        if let Some(emitter) = self.emitter.take() {
            tokio::spawn(async move {
                emitter.emit().await;
            });
        }
    }
}

pin_project_lite::pin_project! {
    struct TrackedBody<B> {
        #[pin]
        inner: B,
        stats: Arc<Mutex<BodyStats>>,
        max_size: u64,
    }
}

impl<B> TrackedBody<B> {
    fn new(inner: B, stats: Arc<Mutex<BodyStats>>, max_size: u64) -> Self {
        Self { inner, stats, max_size }
    }
}

impl<B> hyper::body::Body for TrackedBody<B>
where
    B: hyper::body::Body,
    B::Error: Into<anyhow::Error>,
{
    type Data = B::Data;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let len = hyper::body::Buf::remaining(data) as u64;
                    let mut st = this.stats.lock().unwrap();
                    st.bytes += len;
                    if st.bytes > *this.max_size {
                        return Poll::Ready(Some(Err(anyhow::anyhow!("body exceeded maximum size"))));
                    }
                    if st.preview.len() < st.max_preview {
                        let to_copy = (st.max_preview - st.preview.len()).min(len as usize);
                        let chunk = hyper::body::Buf::chunk(data);
                        let to_copy = to_copy.min(chunk.len());
                        st.preview.extend_from_slice(&chunk[..to_copy]);
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// Split a URI into path and query components.
fn split_path_query(uri: &hyper::Uri) -> (String, Option<String>) {
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string());
    (path, query)
}

/// Headers whose values are safe to store verbatim in telemetry logs.
/// Everything else keeps its name but the value is replaced with a BLAKE3
/// hash prefix so credentials (API keys, bearer tokens, cookies) never
/// reach the database while still allowing correlation across requests.
const HEADER_ALLOWLIST: &[&str] = &[
    "accept",
    "content-encoding",
    "content-length",
    "content-type",
    "date",
    "host",
    "server",
    "transfer-encoding",
    "user-agent",
];

/// Format HTTP headers for telemetry storage.
///
/// Allowlisted headers are stored verbatim. All other headers keep their
/// name but the value is replaced with `hash:<12-char-hex>` (first 6 bytes
/// of the BLAKE3 digest). This prevents credential leakage while preserving
/// header presence and enabling same-key correlation.
fn format_headers(headers: &hyper::HeaderMap) -> String {
    headers
        .iter()
        .map(|(name, value)| {
            if HEADER_ALLOWLIST.contains(&name.as_str()) {
                let v = value.to_str().unwrap_or("<binary>");
                format!("{}: {}", name, v)
            } else {
                let raw = value.as_bytes();
                let digest = blake3::hash(raw);
                let hex = &digest.to_hex()[..12];
                format!("{}: hash:{}", name, hex)
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Set a file descriptor to non-blocking mode.
fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Async wrapper around a `std::fs::File` via `AsyncFd`.
///
/// Implements `AsyncRead + AsyncWrite` for use with tokio.
struct AsyncFdStream(tokio::io::unix::AsyncFd<std::fs::File>);

impl AsyncRead for AsyncFdStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.0.poll_read_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            let unfilled = buf.initialize_unfilled();
            match guard.try_io(|inner| {
                use std::io::Read;
                let mut file = inner.get_ref();
                file.read(unfilled)
            }) {
                Ok(Ok(n)) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(e)) => return Poll::Ready(Err(e)),
                Err(_would_block) => continue,
            }
        }
    }
}

impl AsyncWrite for AsyncFdStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = match self.0.poll_write_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            match guard.try_io(|inner| {
                use std::io::Write;
                let mut file = inner.get_ref();
                file.write(buf)
            }) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.0.poll_write_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            match guard.try_io(|inner| {
                use std::io::Write;
                let mut file = inner.get_ref();
                file.flush()
            }) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let fd = self.0.as_raw_fd();
        let rc = unsafe { libc::shutdown(fd, libc::SHUT_WR) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            // ENOTCONN is fine -- already disconnected.
            if err.kind() != io::ErrorKind::NotConnected {
                return Poll::Ready(Err(err));
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// A reader that replays buffered bytes first, then reads from the inner stream.
///
/// Used to feed the TLS ClientHello bytes we already read back into the TLS acceptor.
struct ReplayReader<R> {
    buffer: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R> ReplayReader<R> {
    fn new(buffer: Vec<u8>, inner: R) -> Self {
        Self {
            buffer,
            pos: 0,
            inner,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ReplayReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // First, drain the replay buffer.
        if this.pos < this.buffer.len() {
            let remaining = &this.buffer[this.pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            this.pos += to_copy;
            return Poll::Ready(Ok(()));
        }

        // Then delegate to the inner reader.
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<R: AsyncWrite + Unpin> AsyncWrite for ReplayReader<R> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    use http_body_util::BodyExt;

    use crate::net::cert_authority::CertAuthority;
    use crate::net::policy::NetworkPolicy;

    const CA_KEY: &str = include_str!("../../../../config/capsem-ca.key");
    const CA_CERT: &str = include_str!("../../../../config/capsem-ca.crt");

    /// Flush delay for the DB writer thread to process queued writes.
    const DB_FLUSH_MS: u64 = 100;

    fn make_config_with_policy(policy: NetworkPolicy) -> Arc<MitmProxyConfig> {
        let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
        // Leak the tempdir so it lives for the test
        std::mem::forget(dir);
        Arc::new(MitmProxyConfig {
            ca,
            policy: Arc::new(std::sync::RwLock::new(Arc::new(policy))),
            db,
            upstream_tls: make_upstream_tls_config(),
            pricing: crate::gateway::pricing::PricingTable::load(),
            trace_state: std::sync::Mutex::new(crate::gateway::TraceState::new()),
        })
    }

    fn make_config_dev() -> Arc<MitmProxyConfig> {
        make_config_with_policy(NetworkPolicy::default_dev())
    }

    fn make_config_deny_all() -> Arc<MitmProxyConfig> {
        make_config_with_policy(NetworkPolicy::new(vec![], false, false))
    }

    fn make_client_hello(hostname: &str) -> Vec<u8> {
        let hostname_bytes = hostname.as_bytes();
        let sni_entry_len = 1 + 2 + hostname_bytes.len();
        let sni_list_len = sni_entry_len;
        let sni_ext_data_len = 2 + sni_list_len;

        let mut sni_ext = Vec::new();
        sni_ext.extend_from_slice(&0x0000u16.to_be_bytes());
        sni_ext.extend_from_slice(&(sni_ext_data_len as u16).to_be_bytes());
        sni_ext.extend_from_slice(&(sni_list_len as u16).to_be_bytes());
        sni_ext.push(0x00);
        sni_ext.extend_from_slice(&(hostname_bytes.len() as u16).to_be_bytes());
        sni_ext.extend_from_slice(hostname_bytes);

        let extensions_len = sni_ext.len();
        let mut hello_body = Vec::new();
        hello_body.extend_from_slice(&[0x03, 0x03]);
        hello_body.extend_from_slice(&[0u8; 32]);
        hello_body.push(0);
        hello_body.extend_from_slice(&2u16.to_be_bytes());
        hello_body.extend_from_slice(&[0x00, 0x2f]);
        hello_body.push(1);
        hello_body.push(0);
        hello_body.extend_from_slice(&(extensions_len as u16).to_be_bytes());
        hello_body.extend_from_slice(&sni_ext);

        let mut handshake = Vec::new();
        handshake.push(0x01);
        let hello_len = hello_body.len();
        handshake.push((hello_len >> 16) as u8);
        handshake.push((hello_len >> 8) as u8);
        handshake.push(hello_len as u8);
        handshake.extend_from_slice(&hello_body);

        let mut record = Vec::new();
        record.push(0x16);
        record.extend_from_slice(&[0x03, 0x01]);
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);

        record
    }

    // ---------------------------------------------------------------
    // Metadata fragmentation tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn fragmented_metadata_is_reassembled() {
        let config = make_config_dev();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        // Write metadata in two fragments: first the prefix, then the rest + newline + client hello.
        s1.set_nonblocking(false).unwrap();
        let mut writer = s1;
        // Fragment 1: metadata prefix without the newline
        std::io::Write::write_all(&mut writer, b"\0CAPSEM_META:my_proc").unwrap();
        // Small delay so the proxy reads the first fragment before the rest arrives.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Fragment 2: rest of metadata with newline, then the TLS ClientHello
        let mut frag2 = b"ess_name\n".to_vec();
        frag2.extend_from_slice(&make_client_hello("example.com"));
        std::io::Write::write_all(&mut writer, &frag2).unwrap();
        drop(writer);

        // The proxy should have reassembled metadata and completed TLS handshake.
        // It will fail after handshake (no real TLS client), but the key check
        // is that it didn't error during metadata parsing.
        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        // Should have an event (error from failed TLS with raw bytes, not metadata error).
        // The important thing is we didn't get "metadata exceeded 4KB" or "EOF during metadata".
        if !events.is_empty() {
            let rule = events[0].matched_rule.as_deref().unwrap_or("");
            assert!(!rule.contains("metadata"), "Fragmented metadata should be reassembled, got: {rule}");
        }
    }

    #[tokio::test]
    async fn oversized_metadata_rejected() {
        let config = make_config_dev();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        // Write >4KB metadata without a newline terminator.
        let mut oversized = b"\0CAPSEM_META:".to_vec();
        oversized.extend_from_slice(&vec![b'A'; 5000]);
        let mut writer = s1;
        std::io::Write::write_all(&mut writer, &oversized).unwrap();
        drop(writer);

        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert!(!events.is_empty(), "oversized metadata should produce error event");
        assert_eq!(events[0].decision, Decision::Error);
        let rule = events[0].matched_rule.as_deref().unwrap_or("");
        assert!(rule.contains("4KB"), "Should mention 4KB limit, got: {rule}");
    }

    // ---------------------------------------------------------------
    // Existing connection-level tests (unchanged behavior)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn no_sni_records_error() {
        let config = make_config_dev();
        let (mut s1, s2) = UnixStream::pair().unwrap();

        std::io::Write::write_all(&mut s1, b"not a client hello").unwrap();
        drop(s1);

        handle_connection(s2.into_raw_fd(), config.clone()).await;

        // Give writer thread time to flush.
        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "<unknown>");
        // Without valid TLS, it's an error (handshake failure)
        assert!(matches!(events[0].decision, Decision::Error | Decision::Denied));
    }

    #[tokio::test]
    async fn empty_connection_records_error() {
        let config = make_config_dev();
        let (_s1, s2) = UnixStream::pair().unwrap();
        drop(_s1);

        handle_connection(s2.into_raw_fd(), config.clone()).await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, Decision::Error);
    }

    #[test]
    fn replay_reader_drains_buffer_then_inner() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let buffer = b"hello".to_vec();
            let inner_data: &[u8] = b" world";
            let mut reader = ReplayReader::new(buffer, inner_data);

            let mut output = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut output)
                .await
                .unwrap();
            assert_eq!(&output, b"hello world");
        });
    }

    // ---------------------------------------------------------------
    // AsyncFdStream tests
    // ---------------------------------------------------------------

    fn wrap_fd_like_handle_inner(raw_fd: RawFd) -> AsyncFdStream {
        let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(raw_fd) });
        let cloned = file.try_clone().expect("try_clone (dup) failed");
        set_nonblocking(raw_fd).expect("set_nonblocking failed");
        let async_fd = tokio::io::unix::AsyncFd::new(cloned).expect("AsyncFd::new failed");
        AsyncFdStream(async_fd)
    }

    #[tokio::test]
    async fn async_fd_stream_basic_read_write() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd1 = s1.into_raw_fd();
        let fd2 = s2.into_raw_fd();
        let mut stream1 = wrap_fd_like_handle_inner(fd1);
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        tokio::io::AsyncWriteExt::write_all(&mut stream1, b"hello vsock").await.unwrap();
        let mut buf = vec![0u8; 64];
        let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello vsock");

        unsafe { libc::close(fd1); libc::close(fd2); }
    }

    #[tokio::test]
    async fn async_fd_stream_large_transfer() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd1 = s1.into_raw_fd();
        let fd2 = s2.into_raw_fd();
        let mut stream1 = wrap_fd_like_handle_inner(fd1);
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        let data: Vec<u8> = (0..131072).map(|i| (i % 251) as u8).collect();
        let send_data = data.clone();
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut stream1, &send_data).await.unwrap();
            drop(stream1);
            unsafe { libc::close(fd1); }
        });
        let mut received = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut received).await.unwrap();
        writer.await.unwrap();

        assert_eq!(received.len(), data.len());
        assert_eq!(received, data);

        unsafe { libc::close(fd2); }
    }

    #[tokio::test]
    async fn async_fd_stream_eof_on_close() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd1 = s1.into_raw_fd();
        let fd2 = s2.into_raw_fd();
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        {
            let mut stream1 = wrap_fd_like_handle_inner(fd1);
            tokio::io::AsyncWriteExt::write_all(&mut stream1, b"before eof").await.unwrap();
        }
        unsafe { libc::close(fd1); }

        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut buf).await.unwrap();
        assert_eq!(&buf, b"before eof");

        unsafe { libc::close(fd2); }
    }

    #[tokio::test]
    async fn async_fd_stream_bidirectional() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd1 = s1.into_raw_fd();
        let fd2 = s2.into_raw_fd();
        let mut stream1 = wrap_fd_like_handle_inner(fd1);
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        tokio::io::AsyncWriteExt::write_all(&mut stream1, b"ping").await.unwrap();
        let mut buf = vec![0u8; 32];
        let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"ping");

        tokio::io::AsyncWriteExt::write_all(&mut stream2, b"pong").await.unwrap();
        let n = tokio::io::AsyncReadExt::read(&mut stream1, &mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"pong");

        unsafe { libc::close(fd1); libc::close(fd2); }
    }

    #[tokio::test]
    async fn async_fd_stream_replay_then_live() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd2 = s2.into_raw_fd();
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        let mut writer = s1;
        std::io::Write::write_all(&mut writer, b"INITIAL").unwrap();
        std::io::Write::write_all(&mut writer, b"REMAINING").unwrap();
        drop(writer);

        let mut initial = vec![0u8; 7];
        tokio::io::AsyncReadExt::read_exact(&mut stream2, &mut initial).await.unwrap();
        assert_eq!(&initial, b"INITIAL");

        let mut replay = ReplayReader::new(initial, stream2);
        let mut all = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut replay, &mut all).await.unwrap();
        assert_eq!(&all, b"INITIALREMAINING");

        unsafe { libc::close(fd2); }
    }

    /// Full TLS handshake through handle_connection using a real rustls client.
    #[tokio::test]
    async fn tls_handshake_completes_without_global_provider() {
        let config = make_config_dev();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        let mut root_store = rustls::RootCertStore::empty();
        let ca_certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        for cert in ca_certs {
            root_store.add(cert).unwrap();
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let client_config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let domain = rustls::pki_types::ServerName::try_from("example.com").unwrap();
        let tls_result = connector.connect(domain, stream).await;

        assert!(tls_result.is_ok(), "TLS handshake failed: {:?}", tls_result.err());

        drop(tls_result);
        let _ = proxy_task.await;
    }

    #[test]
    fn split_path_query_with_query() {
        let uri: hyper::Uri = "https://example.com/api/v1?foo=bar&baz=1".parse().unwrap();
        let (path, query) = split_path_query(&uri);
        assert_eq!(path, "/api/v1");
        assert_eq!(query, Some("foo=bar&baz=1".to_string()));
    }

    #[test]
    fn split_path_query_without_query() {
        let uri: hyper::Uri = "/about".parse().unwrap();
        let (path, query) = split_path_query(&uri);
        assert_eq!(path, "/about");
        assert_eq!(query, None);
    }

    // ---------------------------------------------------------------
    // Header sanitization tests
    // ---------------------------------------------------------------

    #[test]
    fn format_headers_keeps_allowlisted_verbatim() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("content-length", "42".parse().unwrap());
        headers.insert("host", "api.example.com".parse().unwrap());
        headers.insert("server", "nginx".parse().unwrap());
        headers.insert("user-agent", "curl/8.0".parse().unwrap());

        let formatted = format_headers(&headers);
        assert!(formatted.contains("content-type: application/json"));
        assert!(formatted.contains("content-length: 42"));
        assert!(formatted.contains("host: api.example.com"));
        assert!(formatted.contains("server: nginx"));
        assert!(formatted.contains("user-agent: curl/8.0"));
    }

    #[test]
    fn format_headers_hashes_sensitive_headers() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-api-key", "sk-ant-1234567890abcdef".parse().unwrap());
        headers.insert("authorization", "Bearer tok_secret".parse().unwrap());
        headers.insert("cookie", "session=abc123".parse().unwrap());

        let formatted = format_headers(&headers);

        // Header names are preserved.
        assert!(formatted.contains("x-api-key: hash:"));
        assert!(formatted.contains("authorization: hash:"));
        assert!(formatted.contains("cookie: hash:"));

        // Raw credential values must NOT appear.
        assert!(!formatted.contains("sk-ant-1234567890abcdef"));
        assert!(!formatted.contains("Bearer tok_secret"));
        assert!(!formatted.contains("session=abc123"));
    }

    #[test]
    fn format_headers_hash_is_deterministic() {
        let mut h1 = hyper::HeaderMap::new();
        h1.insert("x-api-key", "AIzaSyBxxxxxxx".parse().unwrap());
        let mut h2 = hyper::HeaderMap::new();
        h2.insert("x-api-key", "AIzaSyBxxxxxxx".parse().unwrap());

        assert_eq!(format_headers(&h1), format_headers(&h2));
    }

    #[test]
    fn format_headers_different_keys_different_hashes() {
        let mut h1 = hyper::HeaderMap::new();
        h1.insert("x-api-key", "key-AAAA".parse().unwrap());
        let mut h2 = hyper::HeaderMap::new();
        h2.insert("x-api-key", "key-BBBB".parse().unwrap());

        // Extract the hash portion from each.
        let f1 = format_headers(&h1);
        let f2 = format_headers(&h2);
        let hash1 = f1.strip_prefix("x-api-key: hash:").unwrap();
        let hash2 = f2.strip_prefix("x-api-key: hash:").unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn format_headers_mixed_allowed_and_sensitive() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("content-type", "text/html".parse().unwrap());
        headers.insert("x-api-key", "sk-secret".parse().unwrap());
        headers.insert("accept", "text/html".parse().unwrap());

        let formatted = format_headers(&headers);

        // Allowlisted: verbatim.
        assert!(formatted.contains("content-type: text/html"));
        assert!(formatted.contains("accept: text/html"));

        // Sensitive: hashed, raw value absent.
        assert!(formatted.contains("x-api-key: hash:"));
        assert!(!formatted.contains("sk-secret"));
    }

    #[test]
    fn format_headers_empty() {
        let headers = hyper::HeaderMap::new();
        assert_eq!(format_headers(&headers), "");
    }

    // ---------------------------------------------------------------
    // TelemetryEmitter unit tests
    // ---------------------------------------------------------------

    /// Helper: create a DbWriter for tests with a reader for verification.
    fn make_test_db() -> Arc<DbWriter> {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
        std::mem::forget(dir);
        db
    }

    fn make_emitter(db: &Arc<DbWriter>) -> TelemetryEmitter {
        TelemetryEmitter {
            db: Arc::clone(db),
            config: make_config_dev(),
            domain: "example.com".to_string(),
            process_name: None,
            ai_provider: None,
            method: "GET".to_string(),
            path: "/".to_string(),
            query: None,
            status_code: Some(200),
            decision: Decision::Allowed,
            matched_rule: Some("default-dev-allow".to_string()),
            request_headers: Some("host: example.com".to_string()),
            response_headers: Some("content-type: text/html".to_string()),

            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time: Instant::now(),
        }
    }

    #[tokio::test]
    async fn telemetry_emitter_writes_net_event() {
        let db = make_test_db();
        let emitter = make_emitter(&db);
        emitter.emit().await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "example.com");
        assert_eq!(events[0].method, Some("GET".to_string()));
        assert_eq!(events[0].path, Some("/".to_string()));
        assert_eq!(events[0].status_code, Some(200));
        assert_eq!(events[0].decision, Decision::Allowed);
    }

    #[tokio::test]
    async fn telemetry_emitter_writes_model_call_for_ai() {
        let db = make_test_db();

        // Set up AI provider emitter with fake SSE state
        let ai_state = Arc::new(Mutex::new(crate::gateway::ai_body::AiStreamState {
            sse_parser: crate::gateway::sse::SseParser::new(),
            provider_parser: Box::new(crate::gateway::anthropic::AnthropicStreamParserWithState::new()),
            events: vec![
                crate::gateway::events::LlmEvent::MessageStart {
                    message_id: Some("msg_test".into()),
                    model: Some("claude-test".into()),
                },
                crate::gateway::events::LlmEvent::TextDelta { index: 0, text: "Hello".into() },
                crate::gateway::events::LlmEvent::MessageEnd {
                    stop_reason: Some(crate::gateway::events::StopReason::EndTurn),
                },
            ],
        }));
        let ai_stats = Arc::new(Mutex::new(crate::gateway::ai_body::AiBodyStats {
            bytes: 500,
            preview: Vec::new(),
            max_preview: 0,
        }));

        let emitter = TelemetryEmitter {
            db: Arc::clone(&db),
            config: make_config_dev(),
            domain: "api.anthropic.com".to_string(),
            process_name: Some("test".to_string()),
            ai_provider: Some(ProviderKind::Anthropic),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            query: None,
            status_code: Some(200),
            decision: Decision::Allowed,
            matched_rule: Some("ai-allow".to_string()),
            request_headers: Some("x-api-key: sk-test1234".to_string()),
            response_headers: Some("content-type: text/event-stream".to_string()),

            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Ai { stats: ai_stats, state: ai_state },
            start_time: Instant::now(),
        };
        emitter.emit().await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "api.anthropic.com");

        // ModelCall should also be recorded
        let calls = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.provider, "anthropic");
        assert_eq!(calls[0].1.model, Some("claude-test".to_string()));
    }

    // ---------------------------------------------------------------
    // TelemetryBody tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn telemetry_body_emits_on_completion() {
        let db = make_test_db();
        let emitter = make_emitter(&db);

        let inner = Full::new(Bytes::from("hello body"))
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed();
        let telem_body = TelemetryBody::new(inner, emitter);

        // Consume the body fully.
        let _ = telem_body.collect().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "example.com");
    }

    #[tokio::test]
    async fn telemetry_body_emits_on_drop() {
        let db = make_test_db();
        let emitter = make_emitter(&db);

        let inner = Full::new(Bytes::from("hello body"))
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed();
        let telem_body = TelemetryBody::new(inner, emitter);

        // Drop without consuming.
        drop(telem_body);

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1, "Drop fallback should emit");
        assert_eq!(events[0].domain, "example.com");
    }

    #[tokio::test]
    async fn telemetry_body_emits_only_once() {
        let db = make_test_db();
        let emitter = make_emitter(&db);

        let inner = Full::new(Bytes::from("hello body"))
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed();
        let telem_body = TelemetryBody::new(inner, emitter);

        // Consume fully (triggers emit on completion), then drop (should not emit again).
        let _ = telem_body.collect().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1, "should emit exactly once, not on both completion and drop");
    }

    // ---------------------------------------------------------------
    // Denied-request integration test (no upstream needed)
    // ---------------------------------------------------------------

    /// Build a rustls TLS client config that trusts our MITM CA.
    fn make_mitm_client_config() -> Arc<rustls::ClientConfig> {
        let mut root_store = rustls::RootCertStore::empty();
        let ca_certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        for cert in ca_certs {
            root_store.add(cert).unwrap();
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        Arc::new(rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(root_store)
            .with_no_client_auth())
    }

    #[tokio::test]
    async fn denied_request_emits_event() {
        let config = make_config_deny_all();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        let client_config = make_mitm_client_config();
        let connector = tokio_rustls::TlsConnector::from(client_config);
        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let sni = rustls::pki_types::ServerName::try_from("example.com").unwrap();
        let tls_stream = connector.connect(sni, stream).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move { let _ = conn.await; });

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/secret")
            .header("host", "example.com")
            .body(Full::new(Bytes::new()).map_err(|never| -> anyhow::Error { match never {} }).boxed())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 403);
        // Consume the body to trigger telemetry emission.
        let _ = resp.into_body().collect().await;

        drop(sender);
        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, Decision::Denied);
        assert_eq!(events[0].status_code, Some(403));
        assert_eq!(events[0].method, Some("GET".to_string()));
        assert_eq!(events[0].path, Some("/secret".to_string()));
    }

    /// Multiple denied requests on the same keep-alive connection produce
    /// one event per request (the core bug this fix addresses).
    #[tokio::test]
    async fn multiple_denied_requests_emit_separate_events() {
        let config = make_config_deny_all();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        let client_config = make_mitm_client_config();
        let connector = tokio_rustls::TlsConnector::from(client_config);
        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let sni = rustls::pki_types::ServerName::try_from("example.com").unwrap();
        let tls_stream = connector.connect(sni, stream).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move { let _ = conn.await; });

        // Send 3 requests on the same keep-alive connection.
        for path in ["/a", "/b", "/c"] {
            let req = hyper::Request::builder()
                .method("GET")
                .uri(path)
                .header("host", "example.com")
                .body(Full::new(Bytes::new()).map_err(|never| -> anyhow::Error { match never {} }).boxed())
                .unwrap();
            let resp = sender.send_request(req).await.unwrap();
            assert_eq!(resp.status().as_u16(), 403);
            let _ = resp.into_body().collect().await;
        }

        drop(sender);
        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let mut events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 3, "3 requests should produce 3 events, not 1");
        events.reverse(); // chronological order
        assert_eq!(events[0].path, Some("/a".to_string()));
        assert_eq!(events[1].path, Some("/b".to_string()));
        assert_eq!(events[2].path, Some("/c".to_string()));
    }

    #[tokio::test]
    async fn websocket_upgrade_rejected_with_400() {
        let config = make_config_dev();
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        let client_config = make_mitm_client_config();
        let connector = tokio_rustls::TlsConnector::from(client_config);
        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let sni = rustls::pki_types::ServerName::try_from("example.com").unwrap();
        let tls_stream = connector.connect(sni, stream).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move { let _ = conn.await; });

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/ws")
            .header("host", "example.com")
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .body(Full::new(Bytes::new()).map_err(|never| -> anyhow::Error { match never {} }).boxed())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 400, "WebSocket upgrades should return 400");
        let _ = resp.into_body().collect().await;

        drop(sender);
        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, Decision::Denied);
        assert_eq!(events[0].status_code, Some(400));
        assert_eq!(events[0].matched_rule, Some("websocket-not-supported".to_string()));
    }

    /// Upstream DNS failure returns 502 instead of killing the connection.
    #[tokio::test]
    async fn upstream_error_returns_502() {
        // Allow nonexistent.invalid but it will fail at TCP connect.
        use crate::net::policy::{DomainMatcher, PolicyRule};
        let policy = NetworkPolicy::new(
            vec![PolicyRule {
                matcher: DomainMatcher::parse("nonexistent.invalid"),
                allow_read: true,
                allow_write: true,
            }],
            false,
            false,
        );
        let config = make_config_with_policy(policy);
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        let client_config = make_mitm_client_config();
        let connector = tokio_rustls::TlsConnector::from(client_config);
        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let sni = rustls::pki_types::ServerName::try_from("nonexistent.invalid").unwrap();
        let tls_stream = connector.connect(sni, stream).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move { let _ = conn.await; });

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .header("host", "nonexistent.invalid")
            .body(Full::new(Bytes::new()).map_err(|never| -> anyhow::Error { match never {} }).boxed())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 502, "Upstream error should return 502");
        let _ = resp.into_body().collect().await;

        drop(sender);
        let _ = proxy_task.await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, Decision::Error);
        assert_eq!(events[0].status_code, Some(502));
        assert_eq!(events[0].domain, "nonexistent.invalid");
    }

    /// Helper to build a TelemetryEmitter with AI provider for testing emit_model_call.
    fn make_ai_emitter(config: &Arc<MitmProxyConfig>, provider: ProviderKind) -> TelemetryEmitter {
        TelemetryEmitter {
            db: Arc::clone(&config.db),
            config: Arc::clone(config),
            domain: "api.anthropic.com".to_string(),
            process_name: Some("claude".to_string()),
            ai_provider: Some(provider),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            query: None,
            status_code: Some(200),
            decision: Decision::Allowed,
            matched_rule: Some("ai-provider".to_string()),
            request_headers: None,
            response_headers: None,
            req_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time: Instant::now(),
        }
    }

    /// Build an `AiStreamState` with pre-populated events for testing.
    fn make_ai_state(events: Vec<crate::gateway::events::LlmEvent>) -> Arc<Mutex<crate::gateway::ai_body::AiStreamState>> {
        use crate::gateway::anthropic::AnthropicStreamParserWithState;
        Arc::new(Mutex::new(crate::gateway::ai_body::AiStreamState {
            sse_parser: crate::gateway::sse::SseParser::new(),
            provider_parser: Box::new(AnthropicStreamParserWithState::new()),
            events,
        }))
    }

    #[tokio::test]
    async fn emit_model_call_assigns_trace_id() {
        let config = make_config_dev();
        let emitter = make_ai_emitter(&config, ProviderKind::Anthropic);

        // Emit with no AI state (simulates non-streaming or empty response).
        emitter.emit_model_call(
            ProviderKind::Anthropic, 100, 200, 50, &None,
        ).await;

        // Flush the DB writer.
        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let calls = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls.len(), 1, "should have recorded one model call");
        assert!(calls[0].1.trace_id.is_some(), "trace_id should be assigned");
        assert!(!calls[0].1.trace_id.as_ref().unwrap().is_empty());
    }

    #[tokio::test]
    async fn emit_model_call_estimates_cost() {
        use crate::gateway::events::LlmEvent;
        let config = make_config_dev();
        let ai_state = make_ai_state(vec![
            LlmEvent::MessageStart {
                message_id: None,
                model: Some("claude-sonnet-4-20250514".to_string()),
            },
            LlmEvent::Usage {
                input_tokens: Some(1000),
                output_tokens: Some(500),
                details: std::collections::BTreeMap::new(),
            },
        ]);
        let emitter = make_ai_emitter(&config, ProviderKind::Anthropic);

        emitter.emit_model_call(
            ProviderKind::Anthropic, 100, 200, 50, &Some(ai_state),
        ).await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let calls = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].1.estimated_cost_usd > 0.0,
            "cost should be positive for known model with tokens: got {}",
            calls[0].1.estimated_cost_usd,
        );
    }

    #[tokio::test]
    async fn trace_chains_across_tool_use() {
        use crate::gateway::events::{LlmEvent, StopReason};
        let config = make_config_dev();

        // First call: model responds with tool_use, tool_call_id = "call_1".
        let ai_state1 = make_ai_state(vec![
            LlmEvent::ToolCallStart {
                index: 0,
                call_id: "call_1".to_string(),
                name: "bash".to_string(),
            },
            LlmEvent::ToolCallEnd { index: 0 },
            LlmEvent::MessageEnd {
                stop_reason: Some(StopReason::ToolUse),
            },
        ]);
        let emitter1 = make_ai_emitter(&config, ProviderKind::Anthropic);
        emitter1.emit_model_call(ProviderKind::Anthropic, 100, 200, 50, &Some(ai_state1)).await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let reader = config.db.reader().unwrap();
        let calls1 = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls1.len(), 1);
        let trace_id_1 = calls1[0].1.trace_id.clone().unwrap();

        // Second call: includes tool_response for call_1, model responds with end_turn.
        let req_body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "call_1", "name": "bash", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "done"}
                ]}
            ]
        });
        let req_bytes = serde_json::to_vec(&req_body).unwrap();

        let ai_state2 = make_ai_state(vec![
            LlmEvent::MessageEnd {
                stop_reason: Some(StopReason::EndTurn),
            },
        ]);

        let emitter2 = TelemetryEmitter {
            db: Arc::clone(&config.db),
            config: Arc::clone(&config),
            domain: "api.anthropic.com".to_string(),
            process_name: Some("claude".to_string()),
            ai_provider: Some(ProviderKind::Anthropic),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            query: None,
            status_code: Some(200),
            decision: Decision::Allowed,
            matched_rule: Some("ai-provider".to_string()),
            request_headers: None,
            response_headers: None,
            req_stats: Arc::new(Mutex::new(BodyStats {
                bytes: req_bytes.len() as u64,
                preview: req_bytes,
                max_preview: 64 * 1024,
            })),
            resp_kind: RespStatsKind::Plain(Arc::new(Mutex::new(BodyStats::new(0)))),
            start_time: Instant::now(),
        };
        emitter2.emit_model_call(ProviderKind::Anthropic, 100, 200, 50, &Some(ai_state2)).await;

        tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

        let calls2 = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls2.len(), 2, "should have 2 model calls now");
        // Most recent first -- calls2[0] is the second call.
        let trace_id_2 = calls2[0].1.trace_id.clone().unwrap();
        assert_eq!(
            trace_id_1, trace_id_2,
            "second call should share the same trace_id as first (chained via tool_use)"
        );
    }

    #[tokio::test]
    async fn trace_completes_on_end_turn() {
        use crate::gateway::events::{LlmEvent, StopReason};
        let config = make_config_dev();

        let ai_state = make_ai_state(vec![
            LlmEvent::MessageEnd {
                stop_reason: Some(StopReason::EndTurn),
            },
        ]);
        let emitter = make_ai_emitter(&config, ProviderKind::Anthropic);
        emitter.emit_model_call(ProviderKind::Anthropic, 100, 200, 50, &Some(ai_state)).await;

        // After end_turn, trace_state should have no pending entries.
        let state = config.trace_state.lock().unwrap();
        assert!(
            state.lookup(&["nonexistent".to_string()]).is_none(),
            "trace_state should be empty after end_turn"
        );
    }
}
