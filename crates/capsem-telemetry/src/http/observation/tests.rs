use super::*;
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use http_body::{Body, Frame, SizeHint};
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

// These two router tests share tracing callsites. Keep local-dispatch capture
// isolated from the other test's deliberately uninstalled subscriber.
static TRACE_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Frames {
    frames: VecDeque<Result<Frame<Bytes>, &'static str>>,
}

impl Body for Frames {
    type Data = Bytes;
    type Error = &'static str;
    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        Poll::Ready(self.frames.pop_front())
    }
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(123)
    }
}

fn request() -> RequestObservation {
    RequestObservation::begin(Component::Gateway, "GET", "unmatched".into())
}

#[test]
fn body_frames_and_size_hint_survive_while_eof_finishes_only_once() {
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        let bytes = Bytes::from_static(b"opaque-body-canary");
        let pointer = bytes.as_ptr();
        let source = Frames {
            frames: [Ok(Frame::data(bytes)), Ok(Frame::trailers(HeaderMap::new()))].into(),
        };
        let mut body = request().ready(StatusCode::OK, source);
        assert_eq!(body.size_hint().exact(), Some(123));
        let mut context = Context::from_waker(Waker::noop());
        let Poll::Ready(Some(Ok(frame))) = Pin::new(&mut body).poll_frame(&mut context) else {
            panic!()
        };
        assert_eq!(frame.data_ref().unwrap().as_ptr(), pointer);
        assert!(matches!(
            Pin::new(&mut body).poll_frame(&mut context),
            Poll::Ready(Some(Ok(_)))
        ));
        assert!(matches!(
            Pin::new(&mut body).poll_frame(&mut context),
            Poll::Ready(None)
        ));
        drop(body);
    });
    let mut finished = 0;
    for (key, _, _, value) in snapshot.snapshot().into_vec() {
        if key.key().name() == super::super::RESPONSES_TOTAL {
            assert!(key
                .key()
                .labels()
                .any(|label| label.key() == "completion" && label.value() == "eof"));
            let DebugValue::Counter(value) = value else { panic!() };
            finished += value;
        }
    }
    assert_eq!(finished, 1);
}

#[test]
fn every_terminal_path_is_distinct_and_measured_once() {
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        drop(request());
        drop(request().ready(
            StatusCode::OK,
            Frames {
                frames: VecDeque::new(),
            },
        ));
        let mut failed = request().ready(
            StatusCode::OK,
            Frames {
                frames: [Err("private-error-canary")].into(),
            },
        );
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut failed).poll_frame(&mut context),
            Poll::Ready(Some(Err(_)))
        ));
        drop(failed);
        drop(request().ready(StatusCode::NO_CONTENT, http_body_util::Empty::<Bytes>::new()));
        drop(request().ready(StatusCode::SWITCHING_PROTOCOLS, http_body_util::Empty::<Bytes>::new()));
    });
    let mut outcomes = std::collections::BTreeMap::new();
    for (key, _, _, value) in snapshot.snapshot().into_vec() {
        assert!(!format!("{key:?}").contains("canary"));
        if key.key().name() == super::super::RESPONSES_TOTAL {
            let completion = key.key().labels().find(|label| label.key() == "completion").unwrap();
            let DebugValue::Counter(value) = value else { panic!() };
            outcomes.insert(completion.value().to_owned(), value);
        }
    }
    assert_eq!(
        outcomes,
        ["body_error", "cancelled", "dropped", "eof", "upgraded"]
            .map(|s| (s.to_owned(), 1))
            .into()
    );
}

#[test]
fn middleware_keys_use_matched_templates_not_user_paths_queries_or_methods() {
    let _capture = TRACE_CAPTURE_LOCK.lock().unwrap();
    use axum::{body::Body as AxumBody, routing::get, Router};
    use tower::ServiceExt;
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        let router = Router::new()
            .route("/vms/{id}/info", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(Component::Gateway, observe));
        for uri in [
            "/vms/private-canary-1/info?token=secret-canary",
            "/vms/private-canary-2/info",
            "/unknown/private-canary-3",
        ] {
            let response = futures::executor::block_on(
                router
                    .clone()
                    .oneshot(axum::http::Request::builder().uri(uri).body(AxumBody::empty()).unwrap()),
            )
            .unwrap();
            futures::executor::block_on(http_body_util::BodyExt::collect(response.into_body())).unwrap();
        }
    });
    for (key, _, _, _) in snapshot.snapshot().into_vec() {
        assert!(!format!("{key:?}").contains("canary"));
        assert!(key
            .key()
            .labels()
            .any(|label| label.key() == "route" && ["/vms/{id}/info", "unmatched"].contains(&label.value())));
    }
}

#[test]
fn pending_body_does_not_finish_until_it_is_dropped() {
    struct Pending;
    impl Body for Pending {
        type Data = Bytes;
        type Error = std::convert::Infallible;
        fn poll_frame(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
            Poll::Pending
        }
    }
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        let mut body = request().ready(StatusCode::OK, Pending);
        let mut context = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut body).poll_frame(&mut context).is_pending());
        assert!(!snapshot
            .snapshot()
            .into_vec()
            .iter()
            .any(|(key, _, _, _)| key.key().name() == super::super::RESPONSES_TOTAL));
        drop(body);
    });
    assert!(snapshot
        .snapshot()
        .into_vec()
        .iter()
        .any(|(key, _, _, value)| key.key().name() == super::super::RESPONSES_TOTAL
            && key
                .key()
                .labels()
                .any(|label| label.key() == "completion" && label.value() == "dropped")
            && matches!(value, DebugValue::Counter(1))));
}

#[cfg(feature = "export")]
#[test]
fn actual_otel_attributes_and_debug_spans_exclude_request_canaries() {
    let _capture = TRACE_CAPTURE_LOCK.lock().unwrap();
    use axum::{body::Body as AxumBody, routing::get, Router};
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let captured = Capture(Arc::default());
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let recorder = crate::export::OtelRecorder::new(provider.meter("http-test"));
    tracing::subscriber::with_default(subscriber, || {
        metrics::with_local_recorder(&recorder, || {
            assert!(tracing::enabled!(tracing::Level::DEBUG));
            tracing::debug!("privacy capture active");
            let router = Router::new()
                .route("/vms/{id}/info", get(|| async { "private-body-canary" }))
                .layer(axum::middleware::from_fn_with_state(Component::Service, observe));
            for (method, path) in [
                ("GET", "/vms/private-id-canary/info?token=query-canary"),
                ("METHOD-CANARY", "/vms/other-id-canary/info"),
                ("GET", "/unmatched-path-canary"),
            ] {
                let request = axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", "private-auth-canary")
                    .body(AxumBody::empty())
                    .unwrap();
                let response = futures::executor::block_on(router.clone().oneshot(request)).unwrap();
                futures::executor::block_on(http_body_util::BodyExt::collect(response.into_body())).unwrap();
            }
        })
    });
    provider.force_flush().unwrap();
    let exported = exporter.get_finished_metrics().unwrap();
    let text = format!("{exported:?}");
    assert!(!text.contains("canary") && !text.contains("CANARY"), "{text}");
    assert!(text.contains("/vms/{id}/info") && text.contains("OTHER"));
    assert!(text.contains(super::super::REQUESTS_TOTAL));
    assert!(text.contains(super::super::RESPONSE_READY_MS));
    assert!(text.contains(super::super::BODY_LIFETIME_MS));
    assert!(text.contains(super::super::RESPONSES_TOTAL));
    let mut observed = 0;
    for metric in exported
        .iter()
        .flat_map(|resource| resource.scope_metrics())
        .flat_map(|scope| scope.metrics())
    {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        if [super::super::REQUESTS_TOTAL, super::super::RESPONSES_TOTAL].contains(&metric.name()) {
            let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() else {
                panic!()
            };
            assert_eq!(sum.data_points().map(|point| point.value()).sum::<u64>(), 3);
            observed += 1;
        } else if [super::super::RESPONSE_READY_MS, super::super::BODY_LIFETIME_MS].contains(&metric.name()) {
            let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
                panic!()
            };
            assert_eq!(histogram.data_points().map(|point| point.count()).sum::<u64>(), 3);
            assert_eq!(metric.unit(), "ms");
            observed += 1;
        }
    }
    assert_eq!(observed, 4);
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("canary") && !logs.contains("CANARY"), "{logs}");
    assert!(logs.contains("/vms/{id}/info") && logs.contains("OTHER"), "{logs}");
}

#[test]
fn cancelling_the_real_handler_future_records_no_fabricated_response_clock() {
    use axum::{body::Body as AxumBody, routing::get, Router};
    use std::future::Future;
    use tower::ServiceExt;
    let _capture = TRACE_CAPTURE_LOCK.lock().unwrap();
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        let router = Router::new()
            .route("/held", get(std::future::pending::<StatusCode>))
            .layer(axum::middleware::from_fn_with_state(Component::Service, observe));
        let request = axum::http::Request::builder()
            .uri("/held")
            .body(AxumBody::empty())
            .unwrap();
        let mut response = Box::pin(router.oneshot(request));
        assert!(response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        drop(response);
    });
    let recorded = snapshot.snapshot().into_vec();
    assert!(!recorded.iter().any(
        |(key, _, _, _)| [super::super::RESPONSE_READY_MS, super::super::BODY_LIFETIME_MS].contains(&key.key().name())
    ));
    let (key, _, _, DebugValue::Counter(1)) = recorded
        .iter()
        .find(|(key, _, _, _)| key.key().name() == super::super::RESPONSES_TOTAL)
        .unwrap()
    else {
        panic!()
    };
    assert!(key
        .key()
        .labels()
        .any(|label| label.key() == "completion" && label.value() == "cancelled"));
    assert!(key
        .key()
        .labels()
        .any(|label| label.key() == "status" && label.value() == "none"));
}
