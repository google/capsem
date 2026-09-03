//! What one plain-HTTP request costs through the MITM proxy.
//!
//! A guest `curl` opens a TCP connection to the proxy, sends one request and
//! reads the answer. This measures that round trip against a loopback
//! upstream, next to the same request sent to the upstream directly, so the
//! proxy's own overhead -- classification, policy, dial, telemetry -- is a
//! number rather than a guess.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::os::unix::io::IntoRawFd;
use std::sync::Arc;

use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy::{self, MitmProxyConfig};
use capsem_core::net::policy::NetworkMechanics;
use capsem_logger::DbWriter;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CA_KEY: &str = include_str!("../resources/ca/capsem-ca.key");
const CA_CERT: &str = include_str!("../resources/ca/capsem-ca.crt");

/// A loopback HTTP/1.1 upstream that answers every request with `ok`.
async fn spawn_upstream() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    head.extend_from_slice(&buf[..n]);
                }
                let close = head.windows(17).any(|w| w.eq_ignore_ascii_case(b"connection: close"));
                let reply: &[u8] = if close {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
                };
                if sock.write_all(reply).await.is_err() || close {
                    let _ = sock.shutdown().await;
                    return;
                }
                head.clear();
                loop {
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        head.extend_from_slice(&buf[..n]);
                    }
                    if sock.write_all(reply).await.is_err() {
                        return;
                    }
                    head.clear();
                }
            });
        }
    });
    addr
}

fn proxy_config(upstream_port: u16) -> Arc<MitmProxyConfig> {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let mut mechanics = NetworkMechanics::new();
    mechanics.http_upstream_ports = vec![80, upstream_port];
    let policy = Arc::new(std::sync::RwLock::new(Arc::new(mechanics)));
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("bench.db"), 256).unwrap());
    std::mem::forget(dir);
    let security_rules = capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new());
    let telemetry = Arc::new(mitm_proxy::telemetry_hook::TelemetryDeps {
        db: db.clone(),
        pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::new(std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(security_rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let pipeline = mitm_proxy::make_production_pipeline(Arc::clone(&policy), Arc::clone(&telemetry));
    Arc::new(MitmProxyConfig {
        ca,
        policy,
        model_endpoints: Arc::new(std::sync::RwLock::new(Arc::new(
            capsem_core::net::policy_config::ProviderRuleProfile::builtin_defaults()
                .endpoint_registry()
                .expect("builtin provider endpoint registry"),
        ))),
        db,
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
        telemetry,
        pipeline,
        mcp_endpoint: None,
    })
}

/// The proxy as the guest sees it: every accepted connection is one
/// `handle_connection`, exactly as the vsock listener runs it.
async fn spawn_proxy(config: Arc<MitmProxyConfig>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                let fd = stream.into_std().unwrap().into_raw_fd();
                mitm_proxy::handle_connection(fd, config).await;
                // handle_connection wraps the fd in ManuallyDrop; we own it.
                unsafe { libc::close(fd) };
            });
        }
    });
    addr
}

async fn get_once(addr: SocketAddr, host_header: &str) -> Vec<u8> {
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("GET /bench HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = Vec::new();
    tcp.read_to_end(&mut out).await.unwrap();
    out
}

/// One connection, `n` requests: what a request costs once the connection
/// setup on both sides is paid, which is how SDK clients talk.
async fn get_keepalive(addr: SocketAddr, host_header: &str, n: usize) {
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!("GET /bench HTTP/1.1\r\nHost: {host_header}\r\n\r\n");
    let mut buf = vec![0u8; 4096];
    for _ in 0..n {
        tcp.write_all(request.as_bytes()).await.unwrap();
        let mut got = Vec::new();
        loop {
            let read = tcp.read(&mut buf).await.unwrap();
            assert!(read > 0, "upstream closed mid-request");
            got.extend_from_slice(&buf[..read]);
            if got.ends_with(b"ok") {
                break;
            }
        }
    }
}

fn mitm_request(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let upstream = rt.block_on(spawn_upstream());
    let proxy = rt.block_on(spawn_proxy(proxy_config(upstream.port())));
    let host = format!("127.0.0.1:{}", upstream.port());

    let direct = rt.block_on(get_once(upstream, &host));
    assert!(direct.ends_with(b"ok"), "upstream answers directly");
    let proxied = rt.block_on(get_once(proxy, &host));
    assert!(
        proxied.ends_with(b"ok"),
        "proxy relays the answer: {:?}",
        String::from_utf8_lossy(&proxied)
    );

    c.bench_function("plain_http_get_direct", |b| {
        b.iter(|| rt.block_on(get_once(upstream, &host)));
    });
    c.bench_function("plain_http_get_through_mitm", |b| {
        b.iter(|| rt.block_on(get_once(proxy, &host)));
    });
    c.bench_function("plain_http_get_keepalive_x100_through_mitm", |b| {
        b.iter(|| rt.block_on(get_keepalive(proxy, &host, 100)));
    });
}

criterion_group!(benches, mitm_request);
criterion_main!(benches);
