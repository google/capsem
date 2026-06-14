/// Integration tests for the MITM proxy -- end-to-end TLS interception.
///
/// These tests spin up the MITM proxy on a local TCP socket (simulating vsock),
/// connect a real TLS client through it, and verify:
/// - Allowed domains complete a full HTTPS request/response cycle
/// - Denied domains are rejected before TLS handshake completes
/// - Telemetry records correct decisions, methods, and status codes
///
/// Requires internet access (the proxy connects upstream to real servers).
use std::collections::BTreeMap;
use std::os::unix::io::IntoRawFd;
use std::sync::Arc;

use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy::NetworkPolicy;
use capsem_logger::{DbWriter, Decision};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

const CA_KEY: &str = include_str!("../../../security/keys/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../security/keys/capsem-ca.crt");

/// Build a proxy config from allow/block lists for integration tests.
///
/// Enforcement intent is compiled into `SecurityRuleSet` so tests exercise the
/// same security-event/CEL rail as production. `NetworkPolicy` remains present
/// for non-enforcement proxy settings such as body capture and HTTP port gates.
fn make_proxy_config(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    make_proxy_config_full(allowed, blocked, default_allow, &[80])
}

fn host_pattern_condition(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let escaped = regex::escape(suffix);
        return Some(format!("http.host.matches(\"(^|.*\\\\.){escaped}$\")"));
    }
    Some(format!("http.host == \"{}\"", pattern.replace('"', "\\\"")))
}

fn host_pattern_negative_condition(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let escaped = regex::escape(suffix);
        return Some(format!(
            "http.host.matches(\"(^|.*\\\\.){escaped}$\") == false"
        ));
    }
    Some(format!("http.host != \"{}\"", pattern.replace('"', "\\\"")))
}

fn security_rules_for_proxy(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> capsem_core::net::policy_config::SecurityRuleSet {
    let mut toml = String::new();
    let blocked_conditions: Vec<String> = blocked
        .iter()
        .filter_map(|pattern| host_pattern_condition(pattern))
        .collect();
    if !blocked_conditions.is_empty() {
        toml.push_str(
            r#"
[profiles.rules.block_test_hosts]
name = "block_test_hosts"
action = "block"
reason = "test blocked host"
match = '''
"#,
        );
        toml.push_str(&blocked_conditions.join("\n|| "));
        toml.push_str(
            r#"
'''
"#,
        );
    }

    if !default_allow {
        let allowed_conditions: Vec<String> = allowed
            .iter()
            .filter_map(|pattern| host_pattern_negative_condition(pattern))
            .collect();
        toml.push_str(
            r#"
[profiles.rules.block_test_default_deny]
name = "block_test_default_deny"
action = "block"
reason = "test default deny"
match = '''
"#,
        );
        if allowed_conditions.is_empty() {
            toml.push_str("http.host != \"\"");
        } else {
            toml.push_str(&allowed_conditions.join("\n&& "));
        }
        toml.push_str(
            r#"
'''
"#,
        );
    }

    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(&toml)
        .expect("test security rule profile");
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("test security rules")
}

/// Like `make_proxy_config` but lets the caller override the
/// `http_upstream_ports` allowlist (T2.2). Used by T2.3's Ollama-shape
/// test that runs a fake upstream on an OS-assigned port.
fn make_proxy_config_full(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
    http_ports: &[u16],
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    make_proxy_config_with_security_rules(
        security_rules_for_proxy(allowed, blocked, default_allow),
        http_ports,
    )
}

fn make_proxy_config_with_security_rules(
    security_rules: capsem_core::net::policy_config::SecurityRuleSet,
    http_ports: &[u16],
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let mut policy_inner = NetworkPolicy::new();
    policy_inner.http_upstream_ports = http_ports.to_vec();
    let policy = Arc::new(std::sync::RwLock::new(Arc::new(policy_inner)));
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    // Leak the tempdir so it lives for the test
    std::mem::forget(dir);
    let telemetry = Arc::new(mitm_proxy::telemetry_hook::TelemetryDeps {
        db: db.clone(),
        pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::new(std::sync::Mutex::new(
            capsem_core::net::ai_traffic::TraceState::new(),
        )),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(security_rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
    });
    let pipeline =
        mitm_proxy::make_production_pipeline(Arc::clone(&policy), Arc::clone(&telemetry));
    let config = Arc::new(MitmProxyConfig {
        ca,
        policy,
        model_endpoints: Arc::new(std::sync::RwLock::new(Arc::new(
            capsem_core::net::policy_config::ProviderRuleProfile::builtin_defaults()
                .endpoint_registry()
                .expect("builtin provider endpoint registry"),
        ))),
        db: db.clone(),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
        telemetry,
        pipeline,
        mcp_endpoint: None,
    });
    (config, db)
}

fn security_rules_from_toml(toml: &str) -> capsem_core::net::policy_config::SecurityRuleSet {
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(toml)
        .expect("test security rule profile");
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("test security rules")
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
    assert_eq!(
        resp.status().as_u16(),
        403,
        "denied domain should return 403"
    );

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

    let tls = connector
        .connect(domain, tcp)
        .await
        .expect("TLS handshake to proxy should succeed");

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
    assert_eq!(
        resp.status().as_u16(),
        502,
        "Bad upstream cert should return 502"
    );
    let _ = resp.into_body().collect().await;

    drop(sender);
    proxy_task.await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(
        !events.is_empty(),
        "Proxy should record the telemetry for failed upstream cert"
    );
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
        assert!(matches!(
            events[0].decision,
            Decision::Error | Decision::Denied
        ));
    }
}

/// T2.2: a plain-HTTP request to a non-allowlisted domain reaches
/// the security-event boundary and is denied with 403 -- proving the plain-HTTP path
/// now serves through the same hyper pipeline as TLS, with the same
/// policy gates. (T2.1 would have stopped at the sniff with an
/// Error connection event.)
#[tokio::test]
async fn mitm_proxy_plain_http_denies_disallowed_host() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    // Plain HTTP/1.1 request directly on the TCP socket, no TLS,
    // no \0CAPSEM_META prefix. Host is not on the allowlist (which
    // is "elie.net" only); default-deny applies -> 403 from
    // the security-event boundary.
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n")
        .await
        .unwrap();

    // Drain the response (a 403 produced by the security-event boundary).
    let mut buf = vec![0u8; 4096];
    let _ = tcp.read(&mut buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty(), "plain HTTP path must record a NetEvent");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].domain, "example.com");
    assert_eq!(events[0].method.as_deref(), Some("GET"));
    assert_eq!(
        events[0].port, 80,
        "plain HTTP defaults to upstream port 80"
    );
}

/// T2.2: a plain-HTTP request whose Host carries a port not on the
/// `http_upstream_ports` allowlist is rejected with 403 before the
/// upstream dial. Default allowlist is `[80]`.
#[tokio::test]
async fn mitm_proxy_plain_http_denies_port_not_in_allowlist() {
    // Allow elie.net (so the domain policy passes) but keep the
    // default port allowlist = [80]. The request explicitly
    // targets port 8080, which must be denied at the port gate.
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: elie.net:8080\r\n\r\n")
        .await
        .unwrap();

    let mut buf = vec![0u8; 4096];
    let _ = tcp.read(&mut buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(
        !events.is_empty(),
        "port-denied path must record a NetEvent"
    );
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].port, 8080);
    let reason = events[0].matched_rule.as_deref().unwrap_or("");
    assert!(
        reason.contains("http-port-not-allowlisted"),
        "expected port-not-allowlisted marker, got matched_rule={reason:?}"
    );
}

/// T2.3: Ollama-shaped end-to-end. A fake plain-HTTP upstream binds
/// on `127.0.0.1:0`; the proxy is configured with that port on its
/// `http_upstream_ports` allowlist and `127.0.0.1` on the domain
/// allowlist. We send `POST /api/generate` with the typical Ollama
/// request shape through the proxy and verify the response is
/// forwarded verbatim from the upstream and `NetEvent` records
/// method/path/status/port/conn_type correctly.
#[tokio::test]
async fn mitm_proxy_plain_http_ollama_shape_records_telemetry() {
    // 1. Fake plain-HTTP upstream. Reads one request (we don't
    //    bother validating its bytes), sends a fixed Ollama-shaped
    //    response, closes.
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let upstream_task = tokio::spawn(async move {
        let (mut sock, _) = upstream_listener.accept().await.unwrap();
        // Drain headers (read until "\r\n\r\n") so the upstream
        // doesn't write before the request is fully sent.
        let mut buf = [0u8; 4096];
        let mut total = 0usize;
        while total < buf.len() {
            let n = sock.read(&mut buf[total..]).await.unwrap();
            if n == 0 {
                break;
            }
            total += n;
            if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let body = b"{\"model\":\"llama2\",\"response\":\"hello\",\"done\":true}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
        sock.flush().await.unwrap();
        // Hold the socket briefly so the proxy has time to read.
        let _ = sock.shutdown().await;
    });

    // 2. Build a proxy config that allows 127.0.0.1 + the
    //    OS-assigned upstream port.
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    // 3. Send a plain HTTP/1.1 POST through the proxy. The Host
    //    header carries the OS-assigned port -- this is exactly the
    //    Ollama redirect shape the guest's iptables rules will
    //    eventually produce (the local-LLM server is on a non-80
    //    port, but the request stays plaintext).
    let req_body = b"{\"model\":\"llama2\",\"prompt\":\"Hi\"}";
    let req_head = format!(
        "POST /api/generate HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        upstream_port,
        req_body.len(),
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(req_body).await.unwrap();
    tcp.flush().await.unwrap();

    // 4. Read response from the proxy.
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();

    // 5. Response is forwarded verbatim.
    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.contains("HTTP/1.1 200"),
        "expected 200 OK forwarded, got:\n{resp_text}"
    );
    assert!(
        resp_text.contains("\"response\":\"hello\""),
        "expected upstream body forwarded, got:\n{resp_text}"
    );

    // 6. Telemetry records the right fields.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1, "exactly one NetEvent for one request");
    let ev = &events[0];
    assert_eq!(ev.decision, Decision::Allowed);
    assert_eq!(ev.method.as_deref(), Some("POST"));
    assert_eq!(ev.path.as_deref(), Some("/api/generate"));
    assert_eq!(ev.status_code, Some(200));
    assert_eq!(ev.domain, "127.0.0.1");
    assert_eq!(
        ev.port, upstream_port,
        "port reflects upstream Host: header port"
    );
    assert_eq!(ev.conn_type.as_deref(), Some("http-mitm"));
    assert!(
        ev.bytes_sent >= req_body.len() as u64,
        "bytes_sent {} should cover the request body",
        ev.bytes_sent
    );
    assert!(ev.bytes_received > 0, "bytes_received should be non-zero");
}

/// T2.2 plain-HTTP fake upstream helper. Spins a TCP listener on
/// `127.0.0.1:0`, accepts ONE connection, runs `serve(socket)` to
/// completion. Returns the bound port + the join handle for the
/// recorded fields. Each test parameterizes `serve` with a closure
/// that reads the request and writes the response shape it wants
/// to validate (chunked, fixed-length, slow, etc.).
async fn spawn_fake_upstream<F>(serve: F) -> (u16, tokio::task::JoinHandle<Vec<u8>>)
where
    F: FnOnce(
            tokio::net::TcpStream,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<u8>> + Send>>
        + Send
        + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        serve(sock).await
    });
    (port, task)
}

/// Drain an HTTP/1.1 request from a TCP stream until the request body
/// (per Content-Length) has been fully read. Returns the raw bytes
/// (head + body) the upstream observed.
async fn read_http11_request(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut head_end: Option<usize> = None;
    let mut content_length: usize = 0;
    loop {
        let n = sock.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if head_end.is_none() {
            if let Some(idx) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                head_end = Some(idx + 4);
                let head = std::str::from_utf8(&buf[..idx]).unwrap_or("");
                for line in head.split("\r\n") {
                    if let Some(v) = line
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                    {
                        content_length = v.parse().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(end) = head_end {
            if buf.len() >= end + content_length {
                break;
            }
        }
    }
    buf
}

/// T2.2: a POST request with a body has its body bytes forwarded
/// verbatim to upstream and `NetEvent.bytes_sent` includes the body
/// length. The response is forwarded back to the client unchanged.
#[tokio::test]
async fn mitm_proxy_plain_http_post_forwards_body_and_records_bytes_sent() {
    let req_body = br#"{"prompt":"hello world","n":42}"#;
    let req_body_len = req_body.len();

    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            let body = b"{\"ok\":true}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req_head = format!(
        "POST /v1/echo HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        upstream_port, req_body_len,
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(req_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // The upstream must have seen the request body verbatim.
    let recv = received.lock().unwrap().clone();
    let recv_str = std::str::from_utf8(&recv).unwrap_or("");
    assert!(
        recv_str.contains(r#""prompt":"hello world""#),
        "upstream did not see the request body: {recv_str:?}"
    );

    // The response body must have come back to the client.
    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.contains("HTTP/1.1 200"),
        "no 200 from proxy:\n{resp_text}"
    );
    assert!(
        resp_text.contains(r#""ok":true"#),
        "response body lost:\n{resp_text}"
    );

    // NetEvent must reflect bytes_sent >= req body, status 200, POST.
    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.method.as_deref(), Some("POST"));
    assert_eq!(ev.path.as_deref(), Some("/v1/echo"));
    assert_eq!(ev.status_code, Some(200));
    assert_eq!(ev.port, upstream_port);
    assert_eq!(ev.conn_type.as_deref(), Some("http-mitm"));
    assert!(
        ev.bytes_sent >= req_body_len as u64,
        "bytes_sent {} should cover request body of {} bytes",
        ev.bytes_sent,
        req_body_len,
    );
}

#[tokio::test]
async fn mitm_proxy_plain_http_unknown_openai_shape_emits_model_call() {
    let req_body = br#"{"model":"gpt-4.1","messages":[{"role":"user","content":"hello from private gateway"}]}"#;
    let req_body_len = req_body.len();

    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            let body = br#"{"id":"chatcmpl-test","object":"chat.completion","model":"gpt-4.1","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req_head = format!(
        "POST /private/model-gateway HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        upstream_port, req_body_len,
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(req_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let recv = received.lock().unwrap().clone();
    let recv_str = std::str::from_utf8(&recv).unwrap_or("");
    assert!(
        recv_str.contains(r#""hello from private gateway""#),
        "upstream did not receive the original private-gateway request body: {recv_str:?}"
    );

    let reader = db.reader().unwrap();
    let model_calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(
        model_calls.len(),
        1,
        "private gateway must emit one ModelCall"
    );
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(call.path, "/private/model-gateway");
    assert_eq!(call.status_code, Some(200));
    assert_eq!(call.request_bytes, req_body_len as u64);
    assert_eq!(call.input_tokens, Some(5));
    assert_eq!(call.output_tokens, Some(2));
}

#[tokio::test]
async fn mitm_proxy_plain_http_unknown_mcp_shape_emits_mcp_call() {
    let req_body = br#"{"jsonrpc":"2.0","id":"call-1","method":"tools/call","params":{"name":"search_web","arguments":{"q":"capsem"}}}"#;
    let req_body_len = req_body.len();

    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            let body = br#"{"jsonrpc":"2.0","id":"call-1","result":{"content":[{"type":"text","text":"ok"}]}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req_head = format!(
        "POST /remote-mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        upstream_port, req_body_len,
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(req_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let recv = received.lock().unwrap().clone();
    let recv_str = std::str::from_utf8(&recv).unwrap_or("");
    assert!(
        recv_str.contains(r#""method":"tools/call""#),
        "upstream did not receive the original MCP request body: {recv_str:?}"
    );

    let reader = db.reader().unwrap();
    let net_events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        net_events.len(),
        1,
        "MCP-over-HTTP still emits HTTP telemetry"
    );
    assert_eq!(net_events[0].path.as_deref(), Some("/remote-mcp"));

    let mcp_calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(
        mcp_calls.len(),
        1,
        "unknown remote MCP-over-HTTP must emit one McpCall"
    );
    let call = &mcp_calls[0];
    assert_eq!(call.method, "tools/call");
    assert_eq!(call.tool_name.as_deref(), Some("search_web"));
    assert_eq!(call.request_id.as_deref(), Some("call-1"));
    assert_eq!(call.decision, "allowed");
    assert_eq!(call.bytes_sent, req_body_len as u64);
    assert!(
        call.server_name.contains("127.0.0.1"),
        "observed MCP server identity should include host/path: {:?}",
        call.server_name
    );
}

#[tokio::test]
async fn mitm_proxy_plain_http_unknown_mcp_shape_can_be_blocked_by_mcp_rule() {
    let req_body = br#"{"jsonrpc":"2.0","id":"call-2","method":"tools/call","params":{"name":"search_web","arguments":{"q":"capsem"}}}"#;
    let req_body_len = req_body.len();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = listener.local_addr().unwrap().port();
    drop(listener);

    let rules = security_rules_from_toml(
        r#"
[profiles.rules.block_search_web_mcp]
name = "block_search_web_mcp"
action = "block"
reason = "test MCP block"
match = 'mcp.tool_call.name == "search_web"'
"#,
    );
    let (config, db) = make_proxy_config_with_security_rules(rules, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req_head = format!(
        "POST /remote-mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        upstream_port, req_body_len,
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(req_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.contains("HTTP/1.1 403"),
        "MCP rule did not block request:\n{resp_text}"
    );

    let reader = db.reader().unwrap();
    let mcp_calls = reader.recent_mcp_calls(10).unwrap();
    assert_eq!(
        mcp_calls.len(),
        1,
        "denied unknown MCP-over-HTTP must still emit one McpCall"
    );
    let call = &mcp_calls[0];
    assert_eq!(call.method, "tools/call");
    assert_eq!(call.tool_name.as_deref(), Some("search_web"));
    assert_eq!(call.decision, "denied");
    assert_eq!(
        call.policy_rule.as_deref(),
        Some("profiles.rules.block_search_web_mcp")
    );
}

/// T2.2: a chunked-transfer-encoding response from upstream is
/// streamed through the proxy frame-by-frame (the ChunkDispatchBody
/// runs the sync ChunkHook chain on every chunk). Verifies
/// `bytes_received` accumulates the full body across multiple
/// chunks, end-of-stream fires, and the client sees every chunk.
#[tokio::test]
async fn mitm_proxy_plain_http_chunked_streaming_response_aggregates_bytes() {
    let chunk_strs = ["alpha-data-", "beta-data-", "gamma-data-", "delta-end"];
    let total_body_bytes: usize = chunk_strs.iter().map(|s| s.len()).sum();
    let chunks_for_serve: Vec<Vec<u8>> = chunk_strs.iter().map(|s| s.as_bytes().to_vec()).collect();

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let _ = read_http11_request(&mut sock).await;
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            sock.write_all(head).await.unwrap();
            for c in &chunks_for_serve {
                let frame = format!("{:x}\r\n", c.len());
                sock.write_all(frame.as_bytes()).await.unwrap();
                sock.write_all(c).await.unwrap();
                sock.write_all(b"\r\n").await.unwrap();
                sock.flush().await.unwrap();
            }
            sock.write_all(b"0\r\n\r\n").await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = format!(
        "GET /stream HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        upstream_port,
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let resp_text = String::from_utf8_lossy(&resp_buf);
    for s in &chunk_strs {
        assert!(
            resp_text.contains(s),
            "client missed chunk {s:?} -- response body forwarding broke:\n{resp_text}"
        );
    }

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.status_code, Some(200));
    // bytes_received counts the post-decode body bytes the chunk
    // dispatcher saw (TelemetryHook::on_response_chunk sums chunk
    // lengths). Hyper de-frames the chunked encoding before passing
    // chunks to the response body, so this should equal the
    // concatenated payload.
    assert_eq!(
        ev.bytes_received as usize, total_body_bytes,
        "bytes_received should be the concatenated chunk lengths ({} bytes), got {}",
        total_body_bytes, ev.bytes_received,
    );
}

/// T2.2: three sequential requests on a single keep-alive client
/// connection emit three separate NetEvents, each with the correct
/// per-request fields. The upstream sees three back-to-back
/// requests. Validates the per-connection cached upstream sender
/// path on the plain-HTTP branch.
#[tokio::test]
async fn mitm_proxy_plain_http_keep_alive_emits_one_netevent_per_request() {
    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let mut total_received = Vec::new();
            // Serve N=3 requests on the same upstream-side connection
            // (we close after the 3rd).
            let n_requests = 3usize;
            for _ in 0..n_requests {
                let req = read_http11_request(&mut sock).await;
                total_received.extend_from_slice(&req);
                let body = b"ka-ok";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
                sock.write_all(body).await.unwrap();
                sock.flush().await.unwrap();
            }
            let _ = sock.shutdown().await;
            total_received
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    // Single client TCP connection, three back-to-back requests
    // (we send req N+1 only after the response to req N has been
    // fully drained, so this is keep-alive, not pipelining).
    async fn drain_response(tcp: &mut tokio::net::TcpStream, expected_body: &str) -> String {
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = tcp.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            let s = std::str::from_utf8(&acc).unwrap_or("");
            if s.contains(expected_body) {
                break;
            }
        }
        String::from_utf8_lossy(&acc).into_owned()
    }

    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let req_paths = ["/a", "/b", "/c"];
    for (idx, p) in req_paths.iter().enumerate() {
        let last = idx == req_paths.len() - 1;
        let conn_hdr = if last { "Connection: close\r\n" } else { "" };
        let req = format!(
            "GET {p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{conn_hdr}\r\n",
            port = upstream_port,
        );
        tcp.write_all(req.as_bytes()).await.unwrap();
        tcp.flush().await.unwrap();
        // Drain head + body. ka-ok marker gates per-request progress.
        let s = drain_response(&mut tcp, "ka-ok").await;
        assert!(
            s.contains("ka-ok"),
            "missing body in response for {p}:\n{s}"
        );
    }
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        3,
        "three keep-alive requests = three NetEvents"
    );
    // events[] is reverse-chronological per recent_net_events, but
    // the path field disambiguates regardless of order.
    let paths: std::collections::HashSet<&str> =
        events.iter().filter_map(|e| e.path.as_deref()).collect();
    assert!(paths.contains("/a"));
    assert!(paths.contains("/b"));
    assert!(paths.contains("/c"));
    for ev in &events {
        assert_eq!(ev.method.as_deref(), Some("GET"));
        assert_eq!(ev.status_code, Some(200));
        assert_eq!(ev.port, upstream_port);
        assert_eq!(ev.conn_type.as_deref(), Some("http-mitm"));
    }
}

/// T2.2: the inbound `Host` header is preserved verbatim to the
/// upstream (TLS path rewrites Host from SNI; plain HTTP must not).
/// Verifies via a fake upstream that records the raw request.
#[tokio::test]
async fn mitm_proxy_plain_http_preserves_host_header_to_upstream() {
    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            let resp = b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
            sock.write_all(resp).await.unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let host_value = format!("127.0.0.1:{upstream_port}");
    let req = format!("GET /headers HTTP/1.1\r\nHost: {host_value}\r\nConnection: close\r\n\r\n");
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();

    let recv = received.lock().unwrap().clone();
    let head = String::from_utf8_lossy(&recv);
    let expected = format!("host: {host_value}");
    let host_line_present = head
        .split("\r\n")
        .any(|l| l.eq_ignore_ascii_case(&expected));
    assert!(
        host_line_present,
        "upstream did not receive the inbound Host header verbatim. Saw:\n{head}"
    );
}

/// T2.2: a request to a plain-HTTP upstream that fails to dial
/// produces a 502 response and a NetEvent with Decision::Error +
/// matched_rule containing the dial error reason. No silent drop.
#[tokio::test]
async fn mitm_proxy_plain_http_unresolvable_upstream_emits_502_netevent() {
    // Reserved domain (RFC 6761) that DNS will NXDOMAIN. Default-deny
    // policy + explicit allow on the .invalid host so we get past
    // the security-event boundary into the upstream dial.
    let (config, db) = make_proxy_config_full(&["nonexistent.invalid"], &[], false, &[80, 11434]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = "GET /x HTTP/1.1\r\nHost: nonexistent.invalid\r\nConnection: close\r\n\r\n";
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.contains("HTTP/1.1 502"),
        "expected 502 from proxy on dial fail, got:\n{resp_text}"
    );

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        1,
        "dial failure should still emit one NetEvent"
    );
    let ev = &events[0];
    assert_eq!(ev.decision, Decision::Error);
    assert_eq!(ev.status_code, Some(502));
    assert_eq!(ev.method.as_deref(), Some("GET"));
    assert_eq!(ev.domain, "nonexistent.invalid");
    assert_eq!(ev.conn_type.as_deref(), Some("http-mitm"));
    let reason = ev.matched_rule.as_deref().unwrap_or("");
    assert!(
        !reason.is_empty(),
        "matched_rule should carry the underlying dial error",
    );
}

/// T2.2: every IETF HTTP method (GET / HEAD / OPTIONS / POST / PUT
/// / DELETE / PATCH) round-trips through the plain-HTTP path and
/// produces a NetEvent with the matching `method` field. Validates
/// the method is parsed correctly + propagated to telemetry across
/// both read-classified and write-classified verbs.
#[tokio::test]
async fn mitm_proxy_plain_http_records_every_http_method() {
    let methods = ["GET", "HEAD", "OPTIONS", "POST", "PUT", "DELETE", "PATCH"];

    let n_methods = methods.len();
    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            for _ in 0..n_methods {
                let _ = read_http11_request(&mut sock).await;
                // 204 No Content -- minimal, valid for any method.
                sock.write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                    .await
                    .unwrap();
                sock.flush().await.unwrap();
            }
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    for (idx, m) in methods.iter().enumerate() {
        let last = idx == methods.len() - 1;
        let conn_hdr = if last { "Connection: close\r\n" } else { "" };
        // Methods that take a body get a tiny body; the rest carry no body.
        let body: &[u8] = if matches!(*m, "POST" | "PUT" | "DELETE" | "PATCH") {
            b"{}"
        } else {
            b""
        };
        let req_head = format!(
            "{m} /verb-test HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Length: {clen}\r\n{conn_hdr}\r\n",
            clen = body.len(),
        );
        tcp.write_all(req_head.as_bytes()).await.unwrap();
        if !body.is_empty() {
            tcp.write_all(body).await.unwrap();
        }
        tcp.flush().await.unwrap();

        // Read the response head + 0-byte body fully. 204 has no body.
        let mut resp_chunk = vec![0u8; 4096];
        let n = tcp.read(&mut resp_chunk).await.unwrap();
        assert!(n > 0, "no response for method {m}");
        let s = String::from_utf8_lossy(&resp_chunk[..n]);
        assert!(
            s.contains("204 No Content"),
            "method {m}: bad response:\n{s}"
        );
    }
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(20).unwrap();
    assert_eq!(events.len(), methods.len(), "one NetEvent per method");
    let recorded: std::collections::HashSet<&str> =
        events.iter().filter_map(|e| e.method.as_deref()).collect();
    for m in &methods {
        assert!(
            recorded.contains(m),
            "method {m} not recorded; saw {recorded:?}"
        );
    }
    for ev in &events {
        assert_eq!(ev.status_code, Some(204));
        assert_eq!(ev.path.as_deref(), Some("/verb-test"));
        assert_eq!(ev.conn_type.as_deref(), Some("http-mitm"));
    }
}

/// T2.2: query string is split off the path and stored separately
/// in `NetEvent.query`. Multiple parameters, including ones with
/// repeated keys, equals signs, and percent-encoded values, are
/// preserved verbatim. Path side stays clean (no `?` or query).
#[tokio::test]
async fn mitm_proxy_plain_http_records_query_string_with_parameters() {
    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let query = "q=hello%20world&page=2&filter=active&tag=a&tag=b";
    let req = format!(
        "GET /search?{query} HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nConnection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Upstream saw the full request line including the query.
    let recv = received.lock().unwrap().clone();
    let head = String::from_utf8_lossy(&recv);
    assert!(
        head.lines()
            .next()
            .unwrap_or("")
            .starts_with(&format!("GET /search?{query} HTTP/1.1")),
        "upstream did not see the query verbatim. First line:\n{}",
        head.lines().next().unwrap_or("<empty>"),
    );

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(
        ev.path.as_deref(),
        Some("/search"),
        "path must not include the query"
    );
    assert_eq!(
        ev.query.as_deref(),
        Some(query),
        "NetEvent.query must equal the inbound query string verbatim",
    );
}

/// T2.2: arbitrary custom headers (`X-*`) are forwarded to the
/// upstream verbatim. Validates that the proxy's header-passthrough
/// loop in `handle_request` doesn't drop unknown headers.
#[tokio::test]
async fn mitm_proxy_plain_http_forwards_custom_headers_to_upstream() {
    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            bytes
        })
    })
    .await;

    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = format!(
        "GET /headers HTTP/1.1\r\n\
         Host: 127.0.0.1:{upstream_port}\r\n\
         User-Agent: capsem-test/1.0\r\n\
         X-Trace-Id: abc-123-def-456\r\n\
         X-Custom-Flag: enabled\r\n\
         Authorization: Bearer ROTATE_ME_DO_NOT_LEAK\r\n\
         Connection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();

    let recv = received.lock().unwrap().clone();
    let head = String::from_utf8_lossy(&recv).to_ascii_lowercase();
    // Allowlisted user-agent forwarded.
    assert!(
        head.contains("user-agent: capsem-test/1.0"),
        "user-agent dropped:\n{head}"
    );
    // Custom headers (not in the allowlist) still forwarded by name + value.
    assert!(
        head.contains("x-trace-id: abc-123-def-456"),
        "X-Trace-Id dropped:\n{head}"
    );
    assert!(
        head.contains("x-custom-flag: enabled"),
        "X-Custom-Flag dropped:\n{head}"
    );
    // Sensitive Authorization forwarded so upstream auth still works.
    assert!(
        head.contains("authorization: bearer rotate_me_do_not_leak"),
        "Authorization dropped:\n{head}"
    );
    // accept-encoding is rewritten by the proxy to "gzip" (we only
    // accept what we can decompress). Verify that.
    assert!(
        head.contains("accept-encoding: gzip"),
        "accept-encoding should be normalized to 'gzip', got:\n{head}",
    );
}

/// T2.2 + telemetry security: NetEvent.request_headers stores
/// allowlisted headers verbatim but hashes everything else
/// (`hash:<12-char-hex>`). Sensitive headers like `Authorization`,
/// `X-API-Key`, `Cookie` must NEVER appear verbatim in telemetry.
/// Allowlisted ones (User-Agent, Content-Type, Host, ...) DO appear
/// verbatim because they're useful for debugging and don't typically
/// carry secrets.
#[tokio::test]
async fn mitm_proxy_plain_http_telemetry_hashes_non_allowlisted_headers() {
    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let _ = read_http11_request(&mut sock).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let secret = "Bearer SUPER-SECRET-TOKEN-DO-NOT-LEAK";
    let req = format!(
        "GET /headers HTTP/1.1\r\n\
         Host: 127.0.0.1:{upstream_port}\r\n\
         User-Agent: capsem-test/1.0\r\n\
         Authorization: {secret}\r\n\
         X-Api-Key: live_pk_DEADBEEF_DO_NOT_LEAK\r\n\
         Cookie: session=ROTATE_ME_DO_NOT_LEAK\r\n\
         X-Trace-Id: trace-123\r\n\
         Connection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let hdrs = events[0]
        .request_headers
        .as_deref()
        .expect("request_headers should be captured")
        .to_ascii_lowercase();

    // SECURITY: secrets must be redacted to hash:<hex>, never verbatim.
    assert!(
        !hdrs.contains("super-secret-token-do-not-leak"),
        "Authorization secret leaked verbatim into NetEvent.request_headers:\n{hdrs}"
    );
    assert!(
        !hdrs.contains("live_pk_deadbeef"),
        "X-Api-Key secret leaked verbatim:\n{hdrs}"
    );
    assert!(
        !hdrs.contains("session=rotate_me_do_not_leak"),
        "Cookie secret leaked verbatim:\n{hdrs}"
    );

    // Headers must still appear by name, with a hash:<hex> redacted value.
    assert!(
        hdrs.contains("authorization: hash:"),
        "authorization header missing/unhashed:\n{hdrs}"
    );
    assert!(
        hdrs.contains("x-api-key: hash:"),
        "x-api-key header missing/unhashed:\n{hdrs}"
    );
    assert!(
        hdrs.contains("cookie: hash:"),
        "cookie header missing/unhashed:\n{hdrs}"
    );
    assert!(
        hdrs.contains("x-trace-id: hash:"),
        "x-trace-id header missing/unhashed:\n{hdrs}"
    );

    // Allowlisted headers must appear verbatim (Host, User-Agent).
    assert!(
        hdrs.contains("user-agent: capsem-test/1.0"),
        "user-agent missing verbatim from telemetry:\n{hdrs}"
    );
    assert!(
        hdrs.contains(&format!("host: 127.0.0.1:{upstream_port}")),
        "host missing verbatim from telemetry:\n{hdrs}"
    );
}

/// T2.2 risk: a request body larger than the policy's
/// `max_body_capture` (default 4 KB) must be forwarded to the
/// upstream IN FULL but only the first `max_body_capture` bytes
/// land in `request_body_preview`. `bytes_sent` reflects the actual
/// full request body size. Validates we don't accidentally
/// truncate forwarded bytes when capping the telemetry preview.
#[tokio::test]
async fn mitm_proxy_plain_http_body_larger_than_preview_cap_forwards_full_but_caps_preview() {
    // 16 KB of distinguishable bytes -- 4x the default 4 KB
    // max_body_capture. First 4096 bytes are 'A', then 'B', then
    // 'C', then 'D'. Easy to assert preview cap.
    let mut req_body = Vec::with_capacity(16 * 1024);
    for b in [b'A', b'B', b'C', b'D'] {
        req_body.extend(std::iter::repeat_n(b, 4096));
    }
    let req_body_len = req_body.len();
    let req_body_for_serve = req_body.clone();

    let received: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received_for_serve = Arc::clone(&received);

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let bytes = read_http11_request(&mut sock).await;
            *received_for_serve.lock().unwrap() = bytes.clone();
            // Compare just the body portion (after \r\n\r\n).
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            // sanity-echo the expected body length so the test
            // can locate it on failure
            let head_end = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| i + 4)
                .unwrap_or(0);
            assert_eq!(
                bytes[head_end..].len(),
                req_body_for_serve.len(),
                "upstream truncated request body: got {} bytes",
                bytes[head_end..].len(),
            );
            assert_eq!(&bytes[head_end..], req_body_for_serve.as_slice());
            bytes
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req_head = format!(
        "POST /upload HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\n\
         Content-Type: application/octet-stream\r\nContent-Length: {req_body_len}\r\n\
         Connection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(&req_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(
        ev.bytes_sent as usize, req_body_len,
        "bytes_sent should be the full {req_body_len}-byte body",
    );
    if let Some(preview) = &ev.request_body_preview {
        assert!(
            preview.len() <= 4096,
            "preview should cap at default max_body_capture (4096), got {}",
            preview.len(),
        );
        // The preview is the FIRST 4 KB -- all 'A's, no 'B'.
        assert!(preview.starts_with("AAAA"), "preview head wrong");
        assert!(
            !preview.contains('B'),
            "preview should stop before the second 4K block (Bs)",
        );
    }
}

/// T2.2 risk: an IPv6-bracketed `Host` header is currently NOT
/// supported by `parse_http_host_target` (the guest's net_proxy
/// doesn't relay IPv6 anyway). The request must NOT silently
/// succeed -- it gets the `("", 80)` fallback from the parser, the
/// upstream dial against an empty domain fails, and the response
/// is a clean 502 with telemetry. Locks down "we never serve
/// IPv6-shaped Host headers as if they were ipv4".
#[tokio::test]
async fn mitm_proxy_plain_http_ipv6_host_header_does_not_silently_succeed() {
    let (config, db) = make_proxy_config_full(&[""], &[], true, &[80, 11434]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = "GET /v6 HTTP/1.1\r\nHost: [::1]:8080\r\nConnection: close\r\n\r\n";
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        !resp_text.contains("HTTP/1.1 200"),
        "IPv6-bracketed Host MUST NOT yield 200; got:\n{resp_text}"
    );
    // 403 (port-not-in-allowlist on the ("",80) fallback when 80
    // isn't on the list) or 502 (dial fail to "") are both
    // acceptable refusals -- the key invariant is "not 200".
    let is_refusal = resp_text.contains("HTTP/1.1 502") || resp_text.contains("HTTP/1.1 403");
    assert!(is_refusal, "expected 502 or 403 refusal, got:\n{resp_text}");

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert!(matches!(ev.decision, Decision::Error | Decision::Denied));
}

/// T2.2 risk: an upstream that advertises `Content-Encoding: gzip`
/// but sends a corrupted gzip body must not crash the proxy. The
/// `DecompressionHook` either passes the bytes through (its
/// not-gzip / malformed-header branch) or feeds the corrupt body
/// to `flate2::Decompress`, which silently truncates on error.
/// Proxy returns to the client with whatever it could decode and
/// emits a NetEvent. Locks down "garbage gzip = crash" panics.
#[tokio::test]
async fn mitm_proxy_plain_http_corrupted_gzip_response_doesnt_crash() {
    // Plausible-looking gzip header followed by garbage payload --
    // tries to trigger the "gzip-classified, body decode fails"
    // path inside DecompressionHook.
    let bad_gzip = {
        let mut v = Vec::new();
        v.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        v.extend_from_slice(b"this is not deflate-encoded data and never will be 0123456789");
        v
    };
    let bad_gzip_len = bad_gzip.len();
    let bad_gzip_for_serve = bad_gzip.clone();

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let _ = read_http11_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {bad_gzip_len}\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(&bad_gzip_for_serve).await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = format!(
        "GET /badgzip HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nConnection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    // 5s deadline. The "must not hang" half of "must not crash" --
    // a panic in the body wrapper would silently abort the
    // hyper-server task, leaving the client read pending forever.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tcp.read_to_end(&mut resp_buf),
    )
    .await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // The load-bearing assertion is "the proxy still recorded a
    // NetEvent for this request" -- a panic in the chunk path
    // would skip on_response_end and leave net_events empty.
    // Whether the client got any bytes back depends on hyper's
    // internal buffering when every emitted chunk decodes to
    // empty (which is what a fully-corrupt gzip produces); we do
    // not gate on that.
    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        1,
        "corrupted gzip must still emit exactly one NetEvent (no panic on the response path)"
    );
    let ev = &events[0];
    assert_eq!(ev.method.as_deref(), Some("GET"));
    assert_eq!(ev.path.as_deref(), Some("/badgzip"));
    assert_eq!(ev.status_code, Some(200));
    // bytes_received is the post-decompression count. For a
    // fully-corrupt gzip stream, the decoder yields 0 bytes -- the
    // count must therefore be 0, NOT the upstream wire size. If a
    // future change leaks the pre-decode bytes here that's a
    // semantic regression worth catching.
    assert_eq!(
        ev.bytes_received, 0,
        "corrupted gzip should decode to 0 bytes (got {})",
        ev.bytes_received,
    );
}

/// T2.2 risk: upstream advertises `Content-Length: 1000` but only
/// sends 64 bytes then closes the TCP connection. Proxy must not
/// hang -- it should record what it got and emit a NetEvent. This
/// is the "lying upstream" case (truncated response, network
/// drop, server crash mid-stream).
#[tokio::test]
async fn mitm_proxy_plain_http_truncated_upstream_response_doesnt_hang() {
    let partial = b"only-this-much-arrives-before-fin";
    let claimed_len = 1000usize;

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let _ = read_http11_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {claimed_len}\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(partial).await.unwrap();
            sock.flush().await.unwrap();
            // FIN early -- send only `partial.len()` bytes, far short
            // of `claimed_len`.
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = format!(
        "GET /truncated HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nConnection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    // 5s deadline -- if the proxy hangs on the truncated body it
    // won't return inside this window and the test fails.
    let mut resp_buf = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tcp.read_to_end(&mut resp_buf),
    )
    .await;
    drop(tcp);
    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    assert!(read.is_ok(), "proxy hung on truncated upstream body");

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    // bytes_received is whatever made it through. We assert it's
    // strictly less than the lying Content-Length and at most what
    // the upstream actually wrote.
    assert!(
        (ev.bytes_received as usize) <= partial.len(),
        "bytes_received {} exceeds what upstream actually wrote ({})",
        ev.bytes_received,
        partial.len(),
    );
    assert!(
        (ev.bytes_received as usize) < claimed_len,
        "bytes_received {} should not match the lying Content-Length ({})",
        ev.bytes_received,
        claimed_len,
    );
}

/// T2.2 risk: an upstream that returns 200 with a zero-length body
/// (e.g. HEAD-equivalent GET, or an API that signals success only
/// via status). Verifies the chunk hook chain still fires
/// `on_response_end` on an empty body and emits exactly one
/// NetEvent. Edge case for the "what if there are no chunks at
/// all" path.
#[tokio::test]
async fn mitm_proxy_plain_http_zero_length_response_body_emits_netevent() {
    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let _ = read_http11_request(&mut sock).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            Vec::new()
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let req = format!(
        "GET /empty HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nConnection: close\r\n\r\n"
    );
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);
    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        1,
        "zero-body response still emits one NetEvent"
    );
    let ev = &events[0];
    assert_eq!(ev.status_code, Some(200));
    assert_eq!(ev.bytes_received, 0);
    assert_eq!(ev.path.as_deref(), Some("/empty"));
}

/// T2.1: an unrecognized first byte (neither 0x16 nor uppercase ASCII)
/// classifies as `Protocol::Unknown`, never enters TLS or HTTP, and
/// records a connection-level error event whose reason carries the
/// offending byte.
#[tokio::test]
async fn mitm_proxy_classifies_unknown_first_byte() {
    let (config, db) = make_proxy_config(&["elie.net"], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    // 0x01 is neither 0x16 (TLS handshake) nor uppercase ASCII.
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(&[0x01, 0x02, 0x03, 0x04]).await.unwrap();

    let mut buf = vec![0u8; 1024];
    let _ = tcp.read(&mut buf).await;
    drop(tcp);

    proxy_task.await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(
        !events.is_empty(),
        "unknown protocol path must record a NetEvent"
    );
    assert_eq!(events[0].decision, Decision::Error);
    let reason = events[0].matched_rule.as_deref().unwrap_or("");
    assert!(
        reason.starts_with("unknown protocol byte"),
        "expected 'unknown protocol byte' marker, got matched_rule={reason:?}"
    );
}

#[tokio::test]
async fn mitm_proxy_streams_large_payload() {
    let payload_size = 1024 * 1024;
    let large_body = vec![b'A'; payload_size];

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let request = read_http11_request(&mut sock).await;
            let head_end = request
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| i + 4)
                .unwrap_or(0);
            assert_eq!(
                request[head_end..].len(),
                payload_size,
                "upstream should receive the full large request body"
            );
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            request
        })
    })
    .await;

    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req_head = format!(
        "POST /post HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Type: application/octet-stream\r\nContent-Length: {payload_size}\r\nConnection: close\r\n\r\n"
    );
    tcp.write_all(req_head.as_bytes()).await.unwrap();
    tcp.write_all(&large_body).await.unwrap();
    tcp.flush().await.unwrap();
    let mut resp_buf = Vec::new();
    let _ = tcp.read_to_end(&mut resp_buf).await;
    drop(tcp);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();

    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.starts_with("HTTP/1.1 200"),
        "large streaming request failed:\n{resp_text}"
    );

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].method.as_deref(), Some("POST"));
    assert!(
        events[0].bytes_sent >= payload_size as u64,
        "Recorded telemetry bytes_sent {} is smaller than payload size {}",
        events[0].bytes_sent,
        payload_size
    );
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

/// Download a ~10 MB PDF through the MITM proxy and assert throughput >= 1 MB/s.
///
/// Exercises the full proxy pipeline on the host: TLS termination from the
/// "guest" client, upstream TLS to a real CDN, and body streaming back.
/// Uses elie.net directly (not cdn.elie.net) because raw hyper does not
/// follow 301 redirects. Marked #[ignore] so it doesn't run on every
/// `cargo test` -- run explicitly with
/// `cargo test -p capsem-core -- --ignored mitm_proxy_download_throughput`.
#[tokio::test]
#[ignore = "downloads ~10 MB; run explicitly to test proxy throughput"]
async fn mitm_proxy_download_throughput() {
    const DOMAIN: &str = "elie.net";
    const PATH: &str = "/static/files/i-am-a-legend/i-am-a-legend-slides.pdf";
    // Conservative floor; the PDF is ~9.5 MB today but may drift on re-publish.
    const EXPECTED_BYTES: u64 = 9 * 1024 * 1024;
    const MIN_MBPS: f64 = 1.0;

    let (config, _db) = make_proxy_config(&[DOMAIN], &[], false);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let sni = ServerName::try_from(DOMAIN).unwrap();
    let tls = connector
        .connect(sni, tcp)
        .await
        .expect("TLS handshake to elie.net should succeed");

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
        "incomplete download: {:.1} MB (expected >= {:.1} MB)",
        total_bytes as f64 / (1024.0 * 1024.0),
        EXPECTED_BYTES as f64 / (1024.0 * 1024.0),
    );
    assert!(
        mbps >= MIN_MBPS,
        "throughput too low: {mbps:.2} MB/s (minimum {MIN_MBPS} MB/s)"
    );
}
