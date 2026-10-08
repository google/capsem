//! The proxy judges the address it dials, not only the name the guest sent.
//!
//! The rules' local-network guard reads `ip.value`, which used to be set only
//! when the host was an IP literal; the upstream was then dialed by name. A
//! guest asking for any name that resolves to loopback, a private range or
//! the metadata address passed the guard and reached it -- any host service
//! on an allowed port included. Every test here holds the order: resolve,
//! judge the resolved address, dial exactly that address.

use std::time::Duration;

use super::*;
use capsem_core::net::upstream_address::UpstreamResolver;

/// Listener that reports whether the proxy ever dialed it.
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

/// The address half of the built-in `default.000_local_network` guard,
/// without its literal host list: only the resolved address can match.
const LOOPBACK_BY_ADDRESS: &str = r#"
[profiles.rules.loopback_by_address]
name = "loopback_by_address"
action = "block"
reason = "loopback is reached only by explicit allow"
match = 'ip.value.matches("^(127\.|::1$)")'
"#;

/// Send one plain-HTTP request and return the response text, read through
/// its `Content-Length` body: a refused upgrade leaves the connection open.
async fn plain_http(config: Arc<MitmProxyConfig>, request: String) -> String {
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let text = String::from_utf8_lossy(&response).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_string())
                })
                .and_then(|v| v.parse::<usize>().ok());
            if length.is_some_and(|length| body.len() >= length) {
                break;
            }
        }
        match tcp.read(&mut chunk).await.unwrap() {
            0 => break,
            n => response.extend_from_slice(&chunk[..n]),
        }
    }
    drop(tcp);
    proxy_task.await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn a_name_that_resolves_to_loopback_is_judged_by_its_address_and_never_dialed() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_with_security_rules(security_rules_from_toml(LOOPBACK_BY_ADDRESS), &[port]);

    let response = plain_http(
        config,
        format!("GET /admin HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"),
    )
    .await;

    assert!(
        !dialed.await.unwrap(),
        "a name resolving to loopback reached it unjudged"
    );
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].domain, "localhost",
        "the ledger keeps the name the guest asked for"
    );
    assert_eq!(
        events[0].matched_rule.as_deref(),
        Some("profiles.rules.loopback_by_address")
    );
}

/// Serve one request, then report whether a second one arrived on the same
/// upstream connection within a short window.
async fn one_shot_upstream(body: &'static str) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
    spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let mut received = read_http11_request(&mut sock).await;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
            let mut more = [0u8; 4096];
            if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(500), sock.read(&mut more)).await {
                received.extend_from_slice(&more[..n]);
            }
            received
        })
    })
    .await
}

/// A plain-HTTP guest connection can name a different host on every
/// request. The cached upstream sender belonged to the first host; reusing
/// it sent the second request -- judged for its own host -- to the first.
#[tokio::test]
async fn keep_alive_request_for_another_host_does_not_ride_the_first_upstream() {
    let (first_port, first_upstream) = one_shot_upstream("first").await;
    let (second_port, second_upstream) = one_shot_upstream("second").await;
    let (config, _db) = make_proxy_config_full(&[], &[], true, &[first_port, second_port]);
    let (proxy_task, addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("GET /one HTTP/1.1\r\nHost: 127.0.0.1:{first_port}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    while !String::from_utf8_lossy(&response).ends_with("first") {
        let n = tcp.read(&mut chunk).await.unwrap();
        assert!(n > 0, "proxy closed before the first response");
        response.extend_from_slice(&chunk[..n]);
    }
    tcp.write_all(
        format!("GET /two HTTP/1.1\r\nHost: 127.0.0.1:{second_port}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut rest = Vec::new();
    let _ = tcp.read_to_end(&mut rest).await;
    drop(tcp);
    proxy_task.await.unwrap();

    let first = String::from_utf8_lossy(&first_upstream.await.unwrap()).into_owned();
    let second = String::from_utf8_lossy(&second_upstream.await.unwrap()).into_owned();
    assert!(first.contains("GET /one "), "{first}");
    assert!(
        !first.contains("GET /two "),
        "the second host's request reached the first upstream:\n{first}"
    );
    assert!(
        second.contains("GET /two "),
        "the second host never got its request:\n{second}"
    );
    assert!(String::from_utf8_lossy(&rest).ends_with("second"));
}

const DEFAULT_RULES: &str = include_str!("../../../capsem-config/src/default_provider_rules.toml");
const LOCAL_NETWORK: &str = "profiles.rules.default_000_local_network";

fn default_rules() -> capsem_core::net::policy_config::SecurityRuleSet {
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(DEFAULT_RULES).unwrap();
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::BuiltinDefault,
    )
    .unwrap()
}

/// Point `name` at `address` the way a DNS record would, without DNS.
fn resolving(config: Arc<MitmProxyConfig>, name: &str, address: &str) -> Arc<MitmProxyConfig> {
    let mut config = Arc::try_unwrap(config).ok().expect("config not yet shared");
    config.upstream_resolver = UpstreamResolver::system().with_fixed_answer(name, vec![address.parse().unwrap()]);
    config.upstream_grants = Some(Arc::new(IntegrationGrants::new(
        Arc::clone(&config.policy),
        config.upstream_resolver.clone(),
    )));
    Arc::new(config)
}

/// The built-in guard, not a test rule: a name that is on no list but
/// resolves to loopback, where the host's own services listen, is asked.
#[tokio::test]
async fn default_rules_ask_before_a_name_that_resolves_to_loopback_and_never_dial_it() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_with_security_rules(default_rules(), &[port]);
    let config = resolving(config, "gateway.attacker.example", "127.0.0.1");

    let response = plain_http(
        config,
        format!("GET /v1/vms HTTP/1.1\r\nHost: gateway.attacker.example:{port}\r\nConnection: close\r\n\r\n"),
    )
    .await;

    assert!(!dialed.await.unwrap(), "the default guard let a loopback name through");
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    assert!(response.contains("requires approval"), "{response}");
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].domain, "gateway.attacker.example");
    assert_eq!(events[0].matched_rule.as_deref(), Some(LOCAL_NETWORK));
    assert_eq!(events[0].policy_action.as_deref(), Some("ask"));
}

/// HTTPS takes its name from SNI and always dials 443; the same guard
/// applies, e.g. to a name aimed at the cloud metadata address.
#[tokio::test]
async fn default_rules_ask_before_a_tls_name_that_resolves_to_the_metadata_address() {
    let (config, db) = make_proxy_config_with_security_rules(default_rules(), &[80]);
    let config = resolving(config, "metadata.attacker.example", "169.254.169.254");
    let (proxy_task, addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let tls = connector
        .connect(ServerName::try_from("metadata.attacker.example").unwrap(), tcp)
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let request = hyper::Request::builder()
        .uri("/computeMetadata/v1/")
        .header("host", "metadata.attacker.example")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 403);
    drop(sender);
    proxy_task.await.unwrap();

    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events[0].domain, "metadata.attacker.example");
    assert_eq!(events[0].matched_rule.as_deref(), Some(LOCAL_NETWORK));
    assert_eq!(events[0].policy_action.as_deref(), Some("ask"));
}

/// The dial goes to the address the rules judged. `judged.invalid` never
/// resolves in DNS, so reaching the upstream at all proves the proxy dialed
/// the evaluated address and not the name.
#[tokio::test]
async fn the_dial_goes_to_the_judged_address_not_the_name() {
    let (port, upstream) = one_shot_upstream("judged").await;
    let (config, db) = make_proxy_config_full(&[], &[], true, &[port]);
    let config = resolving(config, "judged.invalid", "127.0.0.1");

    let response = plain_http(
        config,
        format!("GET /where HTTP/1.1\r\nHost: judged.invalid:{port}\r\nConnection: close\r\n\r\n"),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200"), "expected 200, got:\n{response}");
    assert!(response.ends_with("judged"));
    let received = String::from_utf8_lossy(&upstream.await.unwrap()).into_owned();
    assert!(received.contains("GET /where "), "{received}");
    assert!(
        received.contains(&format!("host: judged.invalid:{port}")),
        "the Host header is the guest's"
    );
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events[0].decision, Decision::Allowed);
}

/// A WebSocket upgrade is judged by the same resolved address.
#[tokio::test]
async fn a_websocket_upgrade_to_a_loopback_name_is_judged_by_its_address() {
    let (port, dialed) = dial_probe().await;
    let (config, db) = make_proxy_config_with_security_rules(default_rules(), &[port]);
    let config = resolving(config, "ws.attacker.example", "127.0.0.1");

    let response = plain_http(
        config,
        format!(
            "GET /socket HTTP/1.1\r\nHost: ws.attacker.example:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ),
    )
    .await;

    assert!(!dialed.await.unwrap(), "the upgrade reached a loopback name unjudged");
    assert!(response.starts_with("HTTP/1.1 403"), "expected 403, got:\n{response}");
    db.flush().await;
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events[0].matched_rule.as_deref(), Some(LOCAL_NETWORK));
}
