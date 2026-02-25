/// MITM transparent proxy: terminates TLS from the guest, inspects HTTP traffic,
/// applies method+path policy, and bridges to the real upstream server.
///
/// Connection flow:
/// 1. Read initial bytes from vsock fd (TLS ClientHello)
/// 2. Extract SNI hostname
/// 3. Domain-level policy check (early reject before TLS handshake)
/// 4. Downstream TLS handshake (rustls server + MitmCertResolver)
/// 5. Read HTTP request via hyper
/// 6. HTTP-level policy check (method + path)
/// 7. If denied: return 403
/// 8. Upstream TLS to real server
/// 9. Forward request, stream response back
/// 10. Record telemetry
use std::io;
use std::mem::ManuallyDrop;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use super::cert_authority::{CertAuthority, MitmCertResolver};
use super::domain_policy::Action;
use super::http_policy::HttpPolicy;
use super::sni_parser;
use super::telemetry::{Decision, NetEvent, WebDb};

/// Maximum bytes to buffer when peeking at the TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    pub policy: Arc<HttpPolicy>,
    pub web_db: Arc<Mutex<WebDb>>,
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
        reason: match &result {
            Ok(_) => None,
            Err((_, _, e)) => Some(e.clone()),
        },
        method: telemetry.and_then(|t| t.method.clone()),
        path: telemetry.and_then(|t| t.path.clone()),
        status_code: telemetry.and_then(|t| t.status_code),
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
    status_code: Option<u16>,
    request_headers: Option<String>,
    response_headers: Option<String>,
    request_body_preview: Option<String>,
    response_body_preview: Option<String>,
    bytes_sent: u64,
    bytes_received: u64,
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

    // 2. Extract SNI.
    let domain = sni_parser::extract_sni(&initial_buf).ok_or_else(|| {
        (String::new(), Decision::Denied, "no SNI in ClientHello".into())
    })?;

    // 3. Domain-level policy check.
    let domain_check = config.policy.evaluate_domain(&domain);
    if domain_check.action == Action::Deny {
        return Err((
            domain,
            Decision::Denied,
            format!("domain denied: {}", domain_check.reason),
        ));
    }

    // 4. TLS handshake with MITM cert.
    let resolver = MitmCertResolver {
        ca: Arc::clone(&config.ca),
    };
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls_config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| (domain.clone(), Decision::Error, format!("TLS config: {e}")))?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(resolver));
    tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Chain buffered ClientHello bytes with the remaining vsock stream.
    let replay = ReplayReader::new(initial_buf, vsock_stream);
    let tls_stream = acceptor.accept(replay).await.map_err(|e| {
        (domain.clone(), Decision::Error, format!("TLS handshake: {e}"))
    })?;

    // 5. Run hyper HTTP/1.1 server on the MITM TLS stream.
    let io = TokioIo::new(tls_stream);

    let policy = Arc::clone(&config.policy);
    let domain_for_svc = domain.clone();
    let log_bodies = config.policy.log_bodies;
    let max_body = config.policy.max_body_capture;

    // Shared telemetry state.
    let telem = Arc::new(Mutex::new(ConnectionTelemetry {
        domain: domain.clone(),
        decision: Decision::Allowed,
        method: None,
        path: None,
        status_code: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        bytes_sent: 0,
        bytes_received: 0,
    }));
    let telem_svc = Arc::clone(&telem);

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let policy = Arc::clone(&policy);
        let domain = domain_for_svc.clone();
        let telem = Arc::clone(&telem_svc);

        async move {
            let res = handle_request(req, &domain, &policy, &telem, log_bodies, max_body).await;
            if res.is_err() {
                if let Ok(mut t) = telem.lock() {
                    t.decision = Decision::Error;
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
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => {
            let lock = arc.lock().unwrap();
            ConnectionTelemetry {
                domain: lock.domain.clone(),
                decision: lock.decision,
                method: lock.method.clone(),
                path: lock.path.clone(),
                status_code: lock.status_code,
                request_headers: lock.request_headers.clone(),
                response_headers: lock.response_headers.clone(),
                request_body_preview: lock.request_body_preview.clone(),
                response_body_preview: lock.response_body_preview.clone(),
                bytes_sent: lock.bytes_sent,
                bytes_received: lock.bytes_received,
            }
        }
    };
    Ok(result)
}

/// Handle a single HTTP request within the MITM TLS connection.
async fn handle_request(
    req: hyper::Request<hyper::body::Incoming>,
    domain: &str,
    policy: &HttpPolicy,
    telem: &Mutex<ConnectionTelemetry>,
    log_bodies: bool,
    max_body: usize,
) -> Result<hyper::Response<Full<Bytes>>, anyhow::Error> {
    let method = req.method().to_string();
    let path = req.uri().path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    // Capture request metadata.
    let req_hdrs = format_headers(req.headers());
    {
        let mut t = telem.lock().unwrap();
        t.method = Some(method.clone());
        t.path = Some(path.clone());
        t.request_headers = Some(req_hdrs);
    }

    // Check for WebSocket upgrade.
    let is_upgrade = req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    // HTTP-level policy check.
    let decision = policy.evaluate_request(domain, &method, &path);
    if decision.action == Action::Deny {
        telem.lock().unwrap().decision = Decision::Denied;
        let body = format!(
            "Capsem: request denied ({}: {} {})\n",
            decision.reason, method, path
        );
        return Ok(hyper::Response::builder()
            .status(403)
            .body(Full::new(Bytes::from(body)))
            .unwrap());
    }

    // Save original request headers before consuming the body.
    let original_headers = req.headers().clone();
    let original_method = req.method().clone();

    // Collect request body.
    let body_bytes = req.collect().await?.to_bytes();
    {
        let mut t = telem.lock().unwrap();
        t.bytes_sent += body_bytes.len() as u64;
        if log_bodies && !body_bytes.is_empty() {
            let preview_len = body_bytes.len().min(max_body);
            if let Ok(s) = std::str::from_utf8(&body_bytes[..preview_len]) {
                t.request_body_preview = Some(s.to_string());
            }
        }
    }

    // Connect upstream TLS.
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

    let upstream_tcp = tokio::net::TcpStream::connect(format!("{domain}:443")).await?;
    let server_name = rustls::pki_types::ServerName::try_from(domain.to_string())?;
    let upstream_tls = connector.connect(server_name, upstream_tcp).await?;

    if is_upgrade {
        // WebSocket upgrade: log metadata, then blind bridge.
        let resp = hyper::Response::builder()
            .status(101)
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .body(Full::new(Bytes::new()))
            .unwrap();
        // In a full implementation we'd bridge the streams here.
        // For now, just return the 101 and let the connection close.
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
    let mut builder = hyper::Request::builder()
        .method(original_method)
        .uri(&path);
    for (name, value) in original_headers.iter() {
        if name != "host" {
            builder = builder.header(name.clone(), value.clone());
        }
    }
    builder = builder.header("host", domain);
    let upstream_req = builder.body(Full::new(body_bytes))?;

    let resp = sender.send_request(upstream_req).await?;
    let resp_status = resp.status().as_u16();

    // Capture response headers.
    let resp_hdrs = format_headers(resp.headers());

    // Collect response body.
    let resp_body = resp.collect().await?.to_bytes();

    {
        let mut t = telem.lock().unwrap();
        t.status_code = Some(resp_status);
        t.response_headers = Some(resp_hdrs);
        t.bytes_received += resp_body.len() as u64;
        if log_bodies && !resp_body.is_empty() {
            let preview_len = resp_body.len().min(max_body);
            if let Ok(s) = std::str::from_utf8(&resp_body[..preview_len]) {
                t.response_body_preview = Some(s.to_string());
            }
        }
    }

    // Build downstream response.
    let response = hyper::Response::builder()
        .status(resp_status)
        .body(Full::new(resp_body))
        .unwrap();

    Ok(response)
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
    use crate::net::domain_policy::DomainPolicy;
    use crate::net::http_policy::HttpPolicy;

    const CA_KEY: &str = include_str!("../../../../config/capsem-ca.key");
    const CA_CERT: &str = include_str!("../../../../config/capsem-ca.crt");

    fn make_config(default_action: Action) -> Arc<MitmProxyConfig> {
        let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
        let dp = DomainPolicy::new(&[], &["evil.com".to_string()], default_action);
        let policy = Arc::new(HttpPolicy::from_domain_policy(dp));
        let web_db = Arc::new(Mutex::new(WebDb::open_in_memory().unwrap()));
        Arc::new(MitmProxyConfig { ca, policy, web_db })
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
    async fn domain_denied_closes_immediately() {
        let config = make_config(Action::Deny);
        let (mut s1, s2) = UnixStream::pair().unwrap();

        // Send ClientHello for "evil.com" (in block-list).
        std::io::Write::write_all(&mut s1, &make_client_hello("evil.com")).unwrap();
        drop(s1);

        let fd = s2.into_raw_fd();
        handle_connection(fd, config.clone()).await;

        let db = config.web_db.lock().unwrap();
        let events = db.recent(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "evil.com");
        assert_eq!(events[0].decision, Decision::Denied);
        assert_eq!(events[0].conn_type, Some("https-mitm".to_string()));
    }

    #[tokio::test]
    async fn no_sni_closes_immediately() {
        let config = make_config(Action::Allow);
        let (mut s1, s2) = UnixStream::pair().unwrap();

        std::io::Write::write_all(&mut s1, b"not a client hello").unwrap();
        drop(s1);

        handle_connection(s2.into_raw_fd(), config.clone()).await;

        let db = config.web_db.lock().unwrap();
        let events = db.recent(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "<unknown>");
        assert_eq!(events[0].decision, Decision::Denied);
    }

    #[tokio::test]
    async fn empty_connection_records_error() {
        let config = make_config(Action::Allow);
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
        // Test the ReplayReader independently.
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
    // AsyncFdStream tests: exercise the exact fd setup from handle_inner
    // using Unix socket pairs (closest analog to vsock fds).
    // ---------------------------------------------------------------

    /// Reproduce the exact fd setup from handle_inner: ManuallyDrop + try_clone + set_nonblocking + AsyncFd.
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

        // Write from stream1, read from stream2.
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

        // 128KB transfer -- exercises partial writes and reads.
        let data: Vec<u8> = (0..131072).map(|i| (i % 251) as u8).collect();
        let send_data = data.clone();
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut stream1, &send_data).await.unwrap();
            // Close the fd to signal EOF (shutdown is a no-op on AsyncFdStream).
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

        // Write some data then close.
        {
            let mut stream1 = wrap_fd_like_handle_inner(fd1);
            tokio::io::AsyncWriteExt::write_all(&mut stream1, b"before eof").await.unwrap();
            // stream1 drops here, but ManuallyDrop means fd1 is still open.
        }
        // Close the original fd to signal EOF.
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

        // Ping-pong: write from 1, read from 2, write from 2, read from 1.
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
        // Simulate the MITM proxy pattern: read initial bytes,
        // then feed them back via ReplayReader + live stream.
        let (s1, s2) = UnixStream::pair().unwrap();
        let fd2 = s2.into_raw_fd();
        let mut stream2 = wrap_fd_like_handle_inner(fd2);

        // Peer writes two chunks.
        let mut writer = s1;
        std::io::Write::write_all(&mut writer, b"INITIAL").unwrap();
        std::io::Write::write_all(&mut writer, b"REMAINING").unwrap();
        drop(writer);

        // Read initial bytes (simulating ClientHello peek).
        let mut initial = vec![0u8; 7]; // exactly "INITIAL"
        tokio::io::AsyncReadExt::read_exact(&mut stream2, &mut initial).await.unwrap();
        assert_eq!(&initial, b"INITIAL");

        // Wrap in ReplayReader (feeds initial bytes back, then continues reading).
        let mut replay = ReplayReader::new(initial, stream2);
        let mut all = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut replay, &mut all).await.unwrap();
        assert_eq!(&all, b"INITIALREMAINING");

        unsafe { libc::close(fd2); }
    }

    /// Full TLS handshake through handle_connection using a real rustls client.
    ///
    /// This test exercises the ServerConfig::builder_with_provider() path that
    /// previously panicked when no global CryptoProvider was installed.
    /// It does NOT call install_default() -- the proxy must bring its own provider.
    #[tokio::test]
    async fn tls_handshake_completes_without_global_provider() {
        let config = make_config(Action::Allow);
        let (s1, s2) = UnixStream::pair().unwrap();

        let proxy_fd = s2.into_raw_fd();
        let proxy_config = Arc::clone(&config);
        let proxy_task = tokio::spawn(async move {
            handle_connection(proxy_fd, proxy_config).await;
        });

        // Build a TLS client that trusts the MITM CA, also with explicit provider.
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

        // Connect through the Unix socket pair with TLS.
        s1.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(s1).unwrap();
        let domain = rustls::pki_types::ServerName::try_from("example.com").unwrap();
        let tls_result = connector.connect(domain, stream).await;

        // The TLS handshake itself must succeed (cert minting + ServerHello).
        // The HTTP layer may fail (no upstream), but we only care about TLS here.
        assert!(tls_result.is_ok(), "TLS handshake failed: {:?}", tls_result.err());

        // Drop the TLS stream so the proxy can finish.
        drop(tls_result);
        let _ = proxy_task.await;
    }
}
