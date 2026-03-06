/// Integration tests for the MITM proxy -- end-to-end TLS interception.
///
/// These tests spin up the MITM proxy on a local TCP socket (simulating vsock),
/// connect a real TLS client through it, and verify:
/// - Allowed domains complete a full HTTPS request/response cycle
/// - Denied domains are rejected before TLS handshake completes
/// - Telemetry records correct decisions, methods, and status codes
///
/// Requires internet access (the proxy connects upstream to real servers).
use std::os::unix::io::IntoRawFd;
use std::sync::Arc;

use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy::{DomainMatcher, NetworkPolicy, PolicyRule};
use capsem_logger::{DbWriter, Decision};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

const CA_KEY: &str = include_str!("../../../config/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../config/capsem-ca.crt");

/// Build a NetworkPolicy from allow/block lists for integration tests.
fn make_proxy_config(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let mut rules = Vec::new();
    for pattern in blocked {
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(pattern),
            allow_read: false,
            allow_write: false,
        });
    }
    for pattern in allowed {
        rules.push(PolicyRule {
            matcher: DomainMatcher::parse(pattern),
            allow_read: true,
            allow_write: true,
        });
    }
    let policy = Arc::new(std::sync::RwLock::new(Arc::new(NetworkPolicy::new(rules, default_allow, default_allow))));
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    // Leak the tempdir so it lives for the test
    std::mem::forget(dir);
    let config = Arc::new(MitmProxyConfig {
        ca,
        policy,
        db: db.clone(),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
        pricing: capsem_core::gateway::pricing::PricingTable::load(),
        trace_state: std::sync::Mutex::new(capsem_core::gateway::TraceState::new()),
    });
    (config, db)
}

/// Build a rustls ClientConfig that trusts the Capsem MITM CA.
fn make_tls_client_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    let certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    for cert in certs {
        root_store.add(cert).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// Spawn the MITM proxy on a TCP listener and return the address.
/// The proxy handles exactly one connection then exits.
async fn spawn_proxy(
    config: Arc<MitmProxyConfig>,
) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let fd = std_stream.into_raw_fd();
        mitm_proxy::handle_connection(fd, config).await;
        // fd ownership: ManuallyDrop inside handle_connection prevents double-close.
        // We own the original fd, close it here.
        unsafe { libc::close(fd) };
    });

    (handle, addr)
}

#[tokio::test]
async fn mitm_proxy_allows_elie_net() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    // Connect through the proxy with TLS trusting our MITM CA.
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("elie.net").unwrap();
    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake to allowed domain should succeed");

    // Send HTTP HEAD request through the MITM proxy.
    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("HEAD")
        .uri("/")
        .header("host", "elie.net")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    assert!(
        status < 500,
        "expected success/redirect from elie.net, got {status}"
    );

    // Close the connection so the proxy finishes and records telemetry.
    drop(sender);
    proxy_task.await.unwrap();

    // Give writer thread time to flush.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify telemetry.
    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty(), "should have recorded a telemetry event");
    assert_eq!(events[0].domain, "elie.net");
    assert_eq!(events[0].decision, Decision::Allowed);
    assert_eq!(events[0].method.as_deref(), Some("HEAD"));
    assert!(events[0].status_code.is_some());
    assert_eq!(events[0].conn_type.as_deref(), Some("https-mitm"));
}

#[tokio::test]
async fn mitm_proxy_denies_forbidden_domain() {
    let (config, db) = make_proxy_config(&[], &["example.com"], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("example.com").unwrap();
    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake should succeed (denial happens at HTTP level)");

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/test")
        .header("host", "example.com")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403, "denied domain should return 403");

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty(), "should have recorded denial event");
    assert_eq!(events[0].domain, "example.com");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].method.as_deref(), Some("GET"));
    assert_eq!(events[0].path.as_deref(), Some("/test"));
    assert_eq!(events[0].status_code, Some(403));
}

#[tokio::test]
async fn mitm_proxy_denies_default_deny_unlisted_domain() {
    let (config, db) = make_proxy_config(&[], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("unlisted-domain.test").unwrap();
    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake should succeed (denial happens at HTTP level)");

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("POST")
        .uri("/api/data")
        .header("host", "unlisted-domain.test")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].domain, "unlisted-domain.test");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].method.as_deref(), Some("POST"));
    assert_eq!(events[0].path.as_deref(), Some("/api/data"));
}

#[tokio::test]
async fn mitm_proxy_records_http_method_and_path() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("elie.net").unwrap();
    let tls = connector.connect(domain, tcp).await.unwrap();

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/about")
        .header("host", "elie.net")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert!(resp.status().as_u16() < 500);
    let _ = resp.into_body().collect().await;

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].method.as_deref(), Some("GET"));
    assert!(events[0].path.is_some());
}

#[tokio::test]
async fn mitm_proxy_denies_bad_upstream_cert() {
    let (config, db) = make_proxy_config(&["expired.badssl.com"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("expired.badssl.com").unwrap();

    let tls = connector.connect(domain, tcp).await.expect("TLS handshake to proxy should succeed");

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .header("host", "expired.badssl.com")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 502, "Bad upstream cert should return 502");
    let _ = resp.into_body().collect().await;

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty(), "Proxy should record the telemetry for failed upstream cert");
    assert_eq!(events[0].domain, "expired.badssl.com");
    assert_eq!(events[0].decision, Decision::Error);
    assert_eq!(events[0].status_code, Some(502));
}

#[tokio::test]
async fn mitm_proxy_handles_garbage_data() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();

    let garbage: Vec<u8> = (0..1024).map(|i| (i % 255) as u8).collect();
    tcp.write_all(&garbage).await.unwrap();

    let mut buf = vec![0u8; 1024];
    let _ = tcp.read(&mut buf).await;

    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    if !events.is_empty() {
        assert!(matches!(events[0].decision, Decision::Error | Decision::Denied));
    }
}

#[tokio::test]
async fn mitm_proxy_streams_large_payload() {
    let (config, db) = make_proxy_config(&["httpbin.org"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("httpbin.org").unwrap();
    let tls = connector.connect(domain, tcp).await.unwrap();

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let payload_size = 1024 * 1024;
    let large_body = vec![b'A'; payload_size];

    let req = hyper::Request::builder()
        .method("POST")
        .uri("/post")
        .header("host", "httpbin.org")
        .body(Full::new(Bytes::from(large_body)))
        .unwrap();

    let resp = sender.send_request(req).await.unwrap();
    assert!(resp.status().as_u16() < 500, "Large streaming request failed");

    let _ = resp.into_body().collect().await;

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].method.as_deref(), Some("POST"));
    assert!(events[0].bytes_sent >= payload_size as u64, "Recorded telemetry bytes_sent {} is smaller than payload size {}", events[0].bytes_sent, payload_size);
}

/// Multiple requests on one keep-alive connection reuse the upstream connection.
/// This verifies the per-connection pooling produces correct telemetry for each request.
#[tokio::test]
async fn multiple_requests_reuse_upstream_connection() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("elie.net").unwrap();
    let tls = connector.connect(domain, tcp).await.unwrap();

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    // Send 3 requests on the same keep-alive connection.
    for path in ["/", "/about", "/contact"] {
        let req = hyper::Request::builder()
            .method("HEAD")
            .uri(path)
            .header("host", "elie.net")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert!(
            resp.status().as_u16() < 500,
            "request to {path} failed with {}",
            resp.status()
        );
        let _ = resp.into_body().collect().await;
    }

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        3,
        "3 keep-alive requests should produce 3 telemetry events"
    );
    for event in &events {
        assert_eq!(event.domain, "elie.net");
        assert_eq!(event.decision, Decision::Allowed);
        assert_eq!(event.method.as_deref(), Some("HEAD"));
    }
}

/// Download 100 MB through the MITM proxy and assert throughput >= 1 MB/s.
///
/// Exercises the full proxy pipeline on the host: TLS termination from the
/// "guest" client, upstream TLS to a real CDN, and body streaming back.
/// Marked #[ignore] so it doesn't run on every `cargo test` -- run explicitly
/// with `cargo test -p capsem-core -- --ignored mitm_proxy_download_throughput`.
#[tokio::test]
#[ignore = "downloads 100 MB; run explicitly to test proxy throughput"]
async fn mitm_proxy_download_throughput() {
    const DOMAIN: &str = "ash-speed.hetzner.com";
    const PATH: &str = "/100MB.bin";
    const EXPECTED_BYTES: u64 = 100 * 1024 * 1024;
    const MIN_MBPS: f64 = 1.0;

    let (config, _db) = make_proxy_config(&[DOMAIN], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let sni = ServerName::try_from(DOMAIN).unwrap();
    let tls = connector
        .connect(sni, tcp)
        .await
        .expect("TLS handshake to ash-speed.hetzner.com should succeed");

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    let req = hyper::Request::builder()
        .method("GET")
        .uri(PATH)
        .header("host", DOMAIN)
        .body(Full::new(Bytes::new()))
        .unwrap();

    let start = std::time::Instant::now();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 from {DOMAIN}, got {status}");

    // Stream body without buffering 100 MB in one allocation.
    let mut body = resp.into_body();
    let mut total_bytes: u64 = 0;
    loop {
        match BodyExt::frame(&mut body).await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    total_bytes += data.len() as u64;
                }
            }
            Some(Err(e)) => panic!("body error: {e}"),
            None => break,
        }
    }

    let elapsed = start.elapsed();
    let mbps = (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
    println!(
        "\nProxy throughput: {:.1} MB in {:.2}s = {:.2} MB/s",
        total_bytes as f64 / (1024.0 * 1024.0),
        elapsed.as_secs_f64(),
        mbps,
    );

    drop(sender);
    let _ = proxy_task.await;

    assert!(
        total_bytes >= EXPECTED_BYTES,
        "incomplete download: {:.1} MB (expected 100 MB)",
        total_bytes as f64 / (1024.0 * 1024.0)
    );
    assert!(
        mbps >= MIN_MBPS,
        "throughput too low: {mbps:.2} MB/s (minimum {MIN_MBPS} MB/s)"
    );
}
