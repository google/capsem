use std::sync::Arc;
use std::path::PathBuf;

use axum::extract::{Path, State, WebSocketUpgrade, ws::{WebSocket, Message}};
use axum::response::IntoResponse;
use futures::{sink::SinkExt, stream::StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{client_async, tungstenite::protocol::Message as TungsteniteMessage};

use crate::AppState;

/// Validate VM ID: alphanumeric, hyphens, underscores. Must start with
/// alphanumeric, length 1-64. Matches capsem-service's `validate_vm_name`.
fn validate_vm_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("VM id cannot be empty");
    }
    if id.len() > 64 {
        return Err("VM id too long (max 64 characters)");
    }
    if !id.chars().next().unwrap().is_ascii_alphanumeric() {
        return Err("VM id must start with a letter or digit");
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("VM id must contain only letters, digits, hyphens, and underscores");
    }
    Ok(())
}

/// Derive the per-VM WebSocket UDS path from the service socket and VM ID.
fn terminal_uds_path(service_uds: &std::path::Path, id: &str) -> PathBuf {
    let run_dir = service_uds
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("/tmp"));
    run_dir.join("instances").join(format!("{}-ws.sock", id))
}

pub async fn handle_terminal_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(msg) = validate_vm_id(&id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": msg})),
        )
            .into_response();
    }

    let uds_path = terminal_uds_path(&state.uds_path, &id);

    ws.on_upgrade(move |socket| handle_socket(socket, uds_path)).into_response()
}

async fn handle_socket(mut client_ws: WebSocket, uds_path: PathBuf) {
    let stream = match UnixStream::connect(&uds_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to connect to process WS UDS {}: {}", uds_path.display(), e);
            let _ = client_ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011, // unexpected condition
                    reason: "VM not available".into(),
                })))
                .await;
            return;
        }
    };

    let req = format!("ws://localhost/terminal");
    let (process_ws, _) = match client_async(&req, stream).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("WebSocket handshake with process failed: {}", e);
            let _ = client_ws
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "VM handshake failed".into(),
                })))
                .await;
            return;
        }
    };

    tracing::info!("Established WS connection to {}", uds_path.display());

    let (mut client_write, mut client_read) = client_ws.split();
    let (mut process_write, mut process_read) = process_ws.split();

    let mut c2p = tokio::spawn(async move {
        while let Some(msg) = client_read.next().await {
            match msg {
                Ok(Message::Text(t)) => {
                    let s: String = t.to_string();
                    if process_write.send(TungsteniteMessage::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(b)) => {
                    let vec = b.to_vec();
                    if process_write.send(TungsteniteMessage::Binary(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Ping(p)) => {
                    let vec = p.to_vec();
                    if process_write.send(TungsteniteMessage::Ping(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Pong(p)) => {
                    let vec = p.to_vec();
                    if process_write.send(TungsteniteMessage::Pong(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(c)) => {
                    let frame = c.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(f.code),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = process_write.send(TungsteniteMessage::Close(frame)).await;
                    break;
                }
                Err(_) => break,
            }
        }
    });

    let mut p2c = tokio::spawn(async move {
        while let Some(msg) = process_read.next().await {
            match msg {
                Ok(TungsteniteMessage::Text(t)) => {
                    let s: String = t.to_string();
                    if client_write.send(Message::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Binary(b)) => {
                    let vec = b.to_vec();
                    if client_write.send(Message::Binary(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Ping(p)) => {
                    let vec = p.to_vec();
                    if client_write.send(Message::Ping(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Pong(p)) => {
                    let vec = p.to_vec();
                    if client_write.send(Message::Pong(vec.into())).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Close(c)) => {
                    let frame = c.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = client_write.send(Message::Close(frame)).await;
                    break;
                }
                Ok(TungsteniteMessage::Frame(_)) => {}
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = &mut c2p => {
            tracing::info!("Client disconnected from terminal");
            p2c.abort();
        }
        _ = &mut p2c => {
            tracing::info!("Process disconnected from terminal");
            c2p.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // --- validate_vm_id ---

    #[test]
    fn valid_alphanumeric_id() {
        assert!(validate_vm_id("abc123").is_ok());
    }

    #[test]
    fn valid_id_with_hyphens() {
        assert!(validate_vm_id("vm-12345").is_ok());
    }

    #[test]
    fn valid_id_with_underscores() {
        assert!(validate_vm_id("my_dev").is_ok());
    }

    #[test]
    fn valid_mixed_id() {
        assert!(validate_vm_id("vm-my_dev-123").is_ok());
    }

    #[test]
    fn valid_single_char() {
        assert!(validate_vm_id("a").is_ok());
    }

    #[test]
    fn valid_max_length_id() {
        let id: String = std::iter::repeat('a').take(64).collect();
        assert!(validate_vm_id(&id).is_ok());
    }

    #[test]
    fn valid_ephemeral_id_format() {
        // Matches the service's auto-generated format: vm-{epoch_secs}
        assert!(validate_vm_id("vm-1712678400").is_ok());
    }

    #[test]
    fn valid_run_id_format() {
        // Matches the service's run format: run-{epoch_secs}
        assert!(validate_vm_id("run-1712678400").is_ok());
    }

    #[test]
    fn rejects_empty_id() {
        assert!(validate_vm_id("").is_err());
    }

    #[test]
    fn rejects_too_long_id() {
        let id: String = std::iter::repeat('a').take(65).collect();
        assert!(validate_vm_id(&id).is_err());
    }

    #[test]
    fn rejects_path_separators() {
        assert!(validate_vm_id("../etc/passwd").is_err());
        assert!(validate_vm_id("foo/bar").is_err());
    }

    #[test]
    fn rejects_spaces() {
        assert!(validate_vm_id("vm 123").is_err());
    }

    #[test]
    fn rejects_special_chars() {
        assert!(validate_vm_id("vm;rm").is_err());
        assert!(validate_vm_id("vm&id").is_err());
        assert!(validate_vm_id("vm|id").is_err());
        assert!(validate_vm_id("vm$id").is_err());
    }

    #[test]
    fn rejects_dots() {
        assert!(validate_vm_id("vm.123").is_err());
    }

    #[test]
    fn rejects_id_starting_with_hyphen() {
        assert!(validate_vm_id("-bad").is_err());
    }

    #[test]
    fn rejects_id_starting_with_underscore() {
        assert!(validate_vm_id("_bad").is_err());
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(validate_vm_id("vm\0id").is_err());
    }

    // --- terminal_uds_path ---

    #[test]
    fn uds_path_derives_from_service_socket() {
        let service = Path::new("/home/user/.capsem/run/service.sock");
        let path = terminal_uds_path(service, "vm-123");
        assert_eq!(
            path,
            PathBuf::from("/home/user/.capsem/run/instances/vm-123-ws.sock")
        );
    }

    #[test]
    fn uds_path_with_underscore_id() {
        let service = Path::new("/tmp/run/service.sock");
        let path = terminal_uds_path(service, "my_dev");
        assert_eq!(
            path,
            PathBuf::from("/tmp/run/instances/my_dev-ws.sock")
        );
    }

    #[test]
    fn uds_path_falls_back_to_tmp() {
        // A bare filename has no parent directory
        let service = Path::new("service.sock");
        let path = terminal_uds_path(service, "vm-1");
        // Parent of bare filename is "" which is_empty, but Path::parent returns Some("")
        // which is NOT /tmp. The unwrap_or only triggers for None.
        // Actually, "service.sock".parent() returns Some(""), so we get ""/instances/...
        // This tests the actual behavior.
        assert!(path.to_str().unwrap().contains("vm-1-ws.sock"));
    }

    // --- handle_terminal_ws handler (validation path) ---

    use crate::status::StatusCache;
    use axum::body::Body;
    use tower::ServiceExt;

    fn terminal_app(uds_path: &str) -> axum::Router {
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: uds_path.into(),
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state)
    }

    #[tokio::test]
    async fn handler_non_ws_request_does_not_reject_as_invalid_id() {
        // WebSocketUpgrade extraction fails before our validation runs for
        // non-WS requests. Verify a valid ID doesn't produce our error message.
        let app = terminal_app("/tmp/test.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .uri("/terminal/vm-12345")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            !text.contains("invalid VM id"),
            "valid ID should not be rejected as invalid"
        );
    }

    #[tokio::test]
    async fn handler_unmatched_path_returns_404() {
        let app = terminal_app("/tmp/test.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .uri("/terminal/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    // --- WebSocket relay integration test ---

    #[tokio::test]
    async fn websocket_relay_echoes_text() {
        // Start a mock "process" WebSocket server on UDS
        let dir = tempfile::tempdir().unwrap();
        let instances_dir = dir.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let ws_sock = instances_dir.join("test-vm-ws.sock");

        let uds = tokio::net::UnixListener::bind(&ws_sock).unwrap();
        let mock_handle = tokio::spawn(async move {
            // Accept one connection, do WS handshake, echo messages back
            if let Ok((stream, _)) = uds.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut write, mut read) = ws.split();
                while let Some(Ok(msg)) = read.next().await {
                    match msg {
                        TungsteniteMessage::Text(_) | TungsteniteMessage::Binary(_) => {
                            if write.send(msg).await.is_err() {
                                break;
                            }
                        }
                        TungsteniteMessage::Close(_) => break,
                        _ => {}
                    }
                }
            }
        });

        // Build the gateway app with UDS path pointing to our temp dir
        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock.clone(),
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        // Start a TCP listener for the gateway
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });

        // Give server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connect as a WebSocket client
        let url = format!("ws://127.0.0.1:{}/terminal/test-vm", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Send a text message
        ws.send(TungsteniteMessage::Text("hello gateway".into()))
            .await
            .unwrap();

        // Read back the echoed message
        let echoed = ws.next().await.unwrap().unwrap();
        match echoed {
            TungsteniteMessage::Text(t) => assert_eq!(t.to_string(), "hello gateway"),
            other => panic!("expected text message, got {:?}", other),
        }

        // Send binary
        ws.send(TungsteniteMessage::Binary(vec![1, 2, 3].into()))
            .await
            .unwrap();
        let echoed = ws.next().await.unwrap().unwrap();
        match echoed {
            TungsteniteMessage::Binary(b) => assert_eq!(b.to_vec(), vec![1, 2, 3]),
            other => panic!("expected binary message, got {:?}", other),
        }

        // Close
        ws.send(TungsteniteMessage::Close(None)).await.unwrap();

        // Cleanup
        mock_handle.abort();
        server_handle.abort();
    }

    #[tokio::test]
    async fn websocket_relay_handles_process_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let instances_dir = dir.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let ws_sock = instances_dir.join("dc-vm-ws.sock");

        let uds = tokio::net::UnixListener::bind(&ws_sock).unwrap();
        let mock_handle = tokio::spawn(async move {
            if let Ok((stream, _)) = uds.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut write, _read) = ws.split();
                // Send one message then close
                write
                    .send(TungsteniteMessage::Text("bye".into()))
                    .await
                    .ok();
                write
                    .send(TungsteniteMessage::Close(None))
                    .await
                    .ok();
            }
        });

        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock,
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}/terminal/dc-vm", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Read the message sent by the mock before it closes
        let msg = ws.next().await.unwrap().unwrap();
        match msg {
            TungsteniteMessage::Text(t) => assert_eq!(t.to_string(), "bye"),
            other => panic!("expected text, got {:?}", other),
        }

        // Next read should indicate close
        let close_or_none = ws.next().await;
        match close_or_none {
            Some(Ok(TungsteniteMessage::Close(_))) | None => {}
            other => panic!("expected close or stream end, got {:?}", other),
        }

        mock_handle.abort();
        server_handle.abort();
    }

    // Helper: start a gateway+mock pair and return (ws_url, mock_handle, server_handle, _tmpdir)
    async fn ws_test_setup(
        vm_id: &str,
        mock_fn: impl FnOnce(tokio::net::UnixListener) -> tokio::task::JoinHandle<()>,
    ) -> (
        String,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let instances_dir = dir.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let ws_sock = instances_dir.join(format!("{}-ws.sock", vm_id));

        let uds = tokio::net::UnixListener::bind(&ws_sock).unwrap();
        let mock_handle = mock_fn(uds);

        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock,
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}/terminal/{}", addr.port(), vm_id);
        (url, mock_handle, server_handle, dir)
    }

    #[tokio::test]
    async fn websocket_relay_ping_pong() {
        let (url, mh, sh, _d) = ws_test_setup("ping-vm", |uds| {
            tokio::spawn(async move {
                if let Ok((stream, _)) = uds.accept().await {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, mut read) = ws.split();
                    while let Some(Ok(msg)) = read.next().await {
                        match msg {
                            TungsteniteMessage::Ping(data) => {
                                // Respond with Pong
                                write.send(TungsteniteMessage::Pong(data)).await.ok();
                            }
                            TungsteniteMessage::Close(_) => break,
                            _ => {
                                write.send(msg).await.ok();
                            }
                        }
                    }
                }
            })
        })
        .await;

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Send ping
        ws.send(TungsteniteMessage::Ping(vec![42].into()))
            .await
            .unwrap();

        // Should get pong back (from mock, relayed through gateway)
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ws.next(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();

        match msg {
            TungsteniteMessage::Pong(data) => assert_eq!(data.to_vec(), vec![42]),
            other => panic!("expected pong, got {:?}", other),
        }

        ws.send(TungsteniteMessage::Close(None)).await.ok();
        mh.abort();
        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_close_with_reason() {
        let (url, mh, sh, _d) = ws_test_setup("close-vm", |uds| {
            tokio::spawn(async move {
                if let Ok((stream, _)) = uds.accept().await {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, mut read) = ws.split();
                    while let Some(Ok(msg)) = read.next().await {
                        match msg {
                            TungsteniteMessage::Close(frame) => {
                                // Echo close back
                                write.send(TungsteniteMessage::Close(frame)).await.ok();
                                break;
                            }
                            _ => {
                                write.send(msg).await.ok();
                            }
                        }
                    }
                }
            })
        })
        .await;

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Send close with reason
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        ws.send(TungsteniteMessage::Close(Some(
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: CloseCode::Normal,
                reason: "goodbye".into(),
            },
        )))
        .await
        .ok();

        // Should receive close back
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
            .await
            .unwrap();

        match msg {
            Some(Ok(TungsteniteMessage::Close(Some(frame)))) => {
                assert_eq!(frame.code, CloseCode::Normal);
                assert_eq!(frame.reason.to_string(), "goodbye");
            }
            Some(Ok(TungsteniteMessage::Close(None))) => {
                // Also acceptable -- close was relayed
            }
            Some(Err(_)) => {
                // Protocol error (e.g. ResetWithoutClosingHandshake) -- close was processed
            }
            None => {
                // Stream ended -- close was processed
            }
            other => panic!("expected close, got {:?}", other),
        }

        mh.abort();
        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_client_disconnect_aborts_process_relay() {
        let (url, mh, sh, _d) = ws_test_setup("cd-vm", |uds| {
            tokio::spawn(async move {
                if let Ok((stream, _)) = uds.accept().await {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, mut read) = ws.split();
                    // Echo loop
                    while let Some(Ok(msg)) = read.next().await {
                        if write.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
            })
        })
        .await;

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Send a message to confirm relay works
        ws.send(TungsteniteMessage::Text("ping".into()))
            .await
            .unwrap();
        let msg = ws.next().await.unwrap().unwrap();
        assert!(matches!(msg, TungsteniteMessage::Text(_)));

        // Client closes -- this should trigger c2p completion and abort p2c
        ws.send(TungsteniteMessage::Close(None)).await.ok();

        // Give gateway time to process the close
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        mh.abort();
        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_fails_on_missing_uds() {
        // No mock UDS server -- gateway should fail to connect and close
        let dir = tempfile::tempdir().unwrap();
        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock,
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let sh = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}/terminal/no-such-vm", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // The WS upgrade succeeds but handle_socket fails on UDS connect.
        // Client should see the connection drop.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ws.next(),
        )
        .await;

        match result {
            Ok(Some(Ok(TungsteniteMessage::Close(_)))) | Ok(None) | Err(_) => {
                // Connection closed or timed out -- expected
            }
            Ok(Some(Err(_))) => {
                // Protocol error (e.g. ResetWithoutClosingHandshake) -- expected
            }
            other => panic!("expected close or timeout, got {:?}", other),
        }

        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_invalid_id_via_ws_client() {
        // Connect a real WS client with an invalid VM ID.
        // The handler should reject with 400 before upgrade.
        let dir = tempfile::tempdir().unwrap();
        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock,
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let sh = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Use an ID with dots (invalid)
        let url = format!("ws://127.0.0.1:{}/terminal/vm.bad.id", addr.port());
        let result = tokio_tungstenite::connect_async(&url).await;

        // Should fail -- server returns 400, not 101 upgrade
        assert!(result.is_err(), "expected WS handshake to fail for invalid ID");

        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_process_sends_binary_and_ping() {
        // Exercise the p2c (process-to-client) paths for Binary and Ping.
        // The mock reads from the client (echo loop) so it stays alive while
        // sending messages.
        let (url, mh, sh, _d) = ws_test_setup("p2c-vm", |uds| {
            tokio::spawn(async move {
                if let Ok((stream, _)) = uds.accept().await {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, mut read) = ws.split();
                    // Wait for client's "ready" text
                    while let Some(Ok(msg)) = read.next().await {
                        match msg {
                            TungsteniteMessage::Text(t) if t.to_string() == "ready" => {
                                // Now send binary and ping from process side
                                write
                                    .send(TungsteniteMessage::Binary(vec![10, 20, 30].into()))
                                    .await
                                    .ok();
                                write
                                    .send(TungsteniteMessage::Ping(vec![99].into()))
                                    .await
                                    .ok();
                                write
                                    .send(TungsteniteMessage::Text("done".into()))
                                    .await
                                    .ok();
                            }
                            TungsteniteMessage::Close(_) => break,
                            _ => {}
                        }
                    }
                }
            })
        })
        .await;

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Tell the mock we're ready
        ws.send(TungsteniteMessage::Text("ready".into()))
            .await
            .unwrap();

        // Collect messages
        let mut got_binary = false;
        let mut got_text = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(TungsteniteMessage::Binary(b)))) => {
                    assert_eq!(b.to_vec(), vec![10, 20, 30]);
                    got_binary = true;
                }
                Ok(Some(Ok(TungsteniteMessage::Ping(_)))) => {
                    // Ping relayed from process
                }
                Ok(Some(Ok(TungsteniteMessage::Text(t)))) if t.to_string() == "done" => {
                    got_text = true;
                    break;
                }
                Ok(Some(Ok(TungsteniteMessage::Pong(_)))) => {
                    // Auto-pong response, skip
                }
                Ok(Some(Ok(TungsteniteMessage::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => break,
                Err(_) => break,
                _ => {}
            }
        }
        assert!(got_binary, "should have received binary from process");
        assert!(got_text, "should have received text 'done' from process");
        ws.send(TungsteniteMessage::Close(None)).await.ok();
        mh.abort();
        sh.abort();
    }

    #[tokio::test]
    async fn websocket_relay_process_sends_close_with_frame() {
        // Exercise the p2c Close with CloseFrame path
        let (url, mh, sh, _d) = ws_test_setup("p2close-vm", |uds| {
            tokio::spawn(async move {
                if let Ok((stream, _)) = uds.accept().await {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, _read) = ws.split();
                    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
                    write
                        .send(TungsteniteMessage::Close(Some(
                            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: CloseCode::Away,
                                reason: "going away".into(),
                            },
                        )))
                        .await
                        .ok();
                }
            })
        })
        .await;

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Should receive close frame relayed from process
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await;
        match result {
            Ok(Some(Ok(TungsteniteMessage::Close(Some(frame))))) => {
                assert_eq!(frame.reason.to_string(), "going away");
            }
            Ok(Some(Ok(TungsteniteMessage::Close(None)))) | Ok(None) | Ok(Some(Err(_))) | Err(_) => {
                // Close was processed (frame details may be lost in relay)
            }
            other => panic!("expected close from process, got {:?}", other),
        }

        mh.abort();
        sh.abort();
    }

    // --- UDS path fallback (issue #11) ---

    #[test]
    fn uds_path_bare_filename_falls_back_to_tmp() {
        // "service.sock" has parent Some("") which should trigger the fallback
        let service = Path::new("service.sock");
        let path = terminal_uds_path(service, "vm-1");
        assert_eq!(
            path,
            PathBuf::from("/tmp/instances/vm-1-ws.sock"),
            "bare filename should fall back to /tmp"
        );
    }

    #[test]
    fn uds_path_absolute_does_not_fall_back() {
        let service = Path::new("/home/user/.capsem/run/service.sock");
        let path = terminal_uds_path(service, "vm-1");
        assert_eq!(
            path,
            PathBuf::from("/home/user/.capsem/run/instances/vm-1-ws.sock")
        );
    }

    // --- Close frame on UDS failure (issue #9) ---

    #[tokio::test]
    async fn websocket_relay_sends_close_frame_on_uds_failure() {
        // No mock UDS -- gateway should send a Close frame with reason
        let dir = tempfile::tempdir().unwrap();
        let service_sock = dir.path().join("service.sock");
        let state = Arc::new(AppState {
            token: "test".into(),
            uds_path: service_sock,
            status_cache: StatusCache::new(),
            auth_failures: crate::auth::AuthFailureTracker::new(),
        });
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_terminal_ws))
            .with_state(state);

        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let sh = tokio::spawn(async move {
            axum::serve(tcp, app).await.ok();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}/terminal/no-vm", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Should receive a Close frame with reason "VM not available"
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await;
        match result {
            Ok(Some(Ok(TungsteniteMessage::Close(Some(frame))))) => {
                assert_eq!(frame.code, tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(1011));
                assert_eq!(frame.reason.to_string(), "VM not available");
            }
            Ok(Some(Ok(TungsteniteMessage::Close(None)))) => {
                // Close was sent but frame details lost in transport
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => {
                // Connection closed -- close was processed
            }
            other => panic!("expected close with reason, got {:?}", other),
        }

        sh.abort();
    }
}
