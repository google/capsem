//! Every read after the first byte is bounded.
//!
//! The classification read had a deadline, but three reads after it did
//! not: the framed-MCP prefix loop, the guest TLS handshake, and hyper's
//! header read (whose default 30s timeout is silently dropped when the
//! server builder has no timer). A guest that sent one byte and stalled
//! pinned the handler task and its ACTIVE_CONNECTIONS slot forever.
//!
//! These tests run on tokio's paused clock: when every task is idle the
//! runtime jumps to the next timer, so a real deadline fires instantly and
//! a missing one leaves the proxy task pending until the outer bound.

use std::time::Duration;

use super::*;

/// Far beyond any proxy deadline. On the paused clock this only ever
/// elapses when the proxy has no timer of its own.
const OUTER_BOUND: Duration = Duration::from_secs(600);

/// Send `prefix`, then stall. The proxy task returning is the observable
/// release of the connection slot: `handle_connection` drops its gauge
/// guard on return, and nothing else does.
async fn stalled_guest_is_disconnected(prefix: &[u8]) {
    let (config, _db) = make_proxy_config(&[], &[], true);
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(prefix).await.unwrap();

    let finished = tokio::time::timeout(OUTER_BOUND, proxy_task).await;
    assert!(
        finished.is_ok(),
        "proxy kept the connection after {prefix:?} + stall: no deadline on the next read"
    );
    finished.unwrap().unwrap();

    // The guest side observes the teardown too: no bytes, or a close.
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(OUTER_BOUND, tcp.read_to_end(&mut buf))
        .await
        .expect("proxy closed the socket");
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_tls_first_byte_is_disconnected() {
    // 0x16 = TLS handshake record: classification hands off to the TLS
    // acceptor, which then waits for the rest of the ClientHello.
    stalled_guest_is_disconnected(b"\x16").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_client_hello_is_disconnected() {
    // A plausible record header announcing a 512-byte ClientHello that
    // never arrives.
    stalled_guest_is_disconnected(b"\x16\x03\x01\x02\x00\x01").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_http_first_byte_is_disconnected() {
    // 'G': plain HTTP, hyper waits for the rest of the request head.
    stalled_guest_is_disconnected(b"G").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_http_head_is_disconnected() {
    stalled_guest_is_disconnected(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Slow: ").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_mcp_prefix_byte_is_disconnected() {
    // 0x00: the framed-MCP prefix loop pulls up to six bytes for the
    // classifier before deciding.
    stalled_guest_is_disconnected(b"\x00").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_metadata_is_disconnected() {
    // Metadata prefix without its terminating newline.
    stalled_guest_is_disconnected(b"\x00CAPSEM_META:curl").await;
}

/// SSE model responses on default policies must stream `200 OK` headers and
/// initial SSE chunks to the client immediately without waiting for the
/// upstream to finish the entire LLM generation.
#[tokio::test]
async fn streaming_model_sse_delivers_headers_and_first_chunk_before_upstream_eof() {
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let chunk1 = b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"First chunk \"}]}}]}\n\n";
    let chunk2 = b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"second chunk\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":12,\"candidatesTokenCount\":8}}\n\n";

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let req = read_http11_request(&mut sock).await;
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            sock.write_all(head).await.unwrap();
            let frame1 = format!("{:x}\r\n", chunk1.len());
            sock.write_all(frame1.as_bytes()).await.unwrap();
            sock.write_all(chunk1).await.unwrap();
            sock.write_all(b"\r\n").await.unwrap();
            sock.flush().await.unwrap();

            // Hold the SSE stream open until the client proves it already received chunk1.
            let _ = release_rx.await;

            let frame2 = format!("{:x}\r\n", chunk2.len());
            sock.write_all(frame2.as_bytes()).await.unwrap();
            sock.write_all(chunk2).await.unwrap();
            sock.write_all(b"\r\n0\r\n\r\n").await.unwrap();
            sock.flush().await.unwrap();
            let _ = sock.shutdown().await;
            req
        })
    })
    .await;

    let domain = "generativelanguage.googleapis.com";
    let mut policy_inner = NetworkMechanics::new();
    policy_inner.upstream_overrides.insert(
        format!("{domain}:443"),
        UpstreamOverride {
            dial: format!("127.0.0.1:{upstream_port}"),
            protocol: UpstreamOverrideProtocol::Http,
        },
    );
    let rules = security_rules_from_toml(&format!(
        r#"
[profiles.rules.allow_google_http]
name = "allow_google_http"
action = "allow"
priority = 20
match = 'http.host == "{domain}" && tcp.port == "443"'

[profiles.rules.detect_streamed_response_body]
name = "detect_streamed_response_body"
action = "allow"
detection_level = "high"
priority = 10
match = 'model.response.body.contains("First chunk second chunk") && http.status == "200" && http.host == "{domain}"'

[default.http]
name = "default_http"
action = "block"
priority = "default"
match = 'http.valid == "true"'

[default.model]
name = "default_model"
action = "allow"
priority = "default"
match = 'model.valid == "true"'
"#
    ));
    let (config, db) = make_proxy_config_with_mechanics(rules, policy_inner);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let server_name = ServerName::try_from(domain.to_string()).unwrap();
    let mut tls = connector.connect(server_name, tcp).await.unwrap();

    let body = br#"{"contents":[{"role":"user","parts":[{"text":"solve usaco 347"}]}]}"#;
    let path = "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {domain}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    tls.write_all(body).await.unwrap();
    tls.flush().await.unwrap();

    // Read from the TLS stream while upstream is still holding the stream open.
    let mut received = Vec::new();
    let mut buf = [0u8; 4096];
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "stream closed before first chunk arrived");
            received.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&received);
            if text.contains("HTTP/1.1 200 OK") && text.contains("First chunk ") {
                break;
            }
        }
    })
    .await
    .expect("headers and first SSE chunk must arrive before upstream finishes the stream");

    // Now release the second chunk and drain to EOF.
    let _ = release_tx.send(());
    let _ = tls.read_to_end(&mut received).await.unwrap();
    drop(tls);

    upstream_task.await.unwrap();
    proxy_task.await.unwrap();
    db.flush().await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status_code, Some(200));
    assert_eq!(events[0].decision, Decision::Allowed);
    assert_eq!(
        events[0].policy_rule.as_deref(),
        Some("profiles.rules.detect_streamed_response_body")
    );

    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    let call = &calls[0].1;
    assert_eq!(call.provider, "google");
    assert_eq!(call.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(call.text_content.as_deref(), Some("First chunk second chunk"));
    assert_eq!(call.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(call.input_tokens, Some(12));
    assert_eq!(call.output_tokens, Some(8));

    let rule_events = reader.recent_security_rule_events(10).unwrap();
    assert!(
        rule_events.iter().any(|ev| {
            ev.event_type == "model.call"
                && ev.rule_id == "profiles.rules.detect_streamed_response_body"
                && ev.detection_level == capsem_logger::SecurityDetectionLevel::High
        }),
        "detection-only allow rule on model.response.body must fire at stream end: {rule_events:?}"
    );
    assert!(
        rule_events
            .iter()
            .any(|ev| ev.event_type == "model.call" && ev.rule_id == "profiles.rules.default_model"),
        "default.model rule must also be recorded at stream end: {rule_events:?}"
    );
}

/// When an in-flight model request is aborted by the client before upstream
/// sends response headers (e.g. a 60s headersTimeout), MITM still records
/// both NetEvent and ModelCall in session.db.
#[tokio::test]
async fn aborted_in_flight_model_request_before_response_headers_records_telemetry() {
    let (got_req_tx, got_req_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_upstream_tx, release_upstream_rx) = tokio::sync::oneshot::channel::<()>();

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let req = read_http11_request(&mut sock).await;
            let _ = got_req_tx.send(());
            let _ = release_upstream_rx.await;
            req
        })
    })
    .await;

    let domain = "generativelanguage.googleapis.com";
    let (config, db) = make_proxy_config_with_local_http_upstream(domain, upstream_port);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let server_name = ServerName::try_from(domain.to_string()).unwrap();
    let mut tls = connector.connect(server_name, tcp).await.unwrap();

    let body = br#"{"contents":[{"role":"user","parts":[{"text":"solve usaco 347"}]}]}"#;
    let path = "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {domain}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    tls.write_all(body).await.unwrap();
    tls.flush().await.unwrap();

    // Wait until upstream has received the forwarded request (so handle_request
    // is parked in sender.send_request().await), then abort the client socket.
    got_req_rx.await.unwrap();
    drop(tls);

    tokio::time::timeout(Duration::from_secs(5), proxy_task)
        .await
        .expect("proxy task should exit when client disconnects")
        .unwrap();
    let _ = release_upstream_tx.send(());
    let _ = upstream_task.await;
    db.flush().await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1, "aborted in-flight request must emit a NetEvent");
    let ev = &events[0];
    assert_eq!(ev.domain, domain);
    assert_eq!(ev.method.as_deref(), Some("POST"));
    assert_eq!(ev.status_code, None);
    assert_eq!(ev.decision, Decision::Error);
    assert_eq!(ev.matched_rule.as_deref(), Some("client_aborted"));
    assert_eq!(ev.bytes_sent, body.len() as u64);

    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 1, "aborted in-flight model request must emit a ModelCall");
    let call = &calls[0].1;
    assert_eq!(call.provider, "google");
    assert_eq!(call.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(call.status_code, None);
    assert_eq!(call.stop_reason.as_deref(), Some("aborted"));
    assert_eq!(call.request_bytes, body.len() as u64);
    assert_eq!(call.response_bytes, 0);
}

/// When a streaming SSE model response is aborted mid-stream after delivering
/// partial chunks, MITM records the partial response and marks
/// `stop_reason = Some("aborted")`.
#[tokio::test]
async fn aborted_mid_stream_model_sse_response_records_partial_telemetry() {
    let (release_upstream_tx, release_upstream_rx) = tokio::sync::oneshot::channel::<()>();
    let chunk1 = b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Partial answer\"}]}}]}\n\n";

    let (upstream_port, upstream_task) = spawn_fake_upstream(move |mut sock| {
        Box::pin(async move {
            let req = read_http11_request(&mut sock).await;
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            sock.write_all(head).await.unwrap();
            let frame1 = format!("{:x}\r\n", chunk1.len());
            sock.write_all(frame1.as_bytes()).await.unwrap();
            sock.write_all(chunk1).await.unwrap();
            sock.write_all(b"\r\n").await.unwrap();
            sock.flush().await.unwrap();

            // Hold upstream open without sending the terminating 0\r\n\r\n chunk.
            let _ = release_upstream_rx.await;
            req
        })
    })
    .await;

    let domain = "generativelanguage.googleapis.com";
    let (config, db) = make_proxy_config_with_local_http_upstream(domain, upstream_port);
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let connector = TlsConnector::from(Arc::new(make_tls_client_config()));
    let server_name = ServerName::try_from(domain.to_string()).unwrap();
    let mut tls = connector.connect(server_name, tcp).await.unwrap();

    let body = br#"{"contents":[{"role":"user","parts":[{"text":"solve usaco 347"}]}]}"#;
    let path = "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {domain}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    tls.write_all(body).await.unwrap();
    tls.flush().await.unwrap();

    let mut received = Vec::new();
    let mut buf = [0u8; 4096];
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "stream closed before first chunk arrived");
            received.extend_from_slice(&buf[..n]);
            if String::from_utf8_lossy(&received).contains("Partial answer") {
                break;
            }
        }
    })
    .await
    .expect("first SSE chunk should arrive");

    // Client disconnects mid-stream before EOF.
    drop(tls);

    tokio::time::timeout(Duration::from_secs(5), proxy_task)
        .await
        .expect("proxy task should exit when client disconnects mid-stream")
        .unwrap();
    let _ = release_upstream_tx.send(());
    let _ = upstream_task.await;
    db.flush().await;

    let reader = db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status_code, Some(200));
    assert!(events[0].bytes_received > 0);

    let calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(calls.len(), 1);
    let call = &calls[0].1;
    assert_eq!(call.provider, "google");
    assert_eq!(call.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(call.text_content.as_deref(), Some("Partial answer"));
    assert_eq!(call.stop_reason.as_deref(), Some("aborted"));
    assert!(call.response_bytes > 0);
}
