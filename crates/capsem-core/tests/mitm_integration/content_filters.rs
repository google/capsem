use super::*;
use capsem_core::net::policy_config::{SecurityPluginConfig, SecurityPluginMode};

fn enable(config: &MitmProxyConfig, id: &str, mode: SecurityPluginMode) {
    let snapshot = config.engine.policy().snapshot();
    config
        .engine
        .policy()
        .replace(capsem_core::net::proxy_engine::ProxyPolicySnapshot::new(
            "content-filter-test".into(),
            snapshot.network().clone(),
            snapshot.security_rules().as_ref().clone(),
            BTreeMap::from([(
                id.into(),
                SecurityPluginConfig {
                    mode,
                    ..Default::default()
                },
            )]),
            snapshot.model_endpoints().clone(),
        ));
}

#[tokio::test]
async fn content_filter_masks_chunk_split_request_and_gzip_response_on_wire() {
    let (upstream_port, upstream) = spawn_fake_upstream(|mut socket| Box::pin(async move {
        let observed = read_http11_request(&mut socket).await;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gzip, b"reply@example.com").unwrap();
        let gzip = gzip.finish().unwrap();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", gzip.len()).as_bytes()).await.unwrap();
        socket.write_all(&gzip).await.unwrap();
        observed
    })).await;
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    enable(&config, "pii_filter", SecurityPluginMode::Rewrite);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /text HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    for chunk in ["user@", "example.com"] {
        socket
            .write_all(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes())
            .await
            .unwrap();
    }
    socket.write_all(b"0\r\n\r\n").await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    let observed = String::from_utf8(upstream.await.unwrap()).unwrap();
    assert!(
        !observed.contains("user@example.com"),
        "upstream must receive masked bytes: {observed}"
    );
    assert!(observed.contains("****************"));
    let response = String::from_utf8(response).unwrap();
    assert!(response.contains("HTTP/1.1 200"), "{response}");
    assert!(
        !response.contains("reply@example.com"),
        "guest must receive masked response"
    );
    assert!(response.contains("*****************"));
}

#[tokio::test]
async fn content_filter_blocks_secret_before_upstream_connects() {
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = upstream.local_addr().unwrap().port();
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "secret_exfiltration", SecurityPluginMode::Block);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
    socket.write_all(format!("POST /leak HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("HTTP/1.1 403"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    db.flush().await;
    let ledger = capsem_logger::DbHandle::open_external_reader(db.path()).unwrap();
    let counters = ledger.ledger_counters().await.unwrap();
    assert_eq!(counters.plugins["secret_exfiltration"].detections, 1);
    assert_eq!(counters.plugins["secret_exfiltration"].applied, 1);
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let archived = ledger.read_bodies(events[0].event_id.as_ref().unwrap()).await.unwrap();
    for body_record in archived {
        assert!(
            !body_record
                .bytes
                .windows(body.len())
                .any(|part| part == body.as_bytes()),
            "blocked token leaked to archive"
        );
    }
}

#[tokio::test]
async fn content_filter_counts_response_only_match_and_keeps_payload_out_of_archive() {
    let sensitive = "reply@example.com";
    let (port, upstream) = spawn_fake_upstream(move |mut socket| Box::pin(async move {
        let observed = read_http11_request(&mut socket).await;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sensitive}", sensitive.len()).as_bytes()).await.unwrap();
        observed
    })).await;
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "pii_filter", SecurityPluginMode::Rewrite);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(format!("GET /clean HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), upstream)
        .await
        .unwrap()
        .unwrap();
    assert!(response.ends_with(b"*****************"));
    db.flush().await;
    let ledger = capsem_logger::DbHandle::open_external_reader(db.path()).unwrap();
    let counters = ledger.ledger_counters().await.unwrap();
    assert_eq!(counters.plugins["pii_filter"].detections, 1);
    assert_eq!(counters.plugins["pii_filter"].applied, 1);
    let events = db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    for body in ledger.read_bodies(events[0].event_id.as_ref().unwrap()).await.unwrap() {
        assert!(!body
            .bytes
            .windows(sensitive.len())
            .any(|part| part == sensitive.as_bytes()));
    }
}

#[tokio::test]
async fn utf8_guard_rejects_invalid_inbound_text() {
    let (port, upstream) = spawn_fake_upstream(move |mut socket| Box::pin(async move {
        let observed = read_http11_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n\xf0\x9f").await.unwrap();
        observed
    })).await;
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "utf8_guard", SecurityPluginMode::Block);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(format!("GET /invalid HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), upstream)
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8(response).unwrap().contains("HTTP/1.1 403"));
    db.flush().await;
    let counters = capsem_logger::DbHandle::open_external_reader(db.path())
        .unwrap()
        .ledger_counters()
        .await
        .unwrap();
    assert_eq!(counters.plugins["utf8_guard"].applied, 1);
}

#[tokio::test]
async fn utf8_guard_rejects_truncated_text_before_upstream_and_records_detection() {
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = upstream.local_addr().unwrap().port();
    let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "utf8_guard", SecurityPluginMode::Block);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /text HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    socket.write_all(&[0xf0, 0x9f]).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("HTTP/1.1 403"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    db.flush().await;
    let counters = capsem_logger::DbHandle::open_external_reader(db.path())
        .unwrap()
        .ledger_counters()
        .await
        .unwrap();
    assert_eq!(counters.plugins["utf8_guard"].detections, 1);
}

#[tokio::test]
async fn utf8_guard_preserves_binary_request_and_response() {
    let binary = vec![0xff, 0x00, 0x80, 0x89, b'P', b'N', b'G'];
    let reply = binary.clone();
    let (port, upstream) = spawn_fake_upstream(move |mut socket| Box::pin(async move {
        let observed = read_http11_request(&mut socket).await;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",reply.len()).as_bytes()).await.unwrap();
        socket.write_all(&reply).await.unwrap();
        observed
    })).await;
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "utf8_guard", SecurityPluginMode::Block);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /binary HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",binary.len()).as_bytes()).await.unwrap();
    socket.write_all(&binary).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    assert!(upstream.await.unwrap().ends_with(&binary));
    assert!(response.ends_with(&binary));
}

#[tokio::test]
async fn utf8_guard_preserves_unicode_split_across_transfer_chunks() {
    let expected = "café 中文 🦀".as_bytes().to_vec();
    let reply = expected.clone();
    let (port, upstream) = spawn_fake_upstream(move |mut socket| Box::pin(async move {
        let observed = read_http11_request(&mut socket).await;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",reply.len()).as_bytes()).await.unwrap();
        socket.write_all(&reply).await.unwrap();
        observed
    })).await;
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "utf8_guard", SecurityPluginMode::Block);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /unicode HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    for byte in &expected {
        socket
            .write_all(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n'])
            .await
            .unwrap();
    }
    socket.write_all(b"0\r\n\r\n").await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), upstream)
        .await
        .unwrap()
        .unwrap()
        .ends_with(&expected));
    assert!(response.ends_with(&expected));
}

#[tokio::test]
async fn content_filter_rejects_unsupported_encoding_and_mcp_without_sending_bytes() {
    let mcp = br#"{"jsonrpc":"2.0","id":"one","method":"tools/call","params":{"name":"search","arguments":{"q":"user@example.com"}}}"#;
    for (encoding, media, body) in [
        ("br", "text/plain", b"user@example.com".as_slice()),
        ("gzip", "text/plain", b"not a gzip stream".as_slice()),
        ("identity", "application/json", mcp.as_slice()),
    ] {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = upstream.local_addr().unwrap().port();
        let (config, db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
        enable(&config, "pii_filter", SecurityPluginMode::Rewrite);
        let (proxy, addr) = spawn_proxy(config).await;
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket.write_all(format!("POST /remote-mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: {media}\r\nContent-Encoding: {encoding}\r\nX-Copy: user@example.com\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        proxy.await.unwrap();
        assert!(String::from_utf8(response).unwrap().contains("HTTP/1.1 502"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), upstream.accept())
                .await
                .is_err()
        );
        db.flush().await;
        let events = db.reader().unwrap().recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0]
            .request_headers
            .as_deref()
            .unwrap_or("")
            .contains("user@example.com"));
    }
}

#[tokio::test]
async fn content_filter_masks_gzip_request_before_upstream() {
    let (port, upstream) = spawn_fake_upstream(move |mut socket| {
        Box::pin(async move {
            let observed = read_http11_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            observed
        })
    })
    .await;
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, port]);
    enable(&config, "pii_filter", SecurityPluginMode::Rewrite);
    let (proxy, addr) = spawn_proxy(config).await;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gzip, b"user@example.com").unwrap();
    let gzip = gzip.finish().unwrap();
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /text HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", gzip.len()).as_bytes()).await.unwrap();
    socket.write_all(&gzip).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    let observed = tokio::time::timeout(std::time::Duration::from_secs(5), upstream)
        .await
        .unwrap()
        .unwrap();
    assert!(observed.ends_with(b"****************"));
    assert!(!String::from_utf8(observed)
        .unwrap()
        .to_ascii_lowercase()
        .contains("content-encoding:"));
}
