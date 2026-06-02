//! Tests for `proxy` (extracted from inline `mod tests`).

use super::*;
use axum::body::Body;
use axum::Router;
use bytes::Bytes;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

use crate::status::StatusCache;

fn counter_value(snapshotter: &Snapshotter, metric: &str, labels: &[(&str, &str)]) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            if key.key().name() != metric {
                return None;
            }
            let has_labels = labels.iter().all(|(want_key, want_value)| {
                key.key()
                    .labels()
                    .any(|label| label.key() == *want_key && label.value() == *want_value)
            });
            if !has_labels {
                return None;
            }
            match value {
                DebugValue::Counter(count) => Some(count),
                _ => None,
            }
        })
        .sum()
}

fn proxy_app(uds_path: &str) -> Router {
    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: uds_path.into(),
        status_cache: StatusCache::new(),
        auth_failures: crate::auth::AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    Router::new().fallback(handle_proxy).with_state(state)
}

/// Start a mock UDS server with the given router, return (sock_path, join_handle, tempdir).
async fn mock_uds(app: axum::Router) -> (String, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("mock.sock");
    let path_str = sock_path.to_str().unwrap().to_string();
    let uds = tokio::net::UnixListener::bind(&sock_path).unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(uds, app).await.ok();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (path_str, handle, dir)
}

async fn status_of(app: Router, method: &str, uri: &str) -> StatusCode {
    app.oneshot(
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

// --- 502 when UDS unavailable ---

#[tokio::test]
async fn returns_502_when_uds_missing() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let app = proxy_app("/tmp/capsem-gw-test-nonexistent.sock");
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "service unavailable");
    assert_eq!(
        counter_value(
            &snapshotter,
            crate::metrics::PROXY_REQUESTS_TOTAL,
            &[
                ("endpoint", "list"),
                ("method", "GET"),
                ("status_class", "5xx")
            ],
        ),
        1
    );
}

#[tokio::test]
async fn returns_502_for_post_when_uds_missing() {
    let app = proxy_app("/tmp/capsem-gw-test-nonexistent.sock");
    assert_eq!(
        status_of(app, "POST", "/provision").await,
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn returns_502_for_delete_when_uds_missing() {
    let app = proxy_app("/tmp/capsem-gw-test-nonexistent.sock");
    assert_eq!(
        status_of(app, "DELETE", "/delete/abc").await,
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn returns_502_when_uds_exists_but_closed() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("closed.sock");
    // Bind then immediately drop to create a stale socket file
    let _ = tokio::net::UnixListener::bind(&sock_path).unwrap();
    // Drop the listener -- socket file exists but nobody is listening
    drop(std::fs::File::open(&sock_path)); // keep file alive via dir
    let app = proxy_app(sock_path.to_str().unwrap());
    assert_eq!(
        status_of(app, "GET", "/list").await,
        StatusCode::BAD_GATEWAY
    );
}

// --- Forwarding: basic ---

#[tokio::test]
async fn forwards_get_to_uds() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let mock = axum::Router::new().route(
        "/list",
        axum::routing::get(|| async { axum::Json(serde_json::json!({"sandboxes": []})) }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["sandboxes"], serde_json::json!([]));
    assert_eq!(
        counter_value(
            &snapshotter,
            crate::metrics::PROXY_REQUESTS_TOTAL,
            &[
                ("endpoint", "list"),
                ("method", "GET"),
                ("status_class", "2xx")
            ],
        ),
        1
    );
    h.abort();
}

#[tokio::test]
async fn forwards_post_with_body() {
    let mock = axum::Router::new().route(
        "/echo",
        axum::routing::post(|body: Bytes| async move { body }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "application/json")
                .body(Body::from("hello-gateway"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"hello-gateway");
    h.abort();
}

// --- Forwarding: HTTP methods ---

#[tokio::test]
async fn forwards_put_request() {
    let mock = axum::Router::new().route(
        "/item",
        axum::routing::put(|body: Bytes| async move { body }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/item")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"updated":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.starts_with(b"{"));
    h.abort();
}

#[tokio::test]
async fn forwards_patch_request() {
    let mock = axum::Router::new().route("/item", axum::routing::patch(|| async { "patched" }));
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    assert_eq!(status_of(app, "PATCH", "/item").await, StatusCode::OK);
    h.abort();
}

#[tokio::test]
async fn forwards_head_request() {
    let mock =
        axum::Router::new().route("/health", axum::routing::head(|| async { StatusCode::OK }));
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("HEAD")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // HEAD must not have a body
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.is_empty());
    h.abort();
}

#[tokio::test]
async fn forwards_empty_body_post() {
    let mock = axum::Router::new().route(
        "/empty",
        axum::routing::post(|body: Bytes| async move { format!("len={}", body.len()) }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/empty")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"len=0");
    h.abort();
}

#[tokio::test]
async fn forwards_binary_body() {
    let mock = axum::Router::new().route(
        "/bin",
        axum::routing::post(|body: Bytes| async move { body }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let binary_data: Vec<u8> = vec![0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe];
    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/bin")
                .body(Body::from(binary_data.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.to_vec(), binary_data);
    h.abort();
}

// --- Headers ---

#[tokio::test]
async fn preserves_upstream_response_headers() {
    let mock = axum::Router::new().route(
        "/custom",
        axum::routing::get(|| async {
            (
                [
                    (
                        http::header::HeaderName::from_static("x-custom"),
                        "test-value",
                    ),
                    (
                        http::header::HeaderName::from_static("x-request-id"),
                        "abc-123",
                    ),
                ],
                "ok",
            )
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/custom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.headers().get("x-custom").unwrap(), "test-value");
    assert_eq!(resp.headers().get("x-request-id").unwrap(), "abc-123");
    h.abort();
}

#[tokio::test]
async fn preserves_client_headers_except_auth_and_host() {
    let mock = axum::Router::new().route(
        "/headers",
        axum::routing::get(|req: axum::extract::Request| async move {
            let has_accept = req.headers().contains_key("accept");
            let has_x_custom = req.headers().contains_key("x-custom");
            let has_auth = req.headers().contains_key("authorization");
            let has_host = req.headers().contains_key("host");
            axum::Json(serde_json::json!({
                "has_accept": has_accept,
                "has_x_custom": has_x_custom,
                "has_auth": has_auth,
                "has_host": has_host,
            }))
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/headers")
                .header("accept", "application/json")
                .header("x-custom", "should-be-preserved")
                .header("authorization", "Bearer test-token")
                .header("host", "example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["has_accept"], true);
    assert_eq!(json["has_x_custom"], true);
    assert_eq!(json["has_auth"], false);
    assert_eq!(json["has_host"], false);
    h.abort();
}

// --- Status codes ---

#[tokio::test]
async fn preserves_status_codes() {
    let mock = axum::Router::new()
        .route("/ok", axum::routing::get(|| async { StatusCode::OK }))
        .route(
            "/created",
            axum::routing::post(|| async { StatusCode::CREATED }),
        )
        .route(
            "/bad",
            axum::routing::get(|| async { StatusCode::BAD_REQUEST }),
        )
        .route(
            "/err",
            axum::routing::get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        )
        .route(
            "/unavail",
            axum::routing::get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        );
    let (path, h, _d) = mock_uds(mock).await;

    for (method, uri, expected) in [
        ("GET", "/ok", StatusCode::OK),
        ("POST", "/created", StatusCode::CREATED),
        ("GET", "/bad", StatusCode::BAD_REQUEST),
        ("GET", "/err", StatusCode::INTERNAL_SERVER_ERROR),
        ("GET", "/unavail", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let app = proxy_app(&path);
        assert_eq!(
            status_of(app, method, uri).await,
            expected,
            "expected {expected} for {method} {uri}"
        );
    }
    h.abort();
}

// --- Query strings ---

#[tokio::test]
async fn preserves_query_string() {
    let mock = axum::Router::new().route(
        "/search",
        axum::routing::get(|req: axum::extract::Request| async move {
            req.uri().query().unwrap_or("").to_string()
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/search?q=test&limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "q=test&limit=10");
    h.abort();
}

#[tokio::test]
async fn handles_encoded_query_values() {
    let mock = axum::Router::new().route(
        "/search",
        axum::routing::get(|req: axum::extract::Request| async move {
            req.uri().query().unwrap_or("").to_string()
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/search?name=foo%20bar&special=%26%3D")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let qs = std::str::from_utf8(&body).unwrap();
    assert!(
        qs.contains("foo%20bar"),
        "encoded space not preserved: {qs}"
    );
    assert!(
        qs.contains("%26%3D"),
        "encoded special chars not preserved: {qs}"
    );
    h.abort();
}

// --- Body size limit ---

#[tokio::test]
async fn rejects_oversized_body() {
    let mock = axum::Router::new().route("/big", axum::routing::post(|| async { "ok" }));
    let (path, h, _d) = mock_uds(mock).await;

    let oversized = vec![b'x'; MAX_BODY_SIZE + 1];
    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/big")
                .header("content-length", oversized.len().to_string())
                .body(Body::from(oversized))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "request body too large");
    h.abort();
}

#[tokio::test]
async fn accepts_body_under_limit() {
    let mock = axum::Router::new().route(
        "/big",
        axum::routing::post(|body: Bytes| async move { format!("len={}", body.len()) }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    // 1 MB is well under the 10 MB limit
    let under_limit = vec![b'x'; 1024 * 1024];
    let app = proxy_app(&path);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/big")
                .body(Body::from(under_limit))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "len=1048576");
    h.abort();
}

// --- Concurrency ---

#[tokio::test]
async fn concurrent_proxy_requests() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let mock = axum::Router::new().route(
        "/count",
        axum::routing::get(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                format!("{n}")
            }
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let futs: Vec<_> = (0..10)
        .map(|_| {
            let app = proxy_app(&path);
            async move {
                app.oneshot(
                    axum::http::Request::builder()
                        .uri("/count")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        })
        .collect();
    let results = futures::future::join_all(futs).await;
    assert!(results.iter().all(|s| *s == StatusCode::OK));
    assert_eq!(counter.load(Ordering::SeqCst), 10);
    h.abort();
}

// --- Timeout constants (issues #2, #10) ---

#[test]
fn request_timeout_covers_suspend_operation() {
    // Suspend: up to 10s quiescence + 15s wait + 0.5s cleanup = ~26s
    assert!(
        REQUEST_TIMEOUT >= Duration::from_secs(30),
        "proxy timeout must exceed worst-case suspend duration"
    );
}

#[test]
fn conn_driver_timeout_is_bounded() {
    assert!(
        CONN_DRIVER_TIMEOUT <= Duration::from_secs(600),
        "driver timeout should not be excessive"
    );
    assert!(
        CONN_DRIVER_TIMEOUT > REQUEST_TIMEOUT,
        "driver timeout must exceed request timeout"
    );
}
