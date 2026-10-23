use super::*;

#[tokio::test]
async fn admission_refuses_excess_model_requests_before_upstream() {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let _first = Arc::clone(&config.semantic_body_budget).try_acquire_owned().unwrap();
    let _second = Arc::clone(&config.semantic_body_budget).try_acquire_owned().unwrap();
    let (proxy_task, proxy_addr) = spawn_proxy(config).await;

    let mut tcp = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let request = format!(
        "POST /api/generate HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), tcp.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy_task.await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("HTTP/1.1 502"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), upstream_listener.accept())
            .await
            .is_err()
    );
}
