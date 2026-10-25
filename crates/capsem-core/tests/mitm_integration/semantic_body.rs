use super::*;

#[tokio::test(start_paused = true)]
async fn stalled_admission_times_out_before_upstream_without_bypassing_inspection() {
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
    tokio::time::timeout(std::time::Duration::from_secs(120), tcp.read_to_end(&mut response))
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

#[tokio::test]
async fn queued_model_request_reaches_upstream_after_body_capacity_returns() {
    let (upstream_port, upstream) = spawn_fake_upstream(|mut socket| {
        Box::pin(async move {
            let request = read_http11_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
            request
        })
    })
    .await;
    let (config, _db) = make_proxy_config_full(&["127.0.0.1"], &[], false, &[80, upstream_port]);
    let first = Arc::clone(&config.semantic_body_budget).acquire_owned().await.unwrap();
    let second = Arc::clone(&config.semantic_body_budget).acquire_owned().await.unwrap();
    let (proxy, addr) = spawn_proxy(config).await;
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(format!("POST /api/generate HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
    let mut byte = [0];
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.read(&mut byte))
            .await
            .is_err(),
        "capacity pressure must wait instead of returning 502"
    );
    drop(first);
    drop(second);
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("HTTP/1.1 200"));
    assert!(String::from_utf8_lossy(&upstream.await.unwrap()).contains("POST /api/generate"));
}
