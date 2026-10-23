//! The remote tool executes once; its original response remains gated.
use super::approval_fixture::ApprovalFixture;
use super::*;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test]
async fn observed_mcp_response_security_holds_original_response() {
    for (stage, outcome) in [
        ("response", None),
        ("response", Some(ApprovalOutcome::Approved)),
        ("response", Some(ApprovalOutcome::Denied)),
        ("response", Some(ApprovalOutcome::Cancelled)),
        ("request", Some(ApprovalOutcome::Approved)),
    ] {
        let seen = Arc::new(AtomicUsize::new(0));
        let upstream_seen = Arc::clone(&seen);
        let (port, upstream) = spawn_fake_upstream(move |mut socket| {
            Box::pin(async move {
                let request = read_http11_request(&mut socket).await;
                upstream_seen.fetch_add(1, Ordering::SeqCst);
                let body = br#"{"jsonrpc":"2.0","id":71,"result":{"content":[{"type":"text","text":"sensitive-original-response"}]}}"#;
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()
                ).as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
                socket.shutdown().await.unwrap();
                request
            })
        }).await;
        let action = if outcome.is_some() { "ask" } else { "block" };
        let valid = if stage == "response" { "true" } else { "false" };
        let rules = security_rules_from_toml(&format!(
            r#"
            [profiles.rules.response_only]
            name = "response_only"
            action = "{action}"
            match = 'mcp.method == "tools/call" && mcp.response.valid == "{valid}"'
        "#
        ));
        let (config, db) = make_proxy_config_with_security_rules(rules, &[80, port]);
        let fixture = ApprovalFixture::new();
        let (sender, receiver) = (&fixture.sender, &fixture.receiver);
        let mut config = Arc::try_unwrap(config).ok().unwrap();
        config.engine = Arc::new(
            capsem_core::net::proxy_engine::ProxyEngine::local(config.engine.policy().clone(), Arc::clone(&db))
                .with_approvals(Some(Arc::clone(&fixture.capability))),
        );
        let (proxy, address) = spawn_proxy(Arc::new(config)).await;
        let body = br#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"search_web","arguments":{}}}"#;
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(format!(
            "POST /remote-mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()
        ).as_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();
        let original = tokio::spawn(async move {
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            response
        });
        let mut receipt = None;
        if let Some(outcome) = outcome {
            let ApprovalClientMessage::Begin { request_id, request } =
                tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            else {
                panic!("response must become an actual pending operation")
            };
            assert_eq!(seen.load(Ordering::SeqCst), usize::from(stage == "response"));
            assert!(!original.is_finished(), "response escaped before approval");
            receipt = Some(request);
            if outcome == ApprovalOutcome::Cancelled {
                original.abort();
                assert!(original.await.unwrap_err().is_cancelled());
                assert!(matches!(
                    tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap(),
                    ApprovalClientMessage::Cancel { request_id: id } if id == request_id
                ));
                tokio::time::timeout(Duration::from_secs(2), proxy)
                    .await
                    .unwrap()
                    .unwrap();
                upstream.await.unwrap();
                assert_eq!(seen.load(Ordering::SeqCst), 1);
                db.flush_checked().await.unwrap();
                let calls = db
                    .reader()
                    .unwrap()
                    .query_raw("SELECT event_id FROM tool_calls")
                    .unwrap();
                let calls: serde_json::Value = serde_json::from_str(&calls).unwrap();
                assert_eq!(
                    calls["rows"].as_array().unwrap().len(),
                    1,
                    "executed tool audit was lost on disconnect: {calls}"
                );
                assert_eq!(calls["rows"][0][0], receipt.unwrap().event_id);
                fixture.finish().await;
                continue;
            }
            sender
                .send(ApprovalServerMessage { request_id, outcome })
                .await
                .unwrap();
        }
        let response = tokio::time::timeout(Duration::from_secs(2), original)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(response).unwrap();
        let approved = outcome == Some(ApprovalOutcome::Approved);
        assert!(
            text.starts_with(if approved { "HTTP/1.1 200" } else { "HTTP/1.1 403" }),
            "{text}"
        );
        assert_eq!(text.contains("sensitive-original-response"), approved, "{text}");
        proxy.await.unwrap();
        upstream.await.unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 1, "human resolution replayed upstream");
        db.flush_checked().await.unwrap();
        let reader = db.reader().unwrap();
        let calls = reader
            .query_raw("SELECT event_id,decision,policy_rule FROM tool_calls")
            .unwrap();
        let calls: serde_json::Value = serde_json::from_str(&calls).unwrap();
        assert_eq!(calls["rows"].as_array().unwrap().len(), 1, "{calls}");
        assert_eq!(calls["rows"][0][1], if approved { "allowed" } else { "denied" });
        if let Some(receipt) = receipt {
            assert_eq!(calls["rows"][0][0], receipt.event_id);
        }
        assert_eq!(
            calls["rows"][0][2], "profiles.rules.response_only",
            "later unmatched response erased original policy provenance"
        );
        fixture.finish().await;
    }
}
