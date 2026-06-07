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
pub mod policy_hook;
pub mod policy_v2_http_hook;
pub mod policy_v2_model;
pub mod protocol;
pub mod spans;
pub mod sse_parser_hook;
pub mod telemetry_hook;
mod util;

use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use capsem_logger::{DbWriter, Decision, NetEvent, WriteOp};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn, Instrument};

trait TokioReadWrite: AsyncRead + AsyncWrite {}

impl<T> TokioReadWrite for T where T: AsyncRead + AsyncWrite {}

use super::cert_authority::{CertAuthority, MitmCertResolver};
use super::policy::NetworkPolicy;
use crate::net::ai_traffic::provider::ProviderKind;
use body::{BodyStats, ProxyBoxBody, TrackedBody};
use fd_stream::{set_nonblocking, AsyncFdStream, ReplayReader};
use protocol::Protocol;
use telemetry_hook::TelemetryRequestContext;
use util::{
    format_headers, format_headers_for_domain, is_llm_api_path, parse_http_host_target,
    split_path_query,
};

pub use mcp_endpoint::{McpEndpointState, McpTimeouts};

/// Re-exported so capsem-app can reference the type without depending on rustls.
pub type UpstreamTlsConfig = rustls::ClientConfig;

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;

static FIRST_NETWORK_READY_EMITTED: AtomicBool = AtomicBool::new(false);

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    /// Live policy, swappable via RwLock so settings changes take effect
    /// without restarting the VM. Each HTTP request snapshots the Arc so
    /// that disabling a provider blocks the next request even on an
    /// existing keep-alive connection.
    pub policy: Arc<std::sync::RwLock<Arc<NetworkPolicy>>>,
    /// Live Policy V2 config shared with HTTP, DNS, MCP, model, and
    /// hook enforcement. Held here for model request rules, which need
    /// the request body before upstream dispatch.
    pub policy_v2: Arc<tokio::sync::RwLock<Arc<crate::net::policy_config::PolicyConfig>>>,
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
    /// Hook pipeline. `make_production_pipeline` registers PolicyHook
    /// plus the sync ChunkHook chain (decompression → SSE parse →
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

/// Build the production hook pipeline. Registers PolicyHook (async,
/// for `RawRequestHead`) plus the full sync ChunkHook chain
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
    let policy_v2 = Arc::new(tokio::sync::RwLock::new(Arc::new(
        crate::net::policy_config::PolicyConfig::with_builtin_security_rules(),
    )));
    make_production_pipeline_with_policy_v2(policy, policy_v2, telemetry)
}

pub fn make_production_pipeline_with_policy_v2(
    policy: Arc<std::sync::RwLock<Arc<NetworkPolicy>>>,
    policy_v2: Arc<tokio::sync::RwLock<Arc<crate::net::policy_config::PolicyConfig>>>,
    telemetry: Arc<telemetry_hook::TelemetryDeps>,
) -> Arc<pipeline::Pipeline> {
    let p = pipeline::Pipeline::builder()
        .register(Arc::new(policy_hook::PolicyHook::new(policy)))
        .register(Arc::new(policy_v2_http_hook::PolicyV2HttpHook::new(
            policy_v2,
        )))
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
        .protocol_for_host(domain)
}

fn ai_provider_for_target(
    config: &MitmProxyConfig,
    domain: &str,
    upstream_port: u16,
) -> Option<ProviderKind> {
    config
        .model_endpoints
        .read()
        .unwrap()
        .protocol_for_target(domain, upstream_port)
}

fn provider_label(provider: Option<ProviderKind>) -> &'static str {
    provider.map(|provider| provider.as_str()).unwrap_or("none")
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
            let ai_provider = ai_provider_for_target(&config_arc, &request_domain, upstream_port);
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
    let (mut parts, req_body) = req.into_parts();
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

    // Hook-driven policy. The pipeline runs PolicyHook (and any
    // other RawRequestHead-registered hooks). PolicyHook stashes its
    // PolicyDecision in HookCtx::state so we can read matched_rule +
    // reason back here. On deny it returns Stop(Reject(403)); the
    // 403 body is wrapped in ChunkDispatchBody seeded with a
    // TelemetryRequestContext so TelemetryHook still emits a
    // NetEvent for the deny path.
    let dispatch_outcome;
    let policy_decision;
    let policy_v2_decision;
    {
        let conn = hooks::ConnMeta {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            port: upstream_port,
            protocol,
            ai_provider,
        };
        let mut state = hooks::HookState::default();
        let trace_id = crate::telemetry::ambient_capsem_trace_id();
        let policy_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_POLICY_REQUEST,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
            decision = tracing::field::Empty,
            rule_count = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        dispatch_outcome = config
            .pipeline
            .dispatch(
                events::Event::RawRequestHead(&mut parts),
                &mut state,
                trace_id,
                &conn,
            )
            .instrument(policy_span.clone())
            .await;
        let decision = match &dispatch_outcome {
            pipeline::DispatchOutcome::Completed => "allow",
            pipeline::DispatchOutcome::Stopped(_) => "block",
        };
        policy_span.record("decision", decision);
        policy_span.record("status", "ok");
        // Lift the policy decision out of the per-dispatch state so we
        // can use it for the telemetry emitter. Cloned because state
        // drops at the end of this scope.
        policy_decision = state
            .peek::<policy_hook::LastPolicyDecision>()
            .cloned()
            .unwrap_or_default();
        policy_v2_decision = state
            .peek::<policy_v2_http_hook::LastHttpPolicyV2Decision>()
            .cloned()
            .unwrap_or_default();
    }

    let method = parts.method.to_string();
    let (path, query) = split_path_query(&parts.uri);
    let formatted_req_headers = format_headers_for_domain(domain, &parts.headers);
    let req_hdrs = formatted_req_headers.formatted;
    let credential_observations = formatted_req_headers.observations;
    let credential_ref = formatted_req_headers.credential_ref;
    let response_policy_context =
        policy_v2_http_hook::HttpResponsePolicyContext::from_request_parts(
            protocol, domain, &parts,
        );
    let matched_rule = policy_v2_decision
        .policy_rule
        .clone()
        .unwrap_or_else(|| policy_decision.matched_rule.clone());

    // T1 slice 4: per-request counter, partitioned by decision.
    // upstream_error increments are handled at the dial site below.
    let req_decision_label = match &dispatch_outcome {
        pipeline::DispatchOutcome::Completed => "allow",
        pipeline::DispatchOutcome::Stopped(_) => "deny",
    };
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

    if let pipeline::DispatchOutcome::Stopped(stop_action) = dispatch_outcome {
        // Today only the Reject variant ships; Drop / DnsReject land
        // in T2 / T3. Future Stop variants get matched here.
        let hook_resp = match stop_action {
            hooks::StopAction::Reject(r) => r,
            other => {
                // Drop / DnsReject: synthesize a 502 fallback so we
                // emit telemetry consistently. Real handling lands in
                // T2 (plain HTTP) and T3 (DNS).
                let _ = other;
                let body = Full::new(Bytes::from_static(b"capsem: request stopped"))
                    .map_err(|never| match never {})
                    .boxed();
                http::Response::builder()
                    .status(http::StatusCode::BAD_GATEWAY)
                    .body(body)
                    .expect("static response build")
            }
        };

        let (resp_parts, resp_body) = hook_resp.into_parts();

        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(resp_parts.status.as_u16()),
            decision: Decision::Denied,
            matched_rule: Some(matched_rule.clone()),
            request_headers: Some(req_hdrs),
            response_headers: None,
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            policy_mode: policy_v2_decision.policy_mode.clone(),
            policy_action: policy_v2_decision.policy_action.clone(),
            policy_rule: policy_v2_decision.policy_rule.clone(),
            policy_reason: policy_v2_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
        };

        return Ok(hyper::Response::from_parts(
            resp_parts,
            seal_with_telemetry(resp_body, req_ctx),
        ));
    }

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
                policy_mode: policy_v2_decision.policy_mode.clone(),
                policy_action: policy_v2_decision.policy_action.clone(),
                policy_rule: policy_v2_decision.policy_rule.clone(),
                policy_reason: policy_v2_decision.policy_reason.clone(),
                credential_ref: credential_ref.clone(),
                credential_observations: credential_observations.clone(),
            };
            let body = Full::new(Bytes::from(body_text))
                .map_err(|never| match never {})
                .boxed();
            hyper::Response::builder()
                .status(http::StatusCode::BAD_GATEWAY)
                .body(seal_with_telemetry(body, req_ctx))
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
            policy_mode: policy_v2_decision.policy_mode.clone(),
            policy_action: policy_v2_decision.policy_action.clone(),
            policy_rule: policy_v2_decision.policy_rule.clone(),
            policy_reason: policy_v2_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
        };

        let empty_body = Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed();

        return Ok(hyper::Response::from_parts(
            resp_parts,
            seal_with_telemetry(empty_body, req_ctx),
        ));
    }

    // Save original request headers.
    let mut original_headers = parts.headers.clone();
    let original_method = parts.method.clone();
    let mut request_policy_v2_decision = policy_v2_decision.clone();

    // Helper: build a 502 Bad Gateway response with telemetry so upstream
    // errors don't kill keep-alive connections (returns Ok, not Err).
    let make_502 = |error: &dyn std::fmt::Display,
                    method: &str,
                    path: &str,
                    query: &Option<String>,
                    req_hdrs: &str,
                    start: Instant,
                    policy_v2: &policy_v2_http_hook::LastHttpPolicyV2Decision|
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
            policy_mode: policy_v2.policy_mode.clone(),
            policy_action: policy_v2.policy_action.clone(),
            policy_rule: policy_v2.policy_rule.clone(),
            policy_reason: policy_v2.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::builder()
            .status(502)
            .body(seal_with_telemetry(deny_body, req_ctx))
            .unwrap()
    };

    let http_security_event = crate::security_engine::SecurityEvent::new(
        crate::net::policy_config::PolicyCallback::HttpRequest,
    )
    .with_http_request(crate::security_engine::HttpRequestSecurityEvent::new(
        domain,
        ai_provider,
        original_headers.clone(),
        query.clone(),
    ));
    let security_emitter = Arc::new(crate::security_engine::TracingSecurityEventEmitter);
    let security_engine =
        crate::security_engine::SecurityEventEngine::with_builtin_actions(security_emitter);
    let action_rules = request_policy_v2_decision
        .matched_action_rules
        .iter()
        .chain(request_policy_v2_decision.matched_rule.iter())
        .cloned()
        .collect::<Vec<_>>();
    let actions_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_SECURITY_ACTIONS,
        protocol = protocol.label(),
        provider = provider_label(ai_provider),
        action_count = action_rules.len() as u64,
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let http_security_event = match actions_span
        .in_scope(|| security_engine.apply_rules_and_emit(&action_rules, http_security_event))
    {
        Ok(event) => event,
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
                &request_policy_v2_decision,
            ));
        }
    };
    actions_span.record("decision", "allow");
    actions_span.record("status", "ok");
    let upstream_materialized = match actions_span.in_scope(|| {
        crate::security_engine::materialize_http_request_for_upstream(&http_security_event)
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
                &request_policy_v2_decision,
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
    // policy-checked. Default allowlist is `[80]`; tests / dev
    // configs extend it (e.g. 11434 for Ollama in T2.3). The TLS
    // path always uses 443, which is implicit and not gated here.
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
            policy_mode: policy_v2_decision.policy_mode.clone(),
            policy_action: policy_v2_decision.policy_action.clone(),
            policy_rule: policy_v2_decision.policy_rule.clone(),
            policy_reason: policy_v2_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
        };
        let deny_body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        return Ok(hyper::Response::builder()
            .status(403)
            .body(seal_with_telemetry(deny_body, req_ctx))
            .unwrap());
    }

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

    let policy_v2_snapshot = config.policy_v2.read().await.clone();
    let should_evaluate_model_request = ai_provider.is_some_and(|provider| {
        is_llm_api_path(provider, &path)
            && policy_v2_model::has_model_request_rules(&policy_v2_snapshot)
    });
    let upstream_req_body: ProxyBoxBody = if should_evaluate_model_request {
        let model_request_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_MODEL_REQUEST_POLICY,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
            decision = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let collected = match http_body_util::Limited::new(req_body, 100 * 1024 * 1024)
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
                    &request_policy_v2_decision,
                ));
            }
        };
        let body_bytes = collected.to_bytes();
        let mut body_for_upstream = body_bytes.clone();
        {
            let mut st = req_stats.lock().expect("req body stats lock");
            st.bytes = body_bytes.len() as u64;
            let to_copy = st.max_preview.min(body_bytes.len());
            st.preview.extend_from_slice(&body_bytes[..to_copy]);
        }

        if let Some(provider) = ai_provider {
            if let Some(outcome) = policy_v2_model::evaluate_model_request_policy(
                &policy_v2_snapshot,
                provider,
                &original_headers,
                &body_bytes,
            ) {
                match outcome {
                    policy_v2_model::ModelRequestPolicyOutcome::Continue(decision) => {
                        model_request_span.record("decision", "allow");
                        model_request_span.record("status", "ok");
                        request_policy_v2_decision.policy_mode = decision.policy_mode;
                        request_policy_v2_decision.policy_action = decision.policy_action;
                        request_policy_v2_decision.policy_rule = decision.policy_rule;
                        request_policy_v2_decision.policy_reason = decision.policy_reason;
                    }
                    policy_v2_model::ModelRequestPolicyOutcome::Deny(decision) => {
                        model_request_span.record("decision", "block");
                        model_request_span.record("status", "ok");
                        let body_text = format!(
                            "capsem: model request blocked by policy: {}\n",
                            decision
                                .policy_rule
                                .as_deref()
                                .unwrap_or("policy.model.unknown")
                        );
                        let mut scrubbed_stats = BodyStats::new(0);
                        scrubbed_stats.bytes = body_bytes.len() as u64;
                        let req_ctx = TelemetryRequestContext {
                            domain: domain.to_string(),
                            process_name: process_name.clone(),
                            ai_provider,
                            method: method.clone(),
                            path: path.clone(),
                            query: query.clone(),
                            status_code: Some(403),
                            decision: Decision::Denied,
                            matched_rule: decision.policy_rule.clone(),
                            request_headers: Some(req_hdrs.clone()),
                            response_headers: None,
                            start_time,
                            request_body_stats: Arc::new(Mutex::new(scrubbed_stats)),
                            max_response_preview: 0,
                            port: upstream_port,
                            conn_type,
                            policy_mode: decision.policy_mode,
                            policy_action: decision.policy_action,
                            policy_rule: decision.policy_rule,
                            policy_reason: decision.policy_reason,
                            credential_ref: credential_ref.clone(),
                            credential_observations: credential_observations.clone(),
                        };
                        let deny_body = Full::new(Bytes::from(body_text))
                            .map_err(|never| match never {})
                            .boxed();
                        return Ok(hyper::Response::builder()
                            .status(403)
                            .body(seal_with_telemetry(deny_body, req_ctx))
                            .unwrap());
                    }
                    policy_v2_model::ModelRequestPolicyOutcome::RewriteBody { decision, body } => {
                        model_request_span.record("decision", "preprocess");
                        model_request_span.record("status", "ok");
                        request_policy_v2_decision.policy_mode = decision.policy_mode;
                        request_policy_v2_decision.policy_action = decision.policy_action;
                        request_policy_v2_decision.policy_rule = decision.policy_rule;
                        request_policy_v2_decision.policy_reason = decision.policy_reason;

                        {
                            let mut st = req_stats.lock().expect("req body stats lock");
                            st.bytes = body.len() as u64;
                            st.preview.clear();
                            let to_copy = st.max_preview.min(body.len());
                            st.preview.extend_from_slice(&body[..to_copy]);
                        }
                        original_headers.remove(http::header::CONTENT_LENGTH);
                        if let Ok(value) = http::HeaderValue::from_str(&body.len().to_string()) {
                            original_headers.insert(http::header::CONTENT_LENGTH, value);
                        }
                        body_for_upstream = Bytes::from(body);
                    }
                }
            } else {
                model_request_span.record("decision", "allow");
                model_request_span.record("status", "ok");
            }
        }

        Full::new(body_for_upstream)
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed()
    } else {
        TrackedBody::new(req_body, Arc::clone(&req_stats), 100 * 1024 * 1024).boxed()
    };

    // Try to reuse a cached upstream sender, or create a new
    // connection. Each MITM connection serves one upstream via
    // keep-alive, so per-connection caching avoids re-establishing
    // TCP[+TLS] for every request.
    let upstream_prepare_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_UPSTREAM_PREPARE,
        protocol = protocol.label(),
        provider = provider_label(ai_provider),
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

    // Create a fresh upstream connection if needed. TLS path goes
    // TCP -> TLS handshake -> HTTP/1.1 handshake; HTTP path skips
    // the TLS step.
    let mut sender = if let Some(s) = reusable {
        s
    } else {
        let dial_start = Instant::now();
        let tcp_start = Instant::now();
        let upstream_tcp = match tokio::net::TcpStream::connect(format!("{domain}:{upstream_port}"))
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
                    &request_policy_v2_decision,
                ));
            }
        };
        tcp_us = tcp_start.elapsed().as_micros() as u64;

        // TLS path: wrap TCP in a TLS stream, time the handshake.
        // HTTP path: skip TLS, hand the bare TCP stream to hyper.
        let (sender, hs_us) = match protocol {
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
                            &request_policy_v2_decision,
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
                            &request_policy_v2_decision,
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
                            &request_policy_v2_decision,
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
                            &request_policy_v2_decision,
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
        provider = provider_label(ai_provider),
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
                &request_policy_v2_decision,
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

    // Dispatch RawResponseHead before any telemetry capture or guest
    // delivery. Policy V2 response rules can strip/rewrite the head in
    // place or fail closed with a synthetic response.
    let response_dispatch_outcome;
    let response_policy_v2_decision;
    {
        let conn = hooks::ConnMeta {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            port: upstream_port,
            protocol,
            ai_provider,
        };
        let mut state = hooks::HookState::default();
        state.set(response_policy_context);
        let trace_id = crate::telemetry::ambient_capsem_trace_id();
        let response_policy_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_POLICY_RESPONSE,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
            decision = tracing::field::Empty,
            rule_count = tracing::field::Empty,
            status = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        response_dispatch_outcome = config
            .pipeline
            .dispatch(
                events::Event::RawResponseHead(&mut resp_parts),
                &mut state,
                trace_id,
                &conn,
            )
            .instrument(response_policy_span.clone())
            .await;
        let decision = match &response_dispatch_outcome {
            pipeline::DispatchOutcome::Completed => "allow",
            pipeline::DispatchOutcome::Stopped(_) => "block",
        };
        response_policy_span.record("decision", decision);
        response_policy_span.record("status", "ok");
        response_policy_v2_decision = state
            .peek::<policy_v2_http_hook::LastHttpPolicyV2Decision>()
            .cloned()
            .unwrap_or_default();
    }

    let mut effective_policy_v2_decision = if response_policy_v2_decision.policy_action.is_some() {
        response_policy_v2_decision
    } else {
        request_policy_v2_decision.clone()
    };
    let effective_matched_rule = effective_policy_v2_decision
        .policy_rule
        .clone()
        .unwrap_or_else(|| matched_rule.clone());

    if let pipeline::DispatchOutcome::Stopped(stop_action) = response_dispatch_outcome {
        let hook_resp = match stop_action {
            hooks::StopAction::Reject(r) => r,
            other => {
                let _ = other;
                let body = Full::new(Bytes::from_static(b"capsem: response stopped"))
                    .map_err(|never| match never {})
                    .boxed();
                http::Response::builder()
                    .status(http::StatusCode::BAD_GATEWAY)
                    .body(body)
                    .expect("static response build")
            }
        };
        let (deny_parts, deny_body) = hook_resp.into_parts();
        let deny_status = deny_parts.status.as_u16();
        tracing::Span::current().record("status", deny_status);
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            method,
            path,
            query,
            status_code: Some(deny_status),
            decision: Decision::Denied,
            matched_rule: Some(effective_matched_rule),
            request_headers: Some(req_hdrs),
            response_headers: None,
            start_time,
            request_body_stats: Arc::clone(&req_stats),
            max_response_preview: 0,
            port: upstream_port,
            conn_type,
            policy_mode: effective_policy_v2_decision.policy_mode.clone(),
            policy_action: effective_policy_v2_decision.policy_action.clone(),
            policy_rule: effective_policy_v2_decision.policy_rule.clone(),
            policy_reason: effective_policy_v2_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
        };

        return Ok(hyper::Response::from_parts(
            deny_parts,
            seal_with_telemetry(deny_body, req_ctx),
        ));
    }

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

    let should_evaluate_model_response = ai_provider.is_some_and(|provider| {
        is_llm_api_path(provider, &path)
            && policy_v2_model::has_model_response_rules(&policy_v2_snapshot)
    });

    let resp_body: ProxyBoxBody = if should_evaluate_model_response {
        let model_response_span = tracing::debug_span!(
            target: "capsem.mitm",
            spans::MITM_MODEL_RESPONSE_POLICY,
            protocol = protocol.label(),
            provider = provider_label(ai_provider),
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
                    &effective_policy_v2_decision,
                ));
            }
        };
        let mut response_body = collected.to_bytes();

        if let Some(provider) = ai_provider {
            let request_preview = {
                let st = req_stats.lock().expect("req body stats lock");
                st.preview.clone()
            };
            let request_meta =
                crate::net::ai_traffic::request_parser::parse_request(provider, &request_preview);
            if let Some(outcome) = policy_v2_model::evaluate_model_response_policy(
                &policy_v2_snapshot,
                provider,
                &request_meta,
                &response_body,
            ) {
                match outcome {
                    policy_v2_model::ModelResponsePolicyOutcome::Continue(decision) => {
                        model_response_span.record("decision", "allow");
                        model_response_span.record("status", "ok");
                        effective_policy_v2_decision.policy_mode = decision.policy_mode;
                        effective_policy_v2_decision.policy_action = decision.policy_action;
                        effective_policy_v2_decision.policy_rule = decision.policy_rule;
                        effective_policy_v2_decision.policy_reason = decision.policy_reason;
                    }
                    policy_v2_model::ModelResponsePolicyOutcome::Deny(decision) => {
                        model_response_span.record("decision", "block");
                        model_response_span.record("status", "ok");
                        let body_text = format!(
                            "capsem: model response blocked by policy: {}\n",
                            decision
                                .policy_rule
                                .as_deref()
                                .unwrap_or("policy.model.unknown")
                        );
                        let req_ctx = TelemetryRequestContext {
                            domain: domain.to_string(),
                            process_name: process_name.clone(),
                            ai_provider,
                            method,
                            path,
                            query,
                            status_code: Some(403),
                            decision: Decision::Denied,
                            matched_rule: decision.policy_rule.clone(),
                            request_headers: Some(req_hdrs),
                            response_headers: None,
                            start_time,
                            request_body_stats: Arc::clone(&req_stats),
                            max_response_preview: 0,
                            port: upstream_port,
                            conn_type,
                            policy_mode: decision.policy_mode,
                            policy_action: decision.policy_action,
                            policy_rule: decision.policy_rule,
                            policy_reason: decision.policy_reason,
                            credential_ref: credential_ref.clone(),
                            credential_observations: credential_observations.clone(),
                        };
                        let deny_body = Full::new(Bytes::from(body_text))
                            .map_err(|never| match never {})
                            .boxed();
                        return Ok(hyper::Response::builder()
                            .status(403)
                            .body(seal_with_telemetry(deny_body, req_ctx))
                            .unwrap());
                    }
                    policy_v2_model::ModelResponsePolicyOutcome::RewriteBody { decision, body } => {
                        model_response_span.record("decision", "postprocess");
                        model_response_span.record("status", "ok");
                        effective_policy_v2_decision.policy_mode = decision.policy_mode;
                        effective_policy_v2_decision.policy_action = decision.policy_action;
                        effective_policy_v2_decision.policy_rule = decision.policy_rule;
                        effective_policy_v2_decision.policy_reason = decision.policy_reason;
                        resp_parts.headers.remove(http::header::CONTENT_LENGTH);
                        if let Ok(value) = http::HeaderValue::from_str(&body.len().to_string()) {
                            resp_parts
                                .headers
                                .insert(http::header::CONTENT_LENGTH, value);
                        }
                        response_body = Bytes::from(body);
                    }
                }
            } else {
                model_response_span.record("decision", "allow");
                model_response_span.record("status", "ok");
            }
        }

        Full::new(response_body)
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed()
    } else {
        resp_body.map_err(|e| -> anyhow::Error { e.into() }).boxed()
    };

    let req_ctx = TelemetryRequestContext {
        domain: domain.to_string(),
        process_name: process_name.clone(),
        ai_provider,
        method,
        path,
        query,
        status_code: Some(resp_status),
        decision: Decision::Allowed,
        matched_rule: Some(
            effective_policy_v2_decision
                .policy_rule
                .clone()
                .unwrap_or(effective_matched_rule),
        ),
        request_headers: Some(req_hdrs),
        response_headers: Some(resp_hdrs),
        start_time,
        request_body_stats: Arc::clone(&req_stats),
        max_response_preview: resp_max_preview,
        port: upstream_port,
        conn_type,
        policy_mode: effective_policy_v2_decision.policy_mode.clone(),
        policy_action: effective_policy_v2_decision.policy_action.clone(),
        policy_rule: effective_policy_v2_decision.policy_rule.clone(),
        policy_reason: effective_policy_v2_decision.policy_reason.clone(),
        credential_ref: credential_ref.clone(),
        credential_observations: credential_observations.clone(),
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
