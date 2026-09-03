//! Host identity is normalized once at the proxy boundary.
//!
//! `pastebin.com.` and `PASTEBIN.COM` name the same host as `pastebin.com`.
//! Policy used to see the verbatim spelling while the dial went to whatever
//! the resolver made of it, so a block rule on `pastebin.com` was evaded by
//! a trailing dot. Every path -- plain Host header and TLS SNI -- must
//! evaluate, dial, and record the same normalized name.

use std::time::Duration;

use super::*;

async fn dial_probe() -> (u16, tokio::task::JoinHandle<bool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await
            .is_ok()
    });
    (port, task)
}

/// One plain-HTTP GET with the given Host header; returns the raw response,
/// whether the probe listener was dialed, and the recorded net events.
async fn plain_http_get(
    config: Arc<MitmProxyConfig>,
    db: Arc<DbWriter>,
    host_header: &str,
    dialed: tokio::task::JoinHandle<bool>,
) -> (String, bool, Vec<capsem_logger::NetEvent>) {
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("GET /paste HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tcp.read_to_end(&mut buf).await;
    drop(tcp);
    let dialed = dialed.await.unwrap();
    proxy_task.await.unwrap();
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    (String::from_utf8_lossy(&buf).into_owned(), dialed, events)
}

#[tokio::test]
async fn plain_http_block_rule_fires_for_trailing_dot_ip_host() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_full(&[], &["127.0.0.1"], true, &[80, port]);

    let (response, dialed, events) = plain_http_get(config, db, &format!("127.0.0.1.:{port}"), dialed).await;

    assert!(
        !dialed,
        "a blocked host must not be dialed under a trailing-dot spelling"
    );
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].domain, "127.0.0.1", "telemetry records the normalized host");
    assert_eq!(events[0].port, port);
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("profiles.rules.block_test_hosts")
    );
}

/// Case and trailing dot together, on a name rather than an address: the
/// spelling a guest would actually try against a `pastebin.com` rule.
#[tokio::test]
async fn plain_http_block_rule_fires_for_uppercase_trailing_dot_name() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_full(&[], &["localhost"], true, &[80, port]);

    let (response, dialed, events) = plain_http_get(config, db, &format!("LOCALHOST.:{port}"), dialed).await;

    assert!(!dialed, "LOCALHOST. must be refused as localhost");
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].domain, "localhost");
}

/// Several trailing dots are still the same host. Mirrors the DNS parser,
/// which strips every trailing root dot from a qname.
#[tokio::test]
async fn plain_http_block_rule_fires_for_repeated_trailing_dots() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_full(&[], &["localhost"], true, &[80, port]);

    let (response, dialed, events) = plain_http_get(config, db, &format!("localhost..:{port}"), dialed).await;

    assert!(!dialed);
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert_eq!(events[0].domain, "localhost");
}

/// A host that is nothing but dots normalizes to nothing and must not be
/// treated as a reachable upstream.
#[tokio::test]
async fn plain_http_dot_only_host_is_not_dialed() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_full(&[], &[], true, &[80, port]);

    let (response, dialed, _events) = plain_http_get(config, db, &format!("..:{port}"), dialed).await;

    assert!(!dialed, "an empty host cannot name an upstream");
    assert!(
        !response.starts_with("HTTP/1.1 2"),
        "expected a refusal, got:\n{response}"
    );
}

/// The allowed path dials the normalized name: `127.0.0.1.` is not an IP
/// literal and would never connect, but the guest asked for `127.0.0.1`.
#[tokio::test]
async fn plain_http_allowed_trailing_dot_host_dials_normalized_name() {
    let (port, upstream) = spawn_fake_upstream(|mut sock| {
        Box::pin(async move {
            let request = read_http11_request(&mut sock).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            let _ = sock.shutdown().await;
            request
        })
    })
    .await;
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1.:{port}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tcp.read_to_end(&mut buf).await;
    drop(tcp);
    upstream.await.unwrap();
    proxy_task.await.unwrap();
    db.flush().await;

    let response = String::from_utf8_lossy(&buf);
    assert!(response.starts_with("HTTP/1.1 200"), "expected 200, got:\n{response}");
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events[0].decision, Decision::Allowed);
    assert_eq!(events[0].domain, "127.0.0.1");
    assert_eq!(events[0].port, port);
}

/// TLS path: the SNI a rustls client sends keeps its case, and the proxy
/// must evaluate, mint, and record the lowercase name. (rustls clients strip
/// a trailing SNI dot themselves; the raw-ClientHello case is covered at the
/// resolver in `cert_authority::tests`.)
#[tokio::test]
async fn tls_sni_mixed_case_is_normalized_for_policy_and_telemetry() {
    // Default-allow: without normalization the request would be allowed and
    // dialed, so a 403 from the host rule is the whole proof.
    let (config, db) = make_proxy_config(&[], &["example.com"], true);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let tls = connector
        .connect(ServerName::try_from("EXAMPLE.COM").unwrap(), tcp)
        .await
        .expect("leaf cert for the normalized name must verify against the mixed-case SNI");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let req = hyper::Request::builder()
        .method("GET")
        .uri("/paste")
        .header("host", "EXAMPLE.COM.")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    drop(sender);
    proxy_task.await.unwrap();
    db.flush().await;

    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].domain, "example.com",
        "telemetry domain is the normalized SNI"
    );
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("profiles.rules.block_test_hosts")
    );
}
