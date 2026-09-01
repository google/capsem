//! Real HTTP-over-UDS boundary tests for the MCP service client.

use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

struct MockService {
    client: UdsClient,
    request: Option<tokio::sync::oneshot::Receiver<(String, String, Vec<u8>)>>,
    task: tokio::task::JoinHandle<()>,
    path: PathBuf,
}

impl Drop for MockService {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn mock_service(status: hyper::StatusCode, body: &'static str) -> MockService {
    let path = std::env::temp_dir().join(format!(
        "capsem-mcp-test-{}-{}.sock",
        std::process::id(),
        NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (request_tx, request) = tokio::sync::oneshot::channel();
    let request_tx = Arc::new(std::sync::Mutex::new(Some(request_tx)));
    let task = tokio::spawn({
        let request_tx = Arc::clone(&request_tx);
        async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                let request_tx = Arc::clone(&request_tx);
                async move {
                    let method = request.method().to_string();
                    let path = request
                        .uri()
                        .path_and_query()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let payload = request.into_body().collect().await.unwrap().to_bytes().to_vec();
                    if let Some(tx) = request_tx.lock().unwrap().take() {
                        let _ = tx.send((method, path, payload));
                    }
                    Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(status)
                            .body(Full::new(Bytes::from_static(body.as_bytes())))
                            .unwrap(),
                    )
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        }
    });
    MockService {
        client: UdsClient::new(path.clone()),
        request: Some(request),
        task,
        path,
    }
}

#[tokio::test]
async fn uds_json_request_preserves_method_path_and_body() {
    let mut service = mock_service(hyper::StatusCode::OK, r#"{"accepted":true}"#).await;
    let response: Value = service
        .client
        .request("POST", "/vms/create?source=mcp", Some(json!({"name": "sandbox"})))
        .await
        .unwrap();
    assert_eq!(response, json!({"accepted": true}));

    let (method, path, body) = service.request.take().unwrap().await.unwrap();
    assert_eq!(method, "POST");
    assert_eq!(path, "/vms/create?source=mcp");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"name": "sandbox"})
    );
}

#[tokio::test]
async fn uds_json_request_surfaces_structured_service_errors() {
    let service = mock_service(hyper::StatusCode::CONFLICT, r#"{"error":"name already exists"}"#).await;
    let error = service
        .client
        .request::<Value, Value>("GET", "/vms/duplicate", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("409 Conflict"), "unexpected error: {error}");
    assert!(error.contains("name already exists"), "unexpected error: {error}");
}

#[tokio::test]
async fn uds_json_request_rejects_invalid_success_payloads() {
    let service = mock_service(hyper::StatusCode::OK, "not-json").await;
    let error = service
        .client
        .request::<Value, Value>("GET", "/vms/list", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected ident"), "unexpected error: {error}");
}

#[tokio::test]
async fn uds_text_request_accepts_plain_text_and_rejects_plain_errors() {
    let service = mock_service(hyper::StatusCode::OK, "first\nsecond\n").await;
    assert_eq!(
        service.client.request_text("GET", "/service-logs").await.unwrap(),
        "first\nsecond\n"
    );

    let service = mock_service(hyper::StatusCode::BAD_GATEWAY, "upstream unavailable").await;
    let error = service
        .client
        .request_text("GET", "/service-logs")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("502 Bad Gateway"), "unexpected error: {error}");
    assert!(error.contains("upstream unavailable"), "unexpected error: {error}");
}

#[tokio::test]
async fn session_route_resolution_prefers_id_then_name_and_rejects_unknown() {
    let listing = r#"{"sandboxes":[{"id":"vm-7","name":"build"}]}"#;
    let service = mock_service(hyper::StatusCode::OK, listing).await;
    assert_eq!(resolve_session_route_id(&service.client, "vm-7").await.unwrap(), "vm-7");

    let service = mock_service(hyper::StatusCode::OK, listing).await;
    assert_eq!(
        resolve_session_route_id(&service.client, "build").await.unwrap(),
        "vm-7"
    );

    let service = mock_service(hyper::StatusCode::OK, listing).await;
    let error = resolve_session_route_id(&service.client, "missing").await.unwrap_err();
    assert_eq!(error, "unknown session name or id: missing");
}

#[tokio::test]
async fn handler_list_uses_the_production_uds_client_and_response_formatter() {
    let mut service = mock_service(hyper::StatusCode::OK, r#"{"sandboxes":[]}"#).await;
    let handler = CapsemHandler {
        client: Arc::new(UdsClient::new(service.client.uds_path.clone())),
    };
    let rendered = handler.list().await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&rendered).unwrap(),
        json!({"sandboxes": []})
    );
    let (method, path, body) = service.request.take().unwrap().await.unwrap();
    assert_eq!((method.as_str(), path.as_str()), ("GET", "/vms/list"));
    assert!(body.is_empty());
}
