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
use std::sync::{Arc, Mutex};

use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy::{DomainMatcher, NetworkPolicy, PolicyRule};
use capsem_core::net::telemetry::{Decision, WebDb};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

const CA_KEY: &str = include_str!("../../../config/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../config/capsem-ca.crt");

/// Build a NetworkPolicy from allow/block lists for integration tests.
///
/// - Blocked domains: read=false, write=false
/// - Allowed domains: read=true, write=true
/// - default_allow: controls what happens to unlisted domains
fn make_proxy_config(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> (Arc<MitmProxyConfig>, Arc<Mutex<WebDb>>) {
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
    let web_db = Arc::new(Mutex::new(WebDb::open_in_memory().unwrap()));
    let config = Arc::new(MitmProxyConfig {
        ca,
        policy,
        web_db: web_db.clone(),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
    });
    (config, web_db)
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
    let (config, web_db) = make_proxy_config(&["elie.net"], &[], false);
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

    // Verify telemetry.
    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty(), "should have recorded a telemetry event");
    assert_eq!(events[0].domain, "elie.net");
    assert_eq!(events[0].decision, Decision::Allowed);
    assert_eq!(events[0].method.as_deref(), Some("HEAD"));
    assert!(events[0].status_code.is_some());
    assert_eq!(events[0].conn_type.as_deref(), Some("https-mitm"));
}

#[tokio::test]
async fn mitm_proxy_denies_forbidden_domain() {
    let (config, web_db) = make_proxy_config(&[], &["example.com"], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    // TLS now completes even for denied domains (we mint a cert to capture HTTP details).
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("example.com").unwrap();
    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake should succeed (denial happens at HTTP level)");

    // Send HTTP request -- expect 403.
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

    // Verify telemetry records the denial with method and path.
    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty(), "should have recorded denial event");
    assert_eq!(events[0].domain, "example.com");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].method.as_deref(), Some("GET"));
    assert_eq!(events[0].path.as_deref(), Some("/test"));
    assert_eq!(events[0].status_code, Some(403));
}

#[tokio::test]
async fn mitm_proxy_denies_default_deny_unlisted_domain() {
    // Default-deny policy with no allow-list: all domains rejected.
    let (config, web_db) = make_proxy_config(&[], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("unlisted-domain.test").unwrap();
    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake should succeed (denial happens at HTTP level)");

    // Send HTTP request -- expect 403.
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

    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].domain, "unlisted-domain.test");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].method.as_deref(), Some("POST"));
    assert_eq!(events[0].path.as_deref(), Some("/api/data"));
}

#[tokio::test]
async fn mitm_proxy_records_http_method_and_path() {
    let (config, web_db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("elie.net").unwrap();
    let tls = connector.connect(domain, tcp).await.unwrap();

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    // GET a specific path so we can verify telemetry captures method + path.
    let req = hyper::Request::builder()
        .method("GET")
        .uri("/about")
        .header("host", "elie.net")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert!(resp.status().as_u16() < 500);
    // Consume the response body so hyper releases the connection.
    let _ = resp.into_body().collect().await;

    drop(sender);
    proxy_task.await.unwrap();

    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].method.as_deref(), Some("GET"));
    // Path might have been redirected, but should start with /about or be recorded.
    assert!(events[0].path.is_some());
}

#[tokio::test]
async fn mitm_proxy_denies_bad_upstream_cert() {
    let (config, web_db) = make_proxy_config(&["expired.badssl.com"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("expired.badssl.com").unwrap();
    
    // The proxy will successfully complete the TLS handshake with the client using its MITM CA.
    // The failure happens when the proxy attempts to connect upstream during the HTTP request.
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

    let resp_result = sender.send_request(req).await;
    
    // The proxy drops the connection (or hyper fails) because the upstream TLS fails.
    assert!(resp_result.is_err(), "Proxy should drop HTTP connection when upstream cert is bad");

    proxy_task.await.unwrap();

    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty(), "Proxy should record the telemetry for failed upstream cert");
    assert_eq!(events[0].domain, "expired.badssl.com");
    // Depending on exactly when it drops, it might be Denied or Error. Both mean it failed safely.
    assert!(matches!(events[0].decision, Decision::Error | Decision::Denied), "Decision should be Error or Denied");
}

#[tokio::test]
async fn mitm_proxy_handles_garbage_data() {
    // Allow elie.net, but we will send garbage instead of valid SNI/TLS
    let (config, web_db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    
    // Write random garbage instead of a valid TLS ClientHello
    let garbage: Vec<u8> = (0..1024).map(|i| (i % 255) as u8).collect();
    tcp.write_all(&garbage).await.unwrap();
    
    // Read to ensure proxy closes the connection without hanging or panicking.
    // The proxy may send back TLS alert bytes before closing, so accept any response.
    let mut buf = vec![0u8; 1024];
    let _ = tcp.read(&mut buf).await;

    proxy_task.await.unwrap();

    let events = web_db.lock().unwrap().recent(10).unwrap();
    // It shouldn't even parse a domain to log, or it might log <unknown>
    if !events.is_empty() {
        assert!(matches!(events[0].decision, Decision::Error | Decision::Denied));
    }
}

#[tokio::test]
async fn mitm_proxy_streams_large_payload() {
    let (config, web_db) = make_proxy_config(&["httpbin.org"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let domain = ServerName::try_from("httpbin.org").unwrap();
    let tls = connector.connect(domain, tcp).await.unwrap();

    let io = TokioIo::new(tls);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(conn);

    // 1MB payload to test streaming
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

    let events = web_db.lock().unwrap().recent(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].method.as_deref(), Some("POST"));
    // Ensure that bytes sent reflects the large payload
    assert!(events[0].bytes_sent >= payload_size as u64, "Recorded telemetry bytes_sent {} is smaller than payload size {}", events[0].bytes_sent, payload_size);
}
