use super::*;
use axum::extract::ws::{Message as ServerMessage, WebSocketUpgrade};
use axum::extract::Path;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const TOKEN: &str = "stream-tunnel-token-64chars-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A stand-in service: `/vms/{id}/stream` echoes binary frames back with the
/// VM id prefixed and records the headers the gateway forwarded.
async fn fake_service(
    dir: &std::path::Path,
) -> (
    std::path::PathBuf,
    tokio::sync::mpsc::UnboundedReceiver<http::HeaderMap>,
) {
    let socket = dir.join("service.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (headers_tx, headers_rx) = tokio::sync::mpsc::unbounded_channel();
    let app = axum::Router::new().route(
        "/vms/{id}/stream",
        axum::routing::get(
            move |Path(id): Path<String>, headers: http::HeaderMap, upgrade: WebSocketUpgrade| {
                let headers_tx = headers_tx.clone();
                async move {
                    headers_tx.send(headers).unwrap();
                    upgrade
                        .protocols(["capsem.stream.v1"])
                        .on_upgrade(move |mut socket| async move {
                            while let Some(Ok(ServerMessage::Binary(bytes))) = socket.recv().await {
                                let mut reply = id.as_bytes().to_vec();
                                reply.extend_from_slice(&bytes);
                                if socket.send(ServerMessage::Binary(reply.into())).await.is_err() {
                                    break;
                                }
                            }
                        })
                }
            },
        ),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (socket, headers_rx)
}

async fn gateway(service: &std::path::Path) -> std::net::SocketAddr {
    let state = Arc::new(AppState {
        token: TOKEN.into(),
        uds_path: service.to_path_buf(),
        service_client: ServiceClient::new(service),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    let app = service_proxy_routes()
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware,
        ))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

fn request(address: std::net::SocketAddr, query: &str, vm: &str) -> http::Request<()> {
    let mut request = format!("ws://{address}/vms/{vm}/stream{query}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "capsem.stream.v1".parse().unwrap());
    request
}

#[tokio::test]
async fn authenticated_stream_is_tunnelled_byte_for_byte_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (service, mut forwarded) = fake_service(dir.path()).await;
    let address = gateway(&service).await;
    let (mut client, response) = tokio_tungstenite::connect_async(request(address, &format!("?token={TOKEN}"), "box"))
        .await
        .unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "capsem.stream.v1");
    client.send(Message::Binary(vec![0, 0xff, b'x'].into())).await.unwrap();
    match client.next().await.unwrap().unwrap() {
        Message::Binary(bytes) => assert_eq!(&bytes[..], b"box\x00\xffx"),
        other => panic!("unexpected {other:?}"),
    }
    let headers = forwarded.recv().await.unwrap();
    assert_eq!(headers["sec-websocket-protocol"], "capsem.stream.v1");
    assert_eq!(headers[http::header::HOST], "localhost");
    assert!(headers.get(http::header::AUTHORIZATION).is_none());
    assert!(
        !format!("{headers:?}").contains(TOKEN),
        "the gateway token must not reach the service"
    );
}

#[tokio::test]
async fn stream_without_credentials_never_reaches_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let (service, mut forwarded) = fake_service(dir.path()).await;
    let address = gateway(&service).await;
    for query in ["", "?token=wrong"] {
        let error = tokio_tungstenite::connect_async(request(address, query, "box"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("401"), "{query}: {error}");
    }
    assert!(forwarded.try_recv().is_err());
}

#[tokio::test]
async fn invalid_vm_ids_are_refused_before_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let (service, mut forwarded) = fake_service(dir.path()).await;
    let address = gateway(&service).await;
    let error = tokio_tungstenite::connect_async(request(address, &format!("?token={TOKEN}"), "..%2Fetc"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("400"), "{error}");
    assert!(forwarded.try_recv().is_err());
}

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
    let id = "a".repeat(64);
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
    let id = "a".repeat(65);
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
