use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;

use super::*;

async fn echo(headers: HeaderMap, body: Bytes) -> (StatusCode, [(&'static str, &'static str); 1], Bytes) {
    assert_eq!(
        headers.get(http::header::CONTENT_TYPE).unwrap(),
        "application/x-protobuf"
    );
    (StatusCode::ACCEPTED, [("x-capsem-test", "echo")], body)
}

async fn server(app: Router) -> (tempfile::TempDir, std::path::PathBuf, tokio::task::JoinHandle<()>) {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("service.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (directory, socket, task)
}

#[tokio::test]
async fn post_bytes_preserves_status_headers_and_body() {
    let (_directory, socket, task) = server(Router::new().route("/bytes", post(echo))).await;
    let response = post_bytes(
        &socket,
        "/bytes",
        http::HeaderValue::from_static("application/x-protobuf"),
        Bytes::from_static(b"otlp body"),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers().get("x-capsem-test").unwrap(), "echo");
    assert_eq!(response.body(), b"otlp body".as_slice());
    task.abort();
}

#[tokio::test]
async fn post_bytes_refuses_an_oversized_request_before_connecting() {
    let socket = std::path::Path::new("/a/socket/that/must/not/be-opened");
    let error = post_bytes(
        socket,
        "/bytes",
        http::HeaderValue::from_static("application/x-protobuf"),
        Bytes::from(vec![0; MAX_BODY_BYTES + 1]),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("request body exceeds"), "{error:#}");
}

#[tokio::test]
async fn post_bytes_refuses_a_malformed_path_before_connecting() {
    let socket = std::path::Path::new("/a/socket/that/must/not/be-opened");
    let error = post_bytes(
        socket,
        "/invalid\npath",
        http::HeaderValue::from_static("application/x-protobuf"),
        Bytes::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("build service request"), "{error:#}");
}

#[tokio::test]
async fn post_bytes_stops_collecting_an_oversized_response() {
    let app = Router::new().route("/large", post(|| async { Bytes::from(vec![0; MAX_BODY_BYTES + 1]) }));
    let (_directory, socket, task) = server(app).await;
    let error = post_bytes(
        &socket,
        "/large",
        http::HeaderValue::from_static("application/x-protobuf"),
        Bytes::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("response body exceeds"), "{error:#}");
    task.abort();
}
