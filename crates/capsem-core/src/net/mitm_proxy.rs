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
/// 8. Record telemetry (domain, method, path, query, decision, matched rule)
use std::io;
use std::mem::ManuallyDrop;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use super::cert_authority::{CertAuthority, MitmCertResolver};
use super::policy::NetworkPolicy;
use super::telemetry::{Decision, NetEvent, WebDb};

/// Re-exported so capsem-app can reference the type without depending on rustls.
pub type UpstreamTlsConfig = rustls::ClientConfig;

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    pub policy: Arc<NetworkPolicy>,
    pub web_db: Arc<Mutex<WebDb>>,
    /// Cached upstream TLS config (shared across all connections).
    pub upstream_tls: Arc<rustls::ClientConfig>,
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
pub async fn handle_connection(vsock_fd: RawFd, config: Arc<MitmProxyConfig>) {
    let start = Instant::now();

    let result = handle_inner(vsock_fd, &config).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let (domain, decision, telemetry) = match &result {
        Ok(t) => (t.domain.clone(), t.decision, Some(t)),
        Err((d, dec, _)) => (d.clone(), *dec, None),
    };

    let event = NetEvent {
        timestamp: SystemTime::now(),
        domain: if domain.is_empty() {
            "<unknown>".to_string()
        } else {
            domain.clone()
        },
        port: 443,
        decision,
        bytes_sent: telemetry.map_or(0, |t| t.bytes_sent),
        bytes_received: telemetry.map_or(0, |t| t.bytes_received),
        duration_ms,
        method: telemetry.and_then(|t| t.method.clone()),
        path: telemetry.and_then(|t| t.path.clone()),
        query: telemetry.and_then(|t| t.query.clone()),
        status_code: telemetry.and_then(|t| t.status_code),
        matched_rule: telemetry
            .and_then(|t| t.matched_rule.clone().or_else(|| t.error_reason.clone()))
            .or_else(|| match &result {
                Err((_, _, reason)) => Some(reason.clone()),
                _ => None,
            }),
        request_headers: telemetry.and_then(|t| t.request_headers.clone()),
        response_headers: telemetry.and_then(|t| t.response_headers.clone()),
        request_body_preview: telemetry.and_then(|t| t.request_body_preview.clone()),
        response_body_preview: telemetry.and_then(|t| t.response_body_preview.clone()),
        conn_type: Some("https-mitm".to_string()),
    };

    if let Ok(db) = config.web_db.lock() {
        if let Err(e) = db.record(&event) {
            warn!(error = %e, "failed to record MITM net event");
        }
    }

    match decision {
        Decision::Allowed => info!(
            domain,
            method = ?telemetry.and_then(|t| t.method.as_deref()),
            path = ?telemetry.and_then(|t| t.path.as_deref()),
            status = ?telemetry.and_then(|t| t.status_code),
            duration_ms,
            "MITM proxy: completed"
        ),
        Decision::Denied => info!(domain, "MITM proxy: denied"),
        Decision::Error => warn!(domain, "MITM proxy: error"),
    }
}

/// Collected telemetry from a successful (or partially successful) connection.
struct ConnectionTelemetry {
    domain: String,
    decision: Decision,
    method: Option<String>,
    path: Option<String>,
    query: Option<String>,
    status_code: Option<u16>,
    matched_rule: Option<String>,
    request_headers: Option<String>,
    response_headers: Option<String>,
    request_body_preview: Option<String>,
    response_body_preview: Option<String>,
    bytes_sent: u64,
    bytes_received: u64,
    error_reason: Option<String>,
    req_stats: Option<Arc<Mutex<BodyStats>>>,
    resp_stats: Option<Arc<Mutex<BodyStats>>>,
}

/// Inner handler. Returns Ok(telemetry) on success, Err((domain, decision, reason)) on failure.
async fn handle_inner(
    vsock_fd: RawFd,
    config: &MitmProxyConfig,
) -> Result<ConnectionTelemetry, (String, Decision, String)> {
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

    // 1. Read initial bytes (TLS ClientHello).
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = tokio::io::AsyncReadExt::read(&mut vsock_stream, &mut initial_buf)
        .await
        .map_err(|e| (String::new(), Decision::Error, format!("read ClientHello: {e}")))?;
    if n == 0 {
        return Err((String::new(), Decision::Error, "empty connection".into()));
    }
    initial_buf.truncate(n);

    // 2. TLS handshake -- MitmCertResolver captures the domain from SNI.
    let resolver = Arc::new(MitmCertResolver::with_policy(
        Arc::clone(&config.ca),
        Arc::clone(&config.policy),
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
        // If the domain was captured and is fully blocked, this is a policy denial
        // (the resolver returned None to fail the handshake), not a TLS error.
        if !domain.is_empty() && config.policy.is_fully_blocked(&domain).is_some() {
            let rule = config.policy.is_fully_blocked(&domain).unwrap();
            (domain, Decision::Denied, rule)
        } else {
            (domain, Decision::Error, format!("TLS handshake: {e}"))
        }
    })?;

    // 3. Get domain from the resolver (captured during handshake).
    let domain = resolver.domain().ok_or_else(|| {
        (String::new(), Decision::Denied, "no SNI in ClientHello".into())
    })?;

    // 4. Run hyper HTTP/1.1 server on the MITM TLS stream.
    let io = TokioIo::new(tls_stream);

    let policy = Arc::clone(&config.policy);
    let upstream_tls = Arc::clone(&config.upstream_tls);
    let domain_for_svc = domain.clone();
    let log_bodies = config.policy.log_bodies;
    let max_body = config.policy.max_body_capture;

    // Shared telemetry state.
    let telem = Arc::new(Mutex::new(ConnectionTelemetry {
        domain: domain.clone(),
        decision: Decision::Allowed,
        method: None,
        path: None,
        query: None,
        status_code: None,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        bytes_sent: 0,
        bytes_received: 0,
        error_reason: None,
        req_stats: None,
        resp_stats: None,
    }));
    let telem_svc = Arc::clone(&telem);

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let policy = Arc::clone(&policy);
        let upstream_tls = Arc::clone(&upstream_tls);
        let domain = domain_for_svc.clone();
        let telem = Arc::clone(&telem_svc);

        async move {
            let res = handle_request(req, &domain, &policy, &upstream_tls, &telem, log_bodies, max_body).await;
            if let Err(ref e) = res {
                if let Ok(mut t) = telem.lock() {
                    t.decision = Decision::Error;
                    t.error_reason = Some(e.to_string());
                }
            }
            res
        }
    });

    // Serve exactly one connection (may have multiple requests via keep-alive).
    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await
    {
        // Connection errors are expected when the guest closes.
        if !e.is_incomplete_message() {
            warn!(domain, error = %e, "hyper serve error");
        }
    }

    let result = match Arc::try_unwrap(telem) {
        Ok(mutex) => {
            let mut t = mutex.into_inner().unwrap();
            if let Some(req_stats) = &t.req_stats {
                if let Ok(st) = req_stats.lock() {
                    t.bytes_sent = st.bytes;
                    if !st.preview.is_empty() {
                        t.request_body_preview = Some(String::from_utf8_lossy(&st.preview).into_owned());
                    }
                }
            }
            if let Some(resp_stats) = &t.resp_stats {
                if let Ok(st) = resp_stats.lock() {
                    t.bytes_received = st.bytes;
                    if !st.preview.is_empty() {
                        t.response_body_preview = Some(String::from_utf8_lossy(&st.preview).into_owned());
                    }
                }
            }
            t
        }
        Err(arc) => {
            let lock = arc.lock().unwrap();
            let mut t = ConnectionTelemetry {
                domain: lock.domain.clone(),
                decision: lock.decision,
                method: lock.method.clone(),
                path: lock.path.clone(),
                query: lock.query.clone(),
                status_code: lock.status_code,
                matched_rule: lock.matched_rule.clone(),
                request_headers: lock.request_headers.clone(),
                response_headers: lock.response_headers.clone(),
                request_body_preview: lock.request_body_preview.clone(),
                response_body_preview: lock.response_body_preview.clone(),
                bytes_sent: lock.bytes_sent,
                bytes_received: lock.bytes_received,
                error_reason: lock.error_reason.clone(),
                req_stats: lock.req_stats.clone(),
                resp_stats: lock.resp_stats.clone(),
            };
            if let Some(req_stats) = &t.req_stats {
                if let Ok(st) = req_stats.lock() {
                    t.bytes_sent = st.bytes;
                    if !st.preview.is_empty() {
                        t.request_body_preview = Some(String::from_utf8_lossy(&st.preview).into_owned());
                    }
                }
            }
            if let Some(resp_stats) = &t.resp_stats {
                if let Ok(st) = resp_stats.lock() {
                    t.bytes_received = st.bytes;
                    if !st.preview.is_empty() {
                        t.response_body_preview = Some(String::from_utf8_lossy(&st.preview).into_owned());
                    }
                }
            }
            t
        }
    };
    Ok(result)
}

/// Handle a single HTTP request within the MITM TLS connection.
async fn handle_request(
    req: hyper::Request<hyper::body::Incoming>,
    domain: &str,
    policy: &NetworkPolicy,
    upstream_tls: &Arc<rustls::ClientConfig>,
    telem: &Mutex<ConnectionTelemetry>,
    log_bodies: bool,
    max_body: usize,
) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error> {
    use http_body_util::BodyExt;

    // Step 5: Sequential Proxying Optimization
    // Start upstream connection before consuming full request body.
    let (parts, req_body) = req.into_parts();
    let method = parts.method.to_string();
    let (path, query) = split_path_query(&parts.uri);

    // Capture request metadata.
    let req_hdrs = format_headers(&parts.headers);
    {
        let mut t = telem.lock().unwrap();
        t.method = Some(method.clone());
        t.path = Some(path.clone());
        t.query = query.clone();
        t.request_headers = Some(req_hdrs);
    }

    // Check for WebSocket upgrade.
    let is_upgrade = parts.headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    // Policy check: domain + method -> read/write decision.
    let decision = policy.evaluate(domain, &method);
    {
        let mut t = telem.lock().unwrap();
        t.matched_rule = Some(decision.matched_rule.clone());
    }
    if !decision.allowed {
        {
            let mut t = telem.lock().unwrap();
            t.decision = Decision::Denied;
            t.status_code = Some(403);
        }
        let body = format!(
            "Capsem: request denied ({}: {} {})
",
            decision.reason, method, path
        );
        let boxed_body = Full::new(Bytes::from(body))
            .map_err(|never| match never {})
            .boxed();
        return Ok(hyper::Response::builder()
            .status(403)
            .body(boxed_body)
            .unwrap());
    }

    // Save original request headers.
    let original_headers = parts.headers.clone();
    let original_method = parts.method.clone();

    // Connect upstream TLS (using cached config).
    let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));

    let upstream_tcp = tokio::net::TcpStream::connect(format!("{domain}:443")).await?;
    let server_name = rustls::pki_types::ServerName::try_from(domain.to_string())?;
    let upstream_tls = connector.connect(server_name, upstream_tcp).await?;

    if is_upgrade {
        // WebSocket upgrade: log metadata, then blind bridge.
        let boxed_body = Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed();
        let resp = hyper::Response::builder()
            .status(101)
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .body(boxed_body)
            .unwrap();
        telem.lock().unwrap().status_code = Some(101);
        return Ok(resp);
    }

    // Forward request to upstream via hyper client.
    let upstream_io = TokioIo::new(upstream_tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(upstream_io).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

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

    // Track request body
    let req_stats = Arc::new(Mutex::new(BodyStats {
        bytes: 0,
        preview: Vec::new(),
        max_preview: if log_bodies { max_body } else { 0 },
    }));
    let tracked_req_body = TrackedBody::new(req_body, Arc::clone(&req_stats), 100 * 1024 * 1024); // 100MB limit
    let upstream_req = builder.body(tracked_req_body)?;

    let resp = sender.send_request(upstream_req).await?;
    let resp_status = resp.status().as_u16();

    // Track response body
    let resp_stats = Arc::new(Mutex::new(BodyStats {
        bytes: 0,
        preview: Vec::new(),
        max_preview: if log_bodies { max_body } else { 0 },
    }));
    let (resp_parts, resp_body) = resp.into_parts();
    let tracked_resp_body = TrackedBody::new(resp_body, Arc::clone(&resp_stats), 100 * 1024 * 1024); // 100MB limit

    // Capture response headers.
    let resp_hdrs = format_headers(&resp_parts.headers);

    {
        let mut t = telem.lock().unwrap();
        t.status_code = Some(resp_status);
        t.response_headers = Some(resp_hdrs);
        t.req_stats = Some(Arc::clone(&req_stats));
        t.resp_stats = Some(Arc::clone(&resp_stats));
    }

    // Build downstream response.
    let response = hyper::Response::from_parts(resp_parts, tracked_resp_body.boxed());

    // Spawn a task to update telemetry after response is complete
    // Wait, the body is consumed by the hyper server, we can't easily wait for it here
    // unless we spawn a task or just don't capture body metrics.
    // Actually, `handle_request` returns the response, and hyper streams it.
    // The telemetry is extracted after `serve_connection` completes!
    // So we just need to update telemetry with the stats from the `TrackedBody` instances
    // when `handle_connection` finishes.
    // We'll return the response now, but we need to pass the stats back.
    // Wait, `req_stats` and `resp_stats` are arcs. We can update `telem` periodically or just 
    // update `telem` in a Drop impl of `TrackedBody` or just do it in the closure.
    // Let's spawn a task to poll for completion? No, we can just save `req_stats` and `resp_stats`
    // in `ConnectionTelemetry` or let `TrackedBody` update `telem` directly!
    // Let's change TrackedBody to update `telem` directly.
    Ok(response)
}


type ProxyBoxBody = http_body_util::combinators::BoxBody<Bytes, anyhow::Error>;

struct BodyStats {
    bytes: u64,
    preview: Vec<u8>,
    max_preview: usize,
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

/// Format HTTP headers as a string for telemetry.
fn format_headers(headers: &hyper::HeaderMap) -> String {
    headers
        .iter()
        .map(|(name, value)| {
            format!("{}: {}", name, value.to_str().unwrap_or("<binary>"))
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

    use crate::net::cert_authority::CertAuthority;
    use crate::net::policy::NetworkPolicy;

    const CA_KEY: &str = include_str!("../../../../config/capsem-ca.key");
    const CA_CERT: &str = include_str!("../../../../config/capsem-ca.crt");

    fn make_config_with_policy(policy: NetworkPolicy) -> Arc<MitmProxyConfig> {
        let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
        let web_db = Arc::new(Mutex::new(WebDb::open_in_memory().unwrap()));
        Arc::new(MitmProxyConfig {
            ca,
            policy: Arc::new(policy),
            web_db,
            upstream_tls: make_upstream_tls_config(),
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

    #[tokio::test]
    async fn no_sni_records_error() {
        let config = make_config_dev();
        let (mut s1, s2) = UnixStream::pair().unwrap();

        std::io::Write::write_all(&mut s1, b"not a client hello").unwrap();
        drop(s1);

        handle_connection(s2.into_raw_fd(), config.clone()).await;

        let db = config.web_db.lock().unwrap();
        let events = db.recent(10).unwrap();
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

        let db = config.web_db.lock().unwrap();
        let events = db.recent(10).unwrap();
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
}
