//! The original HTTP audit survives a caller leaving during human review.
use super::approval_fixture::ApprovalFixture;
use super::*;
use capsem_proto::approval::ApprovalClientMessage;
use std::time::Duration;

#[tokio::test]
async fn disconnected_http_approval_drains_original_network_audit() {
    original_http_audit(false).await;
}

#[tokio::test]
async fn stopped_http_approval_drains_original_network_audit() {
    original_http_audit(true).await;
}

#[tokio::test]
async fn stopped_unclassified_connection_does_not_delay_retirement() {
    for prefix in [&b""[..], &b"\x16\x03\x03\x00\x20"[..]] {
        let (config, _) = make_proxy_config(&[], &[], false);
        let shutdown = config.shutdown.clone();
        let (proxy, address) = spawn_proxy(config).await;
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(prefix).await.unwrap();
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), proxy)
            .await
            .unwrap()
            .unwrap();
    }
}

async fn original_http_audit(stop: bool) {
    let rules = security_rules_from_toml(
        r#"
        [profiles.rules.ask_http]
        name = "ask_http"
        action = "ask"
        match = 'http.method == "POST"'
        "#,
    );
    let (config, db) = make_proxy_config_with_security_rules(rules, &[80]);
    let fixture = ApprovalFixture::new();
    let mut config = Arc::try_unwrap(config).ok().unwrap();
    config.engine = Arc::new(
        capsem_core::net::proxy_engine::ProxyEngine::local(config.engine.policy().clone(), Arc::clone(&db))
            .with_approvals(Some(Arc::clone(&fixture.capability))),
    );
    let shutdown = config.shutdown.clone();
    let (proxy, address) = spawn_proxy(Arc::new(config)).await;
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"POST /original HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(Duration::from_secs(2), fixture.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("the original HTTP request must reach actual approval admission")
    };
    let client = if stop {
        shutdown.cancel();
        Some(client)
    } else {
        drop(client);
        None
    };
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), fixture.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id
    ));
    tokio::time::timeout(Duration::from_secs(2), proxy)
        .await
        .unwrap()
        .unwrap();
    db.flush_checked().await.unwrap();
    let rows = db
        .reader()
        .unwrap()
        .query_raw("SELECT event_id,decision,method,path,status_code FROM net_events")
        .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(rows["rows"].as_array().unwrap().len(), 1, "original audit lost: {rows}");
    assert_eq!(rows["rows"][0][0], request.event_id);
    assert_eq!(rows["rows"][0][1], "denied");
    assert_eq!(rows["rows"][0][2], "POST");
    assert_eq!(rows["rows"][0][3], "/original");
    assert!(
        rows["rows"][0][4].is_null(),
        "an abandoned caller did not observe a response: {rows}"
    );
    fixture.finish().await;
    drop(client);
}
