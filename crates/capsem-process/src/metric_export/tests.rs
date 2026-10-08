use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use opentelemetry_http::HttpClient as _;

use super::*;

type Received = (String, HeaderMap, Bytes);

async fn receive(
    Path(id): Path<String>,
    State(received): State<Arc<Mutex<Vec<Received>>>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, [(&'static str, &'static str); 1], Bytes) {
    received.lock().unwrap().push((id, headers, body));
    (
        StatusCode::ACCEPTED,
        [("x-collector", "accepted")],
        Bytes::from_static(b"ok"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broker_client_uses_only_the_fixed_local_route_and_protobuf_body() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("service.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/internal/vms/{id}/metrics", post(receive))
        .with_state(Arc::clone(&received));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = ServiceMetricClient::new(socket, "vm-a").unwrap();
    let request = http::Request::post("https://attacker.example/v1/metrics")
        .header(http::header::CONTENT_TYPE, "text/plain")
        .header(http::header::AUTHORIZATION, "Bearer must-not-cross")
        .body(Bytes::from_static(b"encoded otlp"))
        .unwrap();

    let response = tokio::task::spawn_blocking(move || futures::executor::block_on(client.send_bytes(request)))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers().get("x-collector").unwrap(), "accepted");
    assert_eq!(response.body(), b"ok".as_slice());
    let (id, headers, body) = received.lock().unwrap().pop().unwrap();
    assert_eq!(id, "vm-a");
    assert_eq!(
        headers.get(http::header::CONTENT_TYPE).unwrap(),
        "application/x-protobuf"
    );
    assert!(!headers.contains_key(http::header::AUTHORIZATION));
    assert_eq!(body, b"encoded otlp".as_slice());
    server.abort();
}
