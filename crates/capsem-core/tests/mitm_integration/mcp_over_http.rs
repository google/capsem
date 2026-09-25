//! MCP-over-HTTP to a server the proxy does not know still lands in the
//! unified tool-call ledger, allowed or denied by a rule.

use super::*;

/// The tool-call rows every surface counts (the `native`, `mcp`, `builtin`
/// and `local` origins), newest first, one JSON object per row.
fn tool_call_rows(reader: &capsem_logger::DbReader) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let raw = reader
        .query_raw(
            "SELECT origin, model_call_id, method, tool_name, request_id, decision, bytes_sent,
                    server_name, policy_rule
             FROM tool_calls
             WHERE origin IN ('native', 'mcp', 'builtin', 'local')
             ORDER BY id DESC",
        )
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let columns = parsed["columns"].as_array().unwrap();
    parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| column.as_str().unwrap().to_string())
                .zip(row.as_array().unwrap().iter().cloned())
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn mitm_proxy_plain_http_unknown_mcp_shape_emits_tool_call() {
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
    db.flush().await;

    let recv = received.lock().unwrap().clone();
    let recv_str = std::str::from_utf8(&recv).unwrap_or("");
    assert!(
        recv_str.contains(r#""method":"tools/call""#),
        "upstream did not receive the original MCP request body: {recv_str:?}"
    );

    let reader = db.reader().unwrap();
    let net_events = reader.recent_net_events(10).unwrap();
    assert_eq!(net_events.len(), 1, "MCP-over-HTTP still emits HTTP telemetry");
    assert_eq!(net_events[0].path.as_deref(), Some("/remote-mcp"));

    let tool_calls = tool_call_rows(&reader);
    assert_eq!(
        tool_calls.len(),
        1,
        "unknown remote MCP-over-HTTP must emit one unified tool_calls row"
    );
    let call = &tool_calls[0];
    assert_eq!(call["origin"], "mcp");
    assert!(call["model_call_id"].is_null());
    assert_eq!(call["method"], "tools/call");
    assert_eq!(call["tool_name"], "search_web");
    assert_eq!(call["request_id"], "call-1");
    assert_eq!(call["decision"], "allowed");
    assert_eq!(call["bytes_sent"], req_body_len as u64);
    assert!(
        call["server_name"]
            .as_str()
            .is_some_and(|name| name.contains("127.0.0.1")),
        "observed MCP server identity should include host/path: {:?}",
        call["server_name"]
    );
}

#[tokio::test]
async fn mitm_proxy_plain_http_unknown_mcp_tool_call_can_be_blocked_by_rule() {
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
    db.flush().await;

    let resp_text = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_text.contains("HTTP/1.1 403"),
        "MCP rule did not block request:\n{resp_text}"
    );

    let reader = db.reader().unwrap();
    let tool_calls = tool_call_rows(&reader);
    assert_eq!(
        tool_calls.len(),
        1,
        "denied unknown MCP-over-HTTP must still emit one unified tool_calls row"
    );
    let call = &tool_calls[0];
    assert_eq!(call["origin"], "mcp");
    assert!(call["model_call_id"].is_null());
    assert_eq!(call["method"], "tools/call");
    assert_eq!(call["tool_name"], "search_web");
    assert_eq!(call["decision"], "denied");
    assert_eq!(call["policy_rule"], "profiles.rules.block_search_web_mcp");
}
