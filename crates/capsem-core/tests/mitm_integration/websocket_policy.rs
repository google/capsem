//! WebSocket upgrades ride the same policy rail as every other request.
//!
//! The upgrade branch used to dial `{domain}:{port}` before the security
//! boundary and the plain-HTTP port allowlist ran, so a guest could reach
//! any host (the gateway on `127.0.0.1:19222` included) by adding
//! `Upgrade: websocket` to an otherwise blocked request, and the tunnel was
//! then logged as allowed by `security.http.default`. Every test here holds
//! the order: evaluate first, dial second, and record what was enforced.

use std::time::Duration;

use super::*;

/// Listener that reports whether the proxy ever dialed it. A dial would
/// arrive within milliseconds of the request, so one second is decisive.
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

fn upgrade_request(host: &str, upgrade: &str, connection: &str) -> String {
    format!(
        "GET /socket HTTP/1.1\r\nHost: {host}\r\nUpgrade: {upgrade}\r\nConnection: {connection}\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
}

/// Read until the end of the response head. The 403 body may or may not
/// share the read; the caller drops the socket afterwards either way.
async fn read_response_head(tcp: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = tcp.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Send one plain-HTTP upgrade request at a probe listener and return the
/// response head, whether the probe was dialed, and the recorded net events.
async fn plain_http_upgrade(
    config: Arc<MitmProxyConfig>,
    db: Arc<DbWriter>,
    port: u16,
    dialed: tokio::task::JoinHandle<bool>,
    upgrade: &str,
    connection: &str,
) -> (String, bool, Vec<capsem_logger::NetEvent>) {
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(upgrade_request(&format!("127.0.0.1:{port}"), upgrade, connection).as_bytes())
        .await
        .unwrap();
    let response = read_response_head(&mut tcp).await;
    drop(tcp);
    let dialed = dialed.await.unwrap();
    proxy_task.await.unwrap();
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    (response, dialed, events)
}

#[tokio::test]
async fn websocket_upgrade_blocked_by_rule_returns_403_and_never_dials() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_full(&[], &["127.0.0.1"], true, &[80, port]);

    let (response, dialed, events) = plain_http_upgrade(config, db, port, dialed, "websocket", "Upgrade").await;

    assert!(!dialed, "a blocked upgrade must never reach the upstream");
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert_eq!(events.len(), 1, "one NetEvent per request");
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].domain, "127.0.0.1");
    assert_eq!(events[0].port, port);
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("profiles.rules.block_test_hosts"),
        "the refusal names the rule that made it"
    );
    assert_eq!(events[0].policy_action.as_deref(), Some("block"));
}

#[tokio::test]
async fn websocket_upgrade_to_port_outside_allowlist_is_refused_before_dialing() {
    let (port, dialed) = dial_probe().await;
    // Host allowed, port not: only the mechanics allowlist stands in the way.
    let (config, db) = make_proxy_config_full(&[], &[], true, &[80]);

    let (response, dialed, events) = plain_http_upgrade(config, db, port, dialed, "websocket", "Upgrade").await;

    assert!(!dialed, "the host must not dial a port outside the allowlist");
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].port, port);
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("security.web.http_upstream_ports")
    );
}

/// Attackers do not spell headers the way the RFC example does. Every
/// case and token-list variant hyper recognises as an upgrade must be gated.
#[tokio::test]
async fn websocket_upgrade_header_spelling_variants_are_all_gated() {
    for (upgrade, connection) in [
        ("WebSocket", "Upgrade"),
        ("websocket", "keep-alive, Upgrade"),
        ("WEBSOCKET", "upgrade"),
    ] {
        let (port, dialed) = dial_probe().await;
        let (config, db) = make_proxy_config_full(&[], &["127.0.0.1"], true, &[80, port]);

        let (response, dialed, events) = plain_http_upgrade(config, db, port, dialed, upgrade, connection).await;

        assert!(!dialed, "variant {upgrade:?}/{connection:?} reached the upstream");
        assert!(
            response.starts_with("HTTP/1.1 403"),
            "variant {upgrade:?}/{connection:?} expected 403, got:\n{response}"
        );
        assert_eq!(
            events[0].decision,
            Decision::Denied,
            "variant {upgrade:?}/{connection:?}"
        );
    }
}

/// The gate must not break the feature: an allowed upgrade still tunnels
/// bytes both ways, and its telemetry now carries the real enforcement
/// decision instead of the `security.http.default` placeholder.
#[tokio::test]
async fn websocket_upgrade_allowed_still_tunnels_and_records_enforcement() {
    let (port, upstream) = spawn_fake_upstream(|mut sock| {
        Box::pin(async move {
            let request = read_http11_request(&mut sock).await;
            sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
            let mut chunk = [0u8; 64];
            loop {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                sock.write_all(&chunk[..n]).await.unwrap();
            }
            request
        })
    })
    .await;
    let (config, db) = make_proxy_config_full(&[], &[], true, &[80, port]);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(upgrade_request(&format!("127.0.0.1:{port}"), "websocket", "Upgrade").as_bytes())
        .await
        .unwrap();
    let response = read_response_head(&mut tcp).await;
    assert!(response.starts_with("HTTP/1.1 101"), "expected 101, got:\n{response}");

    tcp.write_all(b"ping").await.unwrap();
    let mut echo = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), tcp.read_exact(&mut echo))
        .await
        .expect("tunnel must relay bytes")
        .unwrap();
    assert_eq!(&echo, b"ping");
    drop(tcp);

    // hyper lowercases header names on the wire; the value is what matters.
    let upstream_request = String::from_utf8_lossy(&upstream.await.unwrap()).to_ascii_lowercase();
    assert!(upstream_request.contains("upgrade: websocket"), "{upstream_request}");
    proxy_task.await.unwrap();
    db.flush().await;

    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Allowed);
    assert_eq!(events[0].status_code, Some(101));
    assert_eq!(events[0].port, port);
    assert_eq!(events[0].policy_mode.as_deref(), Some("enforce"));
    assert_eq!(events[0].policy_action.as_deref(), Some("allow"));
}

/// Same gate on the TLS path: a blocked SNI domain cannot be tunnelled by
/// asking for an upgrade. Without the gate the branch dials the real
/// `{domain}:443` (here an unresolvable fixture name, so a 502 instead).
#[tokio::test]
async fn websocket_upgrade_over_tls_blocked_by_rule_returns_403() {
    // Default-allow so the host rule is the only block rule: the matched rule
    // id then proves the upgrade was judged on the SNI name.
    let (config, db) = make_proxy_config(&[], &[HERMETIC_UPSTREAM_DOMAIN], true);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let tls = connector
        .connect(ServerName::try_from(HERMETIC_UPSTREAM_DOMAIN).unwrap(), tcp)
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn.with_upgrades());

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/socket")
        .header("host", HERMETIC_UPSTREAM_DOMAIN)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-version", "13")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403, "blocked domain must refuse the upgrade");
    drop(sender);
    proxy_task.await.unwrap();
    db.flush().await;

    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].domain, HERMETIC_UPSTREAM_DOMAIN);
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("profiles.rules.block_test_hosts")
    );
}
