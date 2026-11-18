use super::*;
use axum::http::Request;
use serde_json::json;

const ENVIRONMENT: &str = "00000000-0000-4000-8000-000000000001";
const REVISION: &str = "00000000-0000-4000-8000-000000000002";
fn selected() -> serde_json::Value {
    json!({"environment_id": ENVIRONMENT, "revision_id": REVISION})
}
fn timing_path() -> String {
    format!("/environments/{ENVIRONMENT}/revisions/{REVISION}/readiness")
}
fn create(body: String) -> Request<Body> {
    Request::post("/vms/create")
        .header("authorization", "Bearer selected")
        .header("x-capsem-actor", "forged")
        .header("x-capsem-gateway", "forged")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn saved_create_uses_exact_authenticated_timing_before_a_single_mutation() {
    for outcome in ["verified", "changed", "refused", "overflow", "oversized"] {
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let upstream = Router::new().fallback(move |request: Request<Body>| {
            let sent = sent.clone();
            async move {
                let path = request.uri().path().to_owned();
                assert_eq!(request.headers()["authorization"], "Bearer selected");
                assert!(!request.headers().contains_key("x-capsem-actor"));
                assert!(!request.headers().contains_key("x-capsem-gateway"));
                let body = axum::body::to_bytes(request.into_body(), MAX_BODY_SIZE).await.unwrap();
                sent.send((path.clone(), body)).unwrap();
                if path.ends_with("/readiness") {
                    if outcome == "refused" {
                        return (StatusCode::FORBIDDEN, axum::Json(json!({"error":"inspection refused"})))
                            .into_response();
                    }
                    if outcome == "oversized" {
                        return axum::Json(json!({"padding":"x".repeat(8192)})).into_response();
                    }
                    let mut environment = selected();
                    if outcome == "changed" {
                        environment["revision_id"] = json!("00000000-0000-4000-8000-000000000009");
                    }
                    return axum::Json(json!({"environment": environment,
                        "startup_timeout_seconds": if outcome == "overflow" { u64::from(u32::MAX) + 1 } else { 600 }}))
                    .into_response();
                }
                assert_eq!(path, "/vms/create");
                axum::Json(json!({"id":"one-vm", "status":"Running"})).into_response()
            }
        });
        let (socket, server, _directory) = mock_uds(upstream).await;
        let body = json!({"environment":selected(), "name":"saved", "labels":{"mode":"retained"}}).to_string();
        let response = proxy_app(&socket).oneshot(create(body.clone())).await.unwrap();
        assert_eq!(
            response.status(),
            match outcome {
                "verified" => StatusCode::OK,
                "refused" => StatusCode::FORBIDDEN,
                _ => StatusCode::BAD_GATEWAY,
            }
        );
        let (path, metadata_body) = received.try_recv().unwrap();
        assert_eq!(path, timing_path());
        assert!(metadata_body.is_empty());
        if outcome == "verified" {
            let (path, forwarded) = received.try_recv().unwrap();
            assert_eq!(path, "/vms/create");
            assert_eq!(forwarded, body);
        }
        assert!(received.try_recv().is_err(), "refusal cannot allocate or replay a VM");
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn saved_create_waits_through_approved_startup_beyond_the_old_gateway_deadline() {
    let received = Arc::new(tokio::sync::Notify::new());
    let accepted = received.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let upstream = Router::new().fallback(move |request: Request<Body>| {
        let accepted = accepted.clone();
        let calls = observed.clone();
        async move {
            if request.uri().path().ends_with("/readiness") {
                return axum::Json(json!({"environment":selected(), "startup_timeout_seconds":600}));
            }
            assert_eq!(request.uri().path(), "/vms/create");
            calls.fetch_add(1, Ordering::SeqCst);
            accepted.notify_one();
            tokio::time::sleep(Duration::from_secs(600)).await;
            axum::Json(json!({"id":"ready-once", "status":"Running"}))
        }
    });
    let (socket, server, _directory) = mock_uds(upstream).await;
    let pending = tokio::spawn(proxy_app(&socket).oneshot(create(json!({"environment":selected()}).to_string())));
    tokio::time::timeout(Duration::from_secs(5), received.notified())
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(600)).await;
    let response = pending.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"],
        "ready-once"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn ordinary_attached_and_invalid_creates_keep_their_original_body_without_timing_reads() {
    for body in [
        json!({}).to_string(),
        json!({"environment":selected(), "attach":true}).to_string(),
        json!({"container":{"image":"docker://busybox", "attach":false}}).to_string(),
        json!({"environment":{"environment_id":"invalid", "revision_id":REVISION}}).to_string(),
        "malformed-json".to_owned(),
    ] {
        let expected = body.clone();
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let upstream = Router::new().fallback(move |request: Request<Body>| {
            let sent = sent.clone();
            async move {
                assert_eq!(request.uri().path(), "/vms/create");
                let bytes = axum::body::to_bytes(request.into_body(), MAX_BODY_SIZE).await.unwrap();
                sent.send(bytes).unwrap();
                (StatusCode::OK, "authoritative reply")
            }
        });
        let (socket, server, _directory) = mock_uds(upstream).await;
        let response = proxy_app(&socket).oneshot(create(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(received.try_recv().unwrap(), expected);
        assert!(received.try_recv().is_err());
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn excessive_create_bytes_are_refused_before_planning_even_with_declared_length() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let upstream = Router::new().fallback(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    });
    let (socket, server, _directory) = mock_uds(upstream).await;
    for sized in [false, true] {
        let mut request = Request::post("/vms/create");
        if sized {
            request = request.header("content-length", "1");
        }
        let response = proxy_app(&socket)
            .oneshot(request.body(Body::from(vec![b'x'; MAX_BODY_SIZE + 1])).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn stalled_timing_headers_or_body_are_bounded_before_any_creation() {
    for stall_body in [false, true] {
        let received = Arc::new(tokio::sync::Notify::new());
        let entered = received.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let upstream = Router::new().fallback(move |request: Request<Body>| {
            let entered = entered.clone();
            let calls = observed.clone();
            async move {
                assert_eq!(request.uri().path(), timing_path());
                calls.fetch_add(1, Ordering::SeqCst);
                entered.notify_one();
                if !stall_body {
                    std::future::pending::<()>().await;
                }
                axum::response::Response::new(Body::from_stream(futures::stream::pending::<
                    Result<Bytes, std::io::Error>,
                >()))
            }
        });
        let (socket, server, _directory) = mock_uds(upstream).await;
        let pending = tokio::spawn(proxy_app(&socket).oneshot(create(json!({"environment":selected()}).to_string())));
        tokio::time::timeout(Duration::from_secs(5), received.notified())
            .await
            .unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        assert_eq!(pending.await.unwrap().unwrap().status(), StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
        tokio::time::resume();
    }
}
