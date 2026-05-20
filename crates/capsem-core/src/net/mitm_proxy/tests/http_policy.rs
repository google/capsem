use super::*;

#[tokio::test]
async fn policy_http_response_rewrite_strips_headers_before_guest_and_telemetry() {
    let (port, upstream_task) = spawn_http_fixture_response(
        302,
        "Found",
        vec![
            ("location", "https://github.com/openai/capsem?ref=secret"),
            ("set-cookie", "session=secret"),
            ("x-secret-token", "secret"),
        ],
        "redirecting",
    )
    .await;
    let host = format!("127.0.0.1:{port}");
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.http.rewrite_response_location]
on = "http.response"
if = 'request.host == "127.0.0.1" && request.path == "/openai/capsem" && response.status == "302"'
decision = "rewrite"
priority = 10
reason = "Mirror redirect and strip response credentials"
rewrite_target = 'response.headers.location =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
strip_response_headers = ["Set-Cookie", "X-Secret-Token"]
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) = open_plain_http_proxy_conn(&config).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/openai/capsem")
        .header("host", host.as_str())
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let has_cookie = resp.headers().contains_key("set-cookie");
    let has_secret_header = resp.headers().contains_key("x-secret-token");
    let _ = resp.into_body().collect().await.unwrap();
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();

    assert_eq!(status, 302);
    assert_eq!(
        location.as_deref(),
        Some("https://github.com/openclaw/capsem?ref=secret")
    );
    assert!(!has_cookie, "guest response must not include Set-Cookie");
    assert!(
        !has_secret_header,
        "guest response must not include stripped secret headers"
    );
    assert!(
        upstream_request.starts_with("GET /openai/capsem "),
        "proxy should still dispatch the original request upstream"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(302));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_response_location")
    );
    let response_headers = event.response_headers.as_deref().unwrap_or_default();
    let rewritten_digest = blake3::hash(b"https://github.com/openclaw/capsem?ref=secret")
        .to_hex()
        .to_string();
    let original_digest = blake3::hash(b"https://github.com/openai/capsem?ref=secret")
        .to_hex()
        .to_string();
    let rewritten_location_marker = format!("location: hash:{}", &rewritten_digest[..12]);
    let original_location_marker = format!("location: hash:{}", &original_digest[..12]);
    assert!(
        response_headers.contains(&rewritten_location_marker),
        "response telemetry should contain the rewritten Location hash, got: {response_headers:?}"
    );
    assert!(
        !response_headers.contains("set-cookie")
            && !response_headers.contains("x-secret-token")
            && !response_headers.contains("session=secret")
            && !response_headers.contains(&original_location_marker),
        "response telemetry must reflect the stripped/re-written response head"
    );
}

#[tokio::test]
async fn policy_http_response_bogus_rewrite_fails_closed_without_leaking_upstream_response() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("x-secret-token", "secret-header")],
        "super-secret-body",
    )
    .await;
    let host = format!("127.0.0.1:{port}");
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.http.rewrite_response_body]
on = "http.response"
if = 'request.host == "127.0.0.1" && response.status == "200"'
decision = "rewrite"
priority = 10
reason = "Body rewrite is not supported on response heads"
rewrite_target = 'response.body =~ "super-secret-body"'
rewrite_value = "[redacted]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) = open_plain_http_proxy_conn(&config).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/secret")
        .header("host", host.as_str())
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let headers = format_headers(resp.headers());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body).into_owned();
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    assert_eq!(status, 403);
    assert!(
        !headers.contains("x-secret-token") && !headers.contains("secret-header"),
        "guest response headers must not leak the upstream response on fail-closed rewrite"
    );
    assert!(
        !body.contains("super-secret-body"),
        "guest response body must not leak upstream content on fail-closed rewrite"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_response_body")
    );
    assert!(
        !event
            .response_headers
            .as_deref()
            .unwrap_or_default()
            .contains("secret-header"),
        "fail-closed telemetry must not preserve upstream response headers"
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("super-secret-body"),
        "fail-closed telemetry must not preserve upstream response body"
    );
}

#[tokio::test]
async fn policy_http_block_stops_before_upstream_and_records_policy_fields() {
    let config = make_config_with_rules_policy(
        allow_test_domain_policy(),
        policy_from_toml(&format!(
            r#"
[policy.http.block_openai_path]
on = "http.request"
if = 'request.host == "{TEST_DOMAIN}" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Do not fetch this path"
"#
        )),
    );
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    let status = send_get(&mut sender, TEST_DOMAIN, "/openai/capsem").await;
    assert_eq!(status, 403, "Policy block should not reach upstream");
    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.block_openai_path")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Do not fetch this path")
    );
}

#[tokio::test]
async fn policy_http_ask_placeholder_confirmer_allows_upstream_dispatch() {
    let (port, upstream_task) =
        spawn_http_fixture_response(200, "OK", vec![("content-type", "text/plain")], "confirmed")
            .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.http.ask_openai_path]
on = "http.request"
if = 'request.host == "127.0.0.1" && request.path.matches("^/openai(/|$)")'
decision = "ask"
priority = 10
reason = "Ask before fetching this path"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, None).await;

    let status = send_get(&mut sender, "127.0.0.1", "/openai/capsem").await;
    assert_eq!(
        status, 200,
        "placeholder-confirmed Policy ask should dispatch upstream"
    );
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("GET /openai/capsem"),
        "ask accept path must reach upstream, got: {upstream_request}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.ask_openai_path")
    );
}

#[tokio::test]
async fn policy_http_rewrite_strips_request_headers_before_telemetry_and_upstream() {
    let config = make_config_with_rules_policy(
        allow_test_domain_policy(),
        policy_from_toml(&format!(
            r#"
[policy.http.rewrite_openai_path]
on = "http.request"
if = 'request.host == "{TEST_DOMAIN}" && request.path.matches("^/openai/") && has(request.headers.authorization)'
decision = "rewrite"
priority = 10
reason = "Mirror path and strip credentials"
rewrite_target = 'request.url =~ "^https://{TEST_DOMAIN}/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://{TEST_DOMAIN}/openclaw/${{repo}}${{rest}}"
strip_request_headers = ["Authorization"]
"#
        )),
    );
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/openai/capsem?token=secret")
        .header("host", TEST_DOMAIN)
        .header("authorization", "Bearer secret")
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(
        resp.status().as_u16(),
        502,
        "rewrite should dispatch the rewritten request; the test domain then fails upstream"
    );
    let _ = resp.into_body().collect().await;
    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Error);
    assert_eq!(event.path.as_deref(), Some("/openclaw/capsem"));
    assert_eq!(event.query.as_deref(), Some("token=secret"));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_openai_path")
    );
    assert!(
        !event
            .request_headers
            .as_deref()
            .unwrap_or_default()
            .contains("authorization"),
        "stripped credential header must not appear in request telemetry"
    );
}

/// Disabling a provider mid-connection blocks subsequent requests on the
/// same keep-alive connection. This is the core regression test for the
/// per-request policy reload fix.
#[tokio::test]
async fn policy_hot_reload_blocks_on_same_connection() {
    let config = make_config_dev();
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // First request: allowed. Returns 502 because there's no real upstream,
    // but 502 proves the policy allowed the request past the policy check
    // (denied would be 403).
    let status1 = send_get(&mut sender, TEST_DOMAIN, "/before-disable").await;
    assert_eq!(
        status1, 502,
        "allowed request should reach upstream (502 = no upstream, not 403)"
    );

    // Hot-reload: swap to a blocking Policy config.
    *config.policy.write().await = http_block_test_domain_config();

    // Second request on the SAME keep-alive connection: must be denied.
    let status2 = send_get(&mut sender, TEST_DOMAIN, "/after-disable").await;
    assert_eq!(
        status2, 403,
        "request after policy swap must be denied on same connection"
    );

    drop(sender);
    let _ = proxy_task.await;

    // Verify telemetry recorded both events with correct decisions.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let reader = config.db.reader().unwrap();
    let mut events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        2,
        "should have 2 events (one allowed, one denied)"
    );
    events.reverse(); // chronological
                      // First event: allowed (502 upstream error, but decision is Error not Denied).
    assert!(
        events[0].decision != Decision::Denied,
        "first request should not be denied, got {:?}",
        events[0].decision
    );
    assert_eq!(events[0].path, Some("/before-disable".to_string()));
    // Second event: denied (403).
    assert_eq!(events[1].decision, Decision::Denied);
    assert_eq!(events[1].path, Some("/after-disable".to_string()));
    assert_eq!(events[1].status_code, Some(403));
}

/// Re-enabling a provider mid-connection allows subsequent requests on
/// the same keep-alive connection (reverse direction of the above test).
#[tokio::test]
async fn policy_hot_reload_allows_on_same_connection() {
    // Start with a blocking Policy config.
    let config = make_config_deny_all();
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // First request: denied.
    let status1 = send_get(&mut sender, TEST_DOMAIN, "/while-denied").await;
    assert_eq!(status1, 403);

    // Hot-reload: swap to an empty Policy config, which allows by default.
    *config.policy.write().await = Arc::new(PolicyConfig::default());

    // Second request: allowed (502 = no upstream, proves policy let it through).
    let status2 = send_get(&mut sender, TEST_DOMAIN, "/after-enable").await;
    assert_eq!(
        status2, 502,
        "request after re-enable should be allowed (502 = no upstream)"
    );

    drop(sender);
    let _ = proxy_task.await;
}

/// Multiple policy swaps on the same connection: deny -> allow -> deny.
/// Verifies each request sees the current policy, not any cached version.
#[tokio::test]
async fn policy_hot_reload_multiple_swaps() {
    let config = make_config_deny_all();
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // Request 1: denied.
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r1").await, 403);

    // Swap to allow.
    *config.policy.write().await = Arc::new(PolicyConfig::default());

    // Request 2: allowed (502).
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r2").await, 502);

    // Swap back to deny.
    *config.policy.write().await = http_block_test_domain_config();

    // Request 3: denied again.
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r3").await, 403);

    drop(sender);
    let _ = proxy_task.await;

    // Verify all 3 events recorded.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        3,
        "all 3 requests should produce telemetry events"
    );
}
