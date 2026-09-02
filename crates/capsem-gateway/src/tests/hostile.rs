//! Attacker-shaped requests against the loopback boundary and the token check.

use super::*;

/// Every spelling a rebinding page, a proxy, or a confused client might put in
/// `Host` that is not this machine. Each must be refused on every route.
#[tokio::test]
async fn hostile_host_spellings_are_all_refused() {
    let hosts = [
        "localhost.evil.example",
        "localhost.",
        "evil.example:19222",
        "127.0.0.1.nip.io",
        "127.0.0.2",
        "127.1",
        "0x7f000001",
        "2130706433",
        "[::ffff:127.0.0.1]",
        "[::ffff:7f00:1]",
        "[::2]",
        "0.0.0.0",
        "10.0.0.1:19222",
        "localhost:19222:19222",
        "user@localhost",
        "user@localhost:19222",
        "localhost/",
        " localhost",
        "localhost\t",
        "",
        "LOCALHOST.EVIL.EXAMPLE",
        "xn--localhost",
        "localhost\u{3002}evil",
    ];
    for host in hosts {
        let (app, _) = guarded_app();
        let resp = get_with_host(app, "/token", Some(host)).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::FORBIDDEN,
            "host {host:?} must not be treated as this machine"
        );
    }
}

#[tokio::test]
async fn a_second_host_header_does_not_smuggle_a_foreign_host() {
    let (app, _) = guarded_app();
    let mut req = http::Request::builder()
        .uri("/token")
        .header(http::header::HOST, "localhost")
        .header(http::header::HOST, "evil.example")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    // The first value wins for hyper; a foreign value anywhere is still a
    // request that never came from a real browser on this machine, so either
    // outcome is acceptable except leaking the token to a foreign first value.
    assert!(
        resp.status() == http::StatusCode::OK || resp.status() == http::StatusCode::FORBIDDEN,
        "{}",
        resp.status()
    );
}

#[tokio::test]
async fn a_foreign_host_on_an_authenticated_route_is_refused_before_auth() {
    // A correct bearer never rescues a foreign Host: the refusal is a property
    // of the connection's origin, not of the caller's credentials.
    let (_, state) = token_app();
    let app = axum::Router::new()
        .route("/status", axum::routing::get(handle_health))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .with_state(state.clone());
    let mut req = http::Request::builder()
        .uri("/status")
        .header(http::header::HOST, "evil.example")
        .header("authorization", format!("Bearer {}", state.token))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

/// Query-string token shapes that must not authenticate a WebSocket route.
#[tokio::test]
async fn hostile_query_tokens_do_not_authenticate() {
    let (_, state) = token_app();
    let token = state.token.clone();
    let queries = [
        format!("xtoken={token}"),
        format!("token={}", &token[..token.len() - 1]),
        format!("token={token}x"),
        format!("token=%20{token}"),
        format!("Token={token}"),
        format!("token={}", token.to_uppercase()),
        "token=".to_string(),
        format!("other=1&tok=en&t={token}"),
    ];
    for query in queries {
        let app = axum::Router::new()
            .route("/events", axum::routing::get(handle_health))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::auth_middleware,
            ))
            .with_state(state.clone());
        let mut req = http::Request::builder()
            .uri(format!("/events?{query}"))
            .header(http::header::HOST, "localhost")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            http::StatusCode::UNAUTHORIZED,
            "query {query:?} must not authenticate"
        );
    }
}

#[tokio::test]
async fn query_token_is_only_honoured_on_websocket_routes() {
    let (_, state) = token_app();
    let app = axum::Router::new()
        .route("/status", axum::routing::get(handle_health))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .with_state(state.clone());
    let mut req = http::Request::builder()
        .uri(format!("/status?token={}", state.token))
        .header(http::header::HOST, "localhost")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[test]
fn token_comparison_rejects_every_near_miss() {
    let token = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZab";
    assert!(auth::token_matches(token, token));
    let near_misses = [
        token.to_lowercase(),
        format!("{token} "),
        format!(" {token}"),
        token[1..].to_string(),
        format!("{}0", &token[..token.len() - 1]),
        token.chars().rev().collect::<String>(),
        String::new(),
        "\u{0}".repeat(token.len()),
    ];
    for candidate in near_misses {
        assert!(!auth::token_matches(&candidate, token), "{candidate:?}");
    }
}
