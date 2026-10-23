//! A disconnected internal HTTP client cannot erase an executed tool's audit.
use super::approval_fixture::ApprovalFixture;
use super::*;
use capsem_core::net::mitm_proxy::{McpEndpointState, McpTimeouts};
use capsem_proto::approval::ApprovalClientMessage;
use capsem_proto::mcp_aggregator::{AggregatorClient, AggregatorMethod, AggregatorResponse, AggregatorResult};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test]
async fn internal_http_response_disconnect_drains_original_tool_audit() {
    let rules = security_rules_from_toml(
        r#"
        [profiles.rules.ask_response]
        name = "ask_response"
        action = "ask"
        match = 'mcp.method == "tools/call" && mcp.response.valid == "true"'
    "#,
    );
    let (config, db) = make_proxy_config_with_security_rules(rules, &[80]);
    let mut config = Arc::try_unwrap(config).ok().unwrap();
    let fixture = ApprovalFixture::new();
    let (aggregator, mut calls) = AggregatorClient::channel(8);
    let seen = Arc::new(AtomicUsize::new(0));
    let executed = Arc::clone(&seen);
    let worker = tokio::spawn(async move {
        while let Some((request, reply)) = calls.recv().await {
            assert!(matches!(request.method, AggregatorMethod::CallTool { .. }));
            executed.fetch_add(1, Ordering::SeqCst);
            let _ = reply.send(AggregatorResponse {
                id: request.id,
                body: AggregatorResult::CallResult {
                    result: serde_json::json!({"content":[{"type":"text","text":"original-private-result"}]}),
                },
            });
        }
    });
    config.mcp_endpoint = Some(Arc::new(
        McpEndpointState::new(
            aggregator,
            Arc::new(std::sync::RwLock::new(Arc::clone(
                config.engine.policy().snapshot().security_rules(),
            ))),
            Arc::new(std::sync::RwLock::new(Arc::clone(
                config.engine.policy().snapshot().plugins(),
            ))),
            Arc::new(tokio::sync::Semaphore::new(4)),
            McpTimeouts::default(),
        )
        .with_approvals(Some(Arc::clone(&fixture.capability))),
    ));
    let (proxy, address) = spawn_proxy(Arc::new(config)).await;
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = br#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"local__test","arguments":{}}}"#;
    client.write_all(format!("POST /mcp HTTP/1.1\r\nHost: mcp.capsem.internal\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
    client.write_all(body).await.unwrap();
    let original = tokio::spawn(async move {
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        response
    });
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(Duration::from_secs(2), fixture.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("original response must be held")
    };
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(!original.is_finished());
    original.abort();
    assert!(original.await.unwrap_err().is_cancelled());
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), fixture.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id:id } if id == request_id)
    );
    tokio::time::timeout(Duration::from_secs(2), proxy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    db.flush_checked().await.unwrap();
    let result = db
        .reader()
        .unwrap()
        .query_raw("SELECT event_id FROM tool_calls")
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        result["rows"].as_array().unwrap().len(),
        1,
        "executed original tool audit disappeared: {result}"
    );
    assert_eq!(result["rows"][0][0], request.event_id);
    fixture.finish().await;
    worker.await.unwrap();
}
