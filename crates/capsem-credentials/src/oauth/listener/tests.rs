use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::*;

fn policy() -> OAuthListenerPolicy {
    OAuthListenerPolicy {
        transaction: OAuthPolicy {
            lifetime: Duration::from_secs(5),
            max_callback_bytes: 1024,
        },
        request_timeout: Duration::from_millis(200),
        max_connections: 2,
        max_header_bytes: 8192,
        max_headers: 16,
    }
}

async fn bound(ip: &str) -> (OAuthListener, SocketAddr, String) {
    let mut owner = OAuthListener::bind(ip.parse().unwrap(), "/google/callback", policy())
        .await
        .unwrap();
    let address = owner.address();
    let state = owner.authorization().unwrap().state.to_string();
    (owner, address, state)
}

fn request(address: SocketAddr, state: &str, tail: &str) -> String {
    format!("GET /google/callback?state={state}&{tail} HTTP/1.1\r\nHost: {address}\r\n\r\n")
}

async fn send(address: SocketAddr, request: &str) -> String {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_ipv4_and_ipv6_callbacks_return_exchange_and_retire_the_listener() {
    for ip in ["127.0.0.1", "::1"] {
        let (owner, address, state) = bound(ip).await;
        let task = tokio::spawn(owner.wait());
        let response = send(address, &request(address, &state, "code=private-code")).await;
        assert!(response.starts_with("HTTP/1.1 200"));
        let lower = response.to_ascii_lowercase();
        for header in [
            "cache-control: no-store",
            "referrer-policy: no-referrer",
            "content-security-policy: default-src 'none'",
            "x-content-type-options: nosniff",
            "connection: close",
        ] {
            assert!(lower.contains(header), "missing {header}");
        }
        assert!(!response.contains(&state) && !response.contains("private-code"));
        assert!(!lower.contains("access-control-allow-origin"));
        let exchange = task.await.unwrap().unwrap();
        assert_eq!(exchange.code(), "private-code");
        assert_eq!(exchange.redirect_uri(), format!("http://{address}/google/callback"));
        assert_eq!(exchange.verifier().len(), 43);
        assert!(TcpStream::connect(address).await.is_err());
    }
}

#[tokio::test]
async fn hostile_requests_refuse_without_consuming_the_legitimate_pending_callback() {
    let (owner, address, state) = bound("127.0.0.1").await;
    let task = tokio::spawn(owner.wait());
    let valid = request(address, &state, "code=private-code");
    let cases = [
        valid.replace(&format!("Host: {address}"), "Host: attacker.example"),
        valid.replace("GET ", "POST "),
        valid.replace("/google/callback?", "/wrong-path?"),
        valid.replace("GET /", &format!("GET http://{address}/")),
        valid.replace("\r\n\r\n", "\r\nOrigin: https://attacker.example\r\n\r\n"),
        valid.replace("\r\n\r\n", "\r\nContent-Length: 1\r\n\r\nx"),
        valid.replace("\r\n\r\n", "\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"),
        valid.replace("\r\n\r\n", "\r\nUpgrade: websocket\r\n\r\n"),
        valid.replace("\r\n\r\n", &format!("\r\nHost: {address}\r\n\r\n")),
        request(address, "wrong-state", "code=private-code"),
        request(address, &state, "code=private-code&code=second"),
        request(address, &state, "code=private-code&%73tate=second"),
        request(address, &state, &format!("code={}", "x".repeat(1500))),
    ];
    for input in cases {
        let response = send(address, &input).await;
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(!response.contains(&state) && !response.contains("private-code"));
        assert!(!task.is_finished());
    }
    assert!(send(address, &valid).await.starts_with("HTTP/1.1 200"));
    assert_eq!(task.await.unwrap().unwrap().code(), "private-code");
}

#[tokio::test]
async fn slow_partial_client_cannot_hold_the_only_connection_slot_past_its_budget() {
    let (mut owner, address, state) = bound("127.0.0.1").await;
    owner.policy.max_connections = 1;
    let accepted = owner.accepted.clone();
    let task = tokio::spawn(owner.wait());
    let mut slow = TcpStream::connect(address).await.unwrap();
    slow.write_all(b"GET /google/callback HTTP/1.1\r\nHost:").await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted.notified())
        .await
        .unwrap();
    assert!(send(address, &request(address, &state, "code=legitimate"))
        .await
        .starts_with("HTTP/1.1 200"));
    assert_eq!(task.await.unwrap().unwrap().code(), "legitimate");
    let mut byte = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(1), slow.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
}

#[tokio::test]
async fn expiry_cancellation_and_drop_close_listener_and_accepted_connections() {
    let ip: IpAddr = "127.0.0.1".parse().unwrap();
    let mut expiry = policy();
    expiry.transaction.lifetime = Duration::from_millis(40);
    let owner = OAuthListener::bind(ip, "/google/callback", expiry).await.unwrap();
    let address = owner.address();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), owner.wait())
            .await
            .unwrap(),
        Err(OAuthListenerError::OAuth(OAuthError::Inactive(OAuthState::Expired)))
    ));
    assert!(TcpStream::connect(address).await.is_err());

    let (owner, address, _) = bound("127.0.0.1").await;
    let accepted = owner.accepted.clone();
    let task = tokio::spawn(owner.wait());
    let mut peer = TcpStream::connect(address).await.unwrap();
    peer.write_all(b"GET /").await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(TcpStream::connect(address).await.is_err());
    let mut byte = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);

    let (owner, address, _) = bound("127.0.0.1").await;
    drop(owner);
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn denied_consent_returns_no_exchange_and_no_secret_browser_echo() {
    let (owner, address, state) = bound("127.0.0.1").await;
    let task = tokio::spawn(owner.wait());
    let response = send(
        address,
        &request(address, &state, "error=access_denied&error_description=private-details"),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(!response.contains(&state) && !response.contains("private-details"));
    assert!(matches!(
        task.await.unwrap(),
        Err(OAuthListenerError::OAuth(OAuthError::Denied))
    ));
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn unsupported_bind_or_unbounded_policy_refuses_before_creating_an_owner() {
    assert!(matches!(
        OAuthListener::bind("0.0.0.0".parse().unwrap(), "/google/callback", policy()).await,
        Err(OAuthListenerError::Transport)
    ));
    for invalid in [
        OAuthListenerPolicy {
            request_timeout: Duration::ZERO,
            ..policy()
        },
        OAuthListenerPolicy {
            request_timeout: Duration::MAX,
            ..policy()
        },
        OAuthListenerPolicy {
            max_connections: 0,
            ..policy()
        },
        OAuthListenerPolicy {
            max_headers: 0,
            ..policy()
        },
        OAuthListenerPolicy {
            max_header_bytes: 1024,
            ..policy()
        },
    ] {
        assert!(matches!(
            OAuthListener::bind("127.0.0.1".parse().unwrap(), "/google/callback", invalid).await,
            Err(OAuthListenerError::OAuth(OAuthError::InvalidPolicy))
        ));
    }
}

#[tokio::test]
async fn header_count_and_byte_limits_refuse_without_echo_or_nonce_consumption() {
    let (owner, address, state) = bound("127.0.0.1").await;
    let task = tokio::spawn(owner.wait());
    let valid = request(address, &state, "code=private-code");
    let many = "X-Extra: value\r\n".repeat(32);
    for headers in [many, format!("X-Extra: {}\r\n", "x".repeat(9500))] {
        let raw = valid.replace("\r\n\r\n", &format!("\r\n{headers}\r\n"));
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
            .await
            .unwrap();
        if let Err(error) = result {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
        }
        if !response.is_empty() {
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 431"));
            assert!(!response.contains(&state) && !response.contains("private-code"));
        }
        assert!(!task.is_finished());
    }
    assert!(send(address, &valid).await.starts_with("HTTP/1.1 200"));
    assert_eq!(task.await.unwrap().unwrap().code(), "private-code");
}

#[tokio::test]
async fn browser_disconnect_does_not_lose_a_matched_exchange() {
    let (owner, address, state) = bound("127.0.0.1").await;
    let task = tokio::spawn(owner.wait());
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(request(address, &state, "code=browser-disconnected").as_bytes())
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    drop(stream);
    let exchange = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(exchange.code(), "browser-disconnected");
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn two_accepted_callbacks_cannot_both_consume_the_same_nonce() {
    let (owner, address, state) = bound("127.0.0.1").await;
    let accepted = owner.accepted.clone();
    let task = tokio::spawn(owner.wait());
    let mut first = TcpStream::connect(address).await.unwrap();
    first
        .write_all(format!("GET /google/callback?state={state}&code=first HTTP/1.1\r\n").as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted.notified())
        .await
        .unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    second
        .write_all(format!("GET /google/callback?state={state}&code=second HTTP/1.1\r\n").as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted.notified())
        .await
        .unwrap();
    let headers = format!("Host: {address}\r\n\r\n");
    let (one, two) = tokio::join!(
        first.write_all(headers.as_bytes()),
        second.write_all(headers.as_bytes())
    );
    one.unwrap();
    two.unwrap();
    let mut first_response = Vec::new();
    let mut second_response = Vec::new();
    let (one, two) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            first.read_to_end(&mut first_response),
            second.read_to_end(&mut second_response)
        )
    })
    .await
    .unwrap();
    for result in [one, two] {
        if let Err(error) = result {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
        }
    }
    let won_first = first_response.starts_with(b"HTTP/1.1 200");
    let won_second = second_response.starts_with(b"HTTP/1.1 200");
    assert_ne!(
        won_first, won_second,
        "exactly one browser received a successful callback response"
    );
    let exchange = task.await.unwrap().unwrap();
    assert_eq!(exchange.code(), if won_first { "first" } else { "second" });
    assert!(TcpStream::connect(address).await.is_err());
}
