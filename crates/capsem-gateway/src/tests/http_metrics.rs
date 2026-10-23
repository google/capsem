use super::*;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

#[tokio::test]
async fn actual_router_observes_auth_cors_and_rejections_with_finite_private_labels() {
    let _trace = HTTP_TRACE_LOCK.lock().await;
    let capture = TraceCapture::default();
    let records = Arc::clone(&capture.records);
    let dispatcher = tracing::Dispatch::new(tracing_subscriber::registry().with(capture));
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    let _local = metrics::set_default_local_recorder(&recorder);
    let (_, state) = health_app("/not-connected/service.sock");
    let app = gateway_router(state);
    for index in 0..100 {
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(format!("PRIVATE-METHOD-{index}").as_str())
                    .uri(format!("/private-path-canary-{index}?token=query-canary"))
                    .header("authorization", "Bearer test")
                    .body(Body::from("body-canary"))
                    .unwrap(),
            )
            .with_subscriber(dispatcher.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
        drop(response);
    }
    for (uri, method, host, status) in [
        ("/health", "GET", "localhost", http::StatusCode::OK),
        (
            "/vms/private-vm-canary/info?token=query-canary",
            "GET",
            "localhost",
            http::StatusCode::UNAUTHORIZED,
        ),
        (
            "/health",
            "GET",
            "foreign-host-canary.invalid",
            http::StatusCode::FORBIDDEN,
        ),
        ("/health", "OPTIONS", "localhost", http::StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri(uri)
                    .method(method)
                    .header("host", host)
                    .header("origin", "http://localhost:19222")
                    .header("access-control-request-method", "GET")
                    .header("authorization", "Bearer header-canary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .with_subscriber(dispatcher.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        drop(response);
    }
    let failed = app
        .oneshot(
            http::Request::builder()
                .uri("/vms/forwarded-private-canary/info?secret=query-canary")
                .header("authorization", "Bearer test")
                .body(Body::empty())
                .unwrap(),
        )
        .with_subscriber(dispatcher)
        .await
        .unwrap();
    assert_eq!(failed.status(), http::StatusCode::BAD_GATEWAY);
    drop(failed);
    let values = snapshot.snapshot().into_vec();
    let mut requests = 0;
    let mut routes = std::collections::BTreeSet::new();
    let mut methods = std::collections::BTreeSet::new();
    let mut statuses = std::collections::BTreeSet::new();
    assert!(values.len() < 25, "user values must not grow the series set");
    for (key, _, _, value) in values {
        assert!(!format!("{key:?}").contains("canary"));
        for label in key.key().labels() {
            match label.key() {
                "route" => {
                    routes.insert(label.value().to_owned());
                }
                "method" => {
                    methods.insert(label.value().to_owned());
                }
                "status" => {
                    statuses.insert(label.value().to_owned());
                }
                _ => {}
            }
        }
        if key.key().name() == capsem_telemetry::http::REQUESTS_TOTAL {
            let DebugValue::Counter(count) = value else { panic!() };
            requests += count;
        }
    }
    assert_eq!(requests, 105);
    assert_eq!(
        routes,
        ["/health", "/vms/{id}/info", "unmatched"].map(str::to_owned).into()
    );
    assert_eq!(methods, ["GET", "OPTIONS", "OTHER"].map(str::to_owned).into());
    assert_eq!(statuses, ["200", "401", "403", "404", "502"].map(str::to_owned).into());
    let records = std::mem::take(&mut *records.lock().unwrap());
    assert!(records.iter().any(|record| record.contains("/vms/{id}/info")));
    assert!(!records
        .iter()
        .any(|record| record.contains("canary") || record.contains("PRIVATE-METHOD")));
}
