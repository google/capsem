//! Exported session metrics are the API's numbers, not a second count.

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

use super::*;
use crate::service_runtime::telemetry_export::{collect, grant_metric_broker, register, SessionTable};

fn net(decision: capsem_logger::Decision, bytes_sent: u64) -> capsem_logger::WriteOp {
    capsem_logger::WriteOp::NetEvent(
        serde_json::from_value(serde_json::json!({
            "timestamp": 1_789_000_000.0, "domain": "example.com", "port": 443,
            "decision": decision.as_str(), "bytes_sent": bytes_sent, "bytes_received": 0, "duration_ms": 1,
        }))
        .unwrap(),
    )
}

fn model(input: u64, output: u64, cost: f64) -> capsem_logger::WriteOp {
    capsem_logger::WriteOp::ModelCall(
        serde_json::from_value(serde_json::json!({
            "timestamp": 1_789_000_000.0, "provider": "anthropic", "model": "m", "method": "POST",
            "path": "/v1/messages", "stream": false, "messages_count": 1, "tools_count": 0,
            "request_bytes": 1, "input_tokens": input, "output_tokens": output, "usage_details": {},
            "duration_ms": 1, "response_bytes": 1, "estimated_cost_usd": cost,
            "tool_calls": [{"call_index": 0, "call_id": "c", "tool_name": "bash", "origin": "native"}],
            "tool_responses": [],
        }))
        .unwrap(),
    )
}

fn session(state: &ServiceState, id: &str, ops: Vec<capsem_logger::WriteOp>) {
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    for op in ops {
        writer.write_blocking(op);
    }
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(state, id, std::process::id(), session_dir);
}

/// Every exported point, as (metric, session.id, extra attribute, value).
fn exported(exporter: &InMemoryMetricExporter) -> Vec<(String, String, String, f64)> {
    let mut points = Vec::new();
    for resource in exporter.get_finished_metrics().unwrap() {
        for metric in resource.scope_metrics().flat_map(|scope| scope.metrics()) {
            let mut push = |attributes: Vec<&opentelemetry::KeyValue>, value: f64| {
                let mut keys: Vec<String> = attributes.iter().map(|kv| kv.key.to_string()).collect();
                keys.sort();
                let session = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == "session.id")
                    .map(|kv| kv.value.to_string())
                    .unwrap();
                let extra = attributes
                    .iter()
                    .find(|kv| !["session.id", "persistent"].contains(&kv.key.as_str()))
                    .map(|kv| kv.value.to_string())
                    .unwrap_or_default();
                let base = ["persistent", "session.id"];
                assert!(
                    base.iter().all(|key| keys.iter().any(|k| k == key)) && keys.len() <= 3,
                    "{}: attributes are exactly session.id, persistent (+ one breakdown): {keys:?}",
                    metric.name()
                );
                points.push((metric.name().to_string(), session, extra, value));
            };
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                    for point in sum.data_points() {
                        push(point.attributes().collect(), point.value() as f64);
                    }
                }
                AggregatedMetrics::F64(MetricData::Sum(sum)) => {
                    for point in sum.data_points() {
                        push(point.attributes().collect(), point.value());
                    }
                }
                other => panic!("{} exported as {other:?}", metric.name()),
            }
        }
    }
    points
}

#[tokio::test]
async fn exported_session_metrics_equal_the_api_totals() {
    let (state, _dir) = make_test_state_with_tempdir();
    session(
        &state,
        "export-a",
        vec![
            net(capsem_logger::Decision::Allowed, 1),
            net(capsem_logger::Decision::Denied, 1),
            model(100, 40, 0.25),
        ],
    );
    session(&state, "export-b", vec![model(7, 3, 0.000_5), model(1, 1, 0.1)]);

    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let table = SessionTable::default();
    let _instruments = register(
        &opentelemetry::metrics::MeterProvider::meter(&provider, "capsem"),
        &table,
    );
    *table.write().unwrap() = collect(&state).await.unwrap();
    provider.force_flush().unwrap();
    let points = exported(&exporter);
    let value = |metric: &str, session: &str, extra: &str| {
        points
            .iter()
            .find(|(m, s, e, _)| m == metric && s == session && e == extra)
            .map(|(_, _, _, value)| *value)
            .unwrap_or_else(|| panic!("no {metric} point for {session} {extra}: {points:?}"))
    };

    let app = build_service_router(Arc::clone(&state));
    for id in ["export-a", "export-b"] {
        let (status, info) =
            route_request(app.clone(), axum::http::Method::GET, &format!("/vms/{id}/info"), None).await;
        assert_eq!(status, StatusCode::OK, "{info}");
        let api = |field: &str| info[field].as_f64().unwrap_or_else(|| panic!("{field}: {info}"));
        use capsem_telemetry::session as n;
        assert_eq!(value(n::SESSION_REQUESTS_TOTAL, id, "allowed"), api("allowed_requests"));
        assert_eq!(value(n::SESSION_REQUESTS_TOTAL, id, "denied"), api("denied_requests"));
        assert_eq!(value(n::SESSION_TOKENS_TOTAL, id, "input"), api("total_input_tokens"));
        assert_eq!(value(n::SESSION_TOKENS_TOTAL, id, "output"), api("total_output_tokens"));
        assert_eq!(value(n::SESSION_MODEL_CALLS_TOTAL, id, ""), api("model_call_count"));
        assert_eq!(value(n::SESSION_TOOL_CALLS_TOTAL, id, ""), api("total_tool_calls"));
        assert_eq!(value(n::SESSION_FILE_EVENTS_TOTAL, id, ""), api("total_file_events"));
        assert_eq!(value(n::SESSION_COST_USD_TOTAL, id, ""), api("total_estimated_cost"));
    }
    assert_eq!(
        value(capsem_telemetry::session::SESSION_TOKENS_TOTAL, "export-b", "input"),
        8.0
    );
}

fn granted(open_telemetry: Option<&str>) -> Vec<String> {
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = open_telemetry.map(str::to_string);
    let mut command = tokio::process::Command::new("capsem-process");
    grant_metric_broker(&mut command, &corp);
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// A VM process learns only whether the service granted brokered export.
#[test]
fn a_vm_process_is_granted_only_the_metric_broker_at_launch() {
    assert_eq!(granted(Some("https://otel.example/")), ["--metric-broker"]);
    assert!(granted(None).is_empty(), "no corp endpoint, no export");
    assert!(granted(Some("  ")).is_empty(), "a blank endpoint is no endpoint");
}

type CollectedMetric = (axum::http::HeaderMap, axum::body::Bytes);

/// The qualification child uses the worker's production transport shape: the
/// OTLP encoder controls only the body, while this client fixes both the local
/// service socket and the session-specific broker route.
struct QualificationMetricClient {
    service_socket: std::path::PathBuf,
    path: String,
    runtime: std::sync::Mutex<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for QualificationMetricClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QualificationMetricClient")
            .field("service_socket", &self.service_socket)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for QualificationMetricClient {
    async fn send_bytes(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .block_on(capsem_core::service_uds::post_bytes(
                &self.service_socket,
                &self.path,
                http::HeaderValue::from_static("application/x-protobuf"),
                request.into_body(),
            ))
            .map_err(|error| Box::new(std::io::Error::other(format!("metric broker request: {error:#}"))) as _)
    }
}

#[test]
fn metric_relay_attribution_child() {
    use std::io::Read as _;

    let (Ok(socket), Ok(id)) = (
        std::env::var("CAPSEM_TEST_METRIC_ATTRIBUTION_SOCKET"),
        std::env::var("CAPSEM_TEST_METRIC_ATTRIBUTION_ID"),
    ) else {
        return;
    };
    let mut release = [0];
    std::io::stdin().read_exact(&mut release).unwrap();
    let client = QualificationMetricClient {
        service_socket: socket.into(),
        path: format!("/internal/vms/{id}/metrics"),
        runtime: std::sync::Mutex::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        ),
    };
    let exporter = capsem_telemetry::export::install_with_http_client(
        client,
        "capsem-process",
        vec![capsem_telemetry::export::KeyValue::new("session.id", id)],
    )
    .unwrap();
    metrics::counter!(capsem_telemetry::db::DB_WRITE_OPS_TOTAL).increment(1);
    exporter.flush().unwrap();
}

async fn metric_collector(
    State(sender): State<tokio::sync::mpsc::UnboundedSender<CollectedMetric>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, [(&'static str, &'static str); 1], axum::body::Bytes) {
    sender.send((headers, body)).unwrap();
    (
        StatusCode::CREATED,
        [("content-type", "application/x-protobuf")],
        axum::body::Bytes::from_static(b"collector response"),
    )
}

fn protobuf_contains(body: &[u8], expected: &str) -> bool {
    body.windows(expected.len()).any(|window| window == expected.as_bytes())
}

#[tokio::test]
async fn worker_metrics_keep_exact_attribution_through_the_service_broker() {
    use tokio::io::AsyncWriteExt as _;

    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let settings_dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, corp_path) = install_empty_settings_env(&settings_dir);
    let collector_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let collector_base = format!("http://{}", collector_listener.local_addr().unwrap());
    let (collected_tx, mut collected_rx) = tokio::sync::mpsc::unbounded_channel();
    let collector = tokio::spawn(async move {
        axum::serve(
            collector_listener,
            axum::Router::new()
                .route("/v1/metrics", axum::routing::post(metric_collector))
                .with_state(collected_tx),
        )
        .await
        .unwrap();
    });
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some(collector_base);
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();

    let (state, directory) = make_test_state_with_tempdir();
    let socket = directory.path().join("metric-attribution.sock");
    let service_listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let service_state = Arc::clone(&state);
    let service = tokio::spawn(async move {
        axum::serve(
            service_listener,
            build_service_router(service_state).into_make_service_with_connect_info::<ServicePeer>(),
        )
        .await
        .unwrap();
    });

    let id = "attributed-vm";
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::telemetry_export::metric_relay_attribution_child",
            "--nocapture",
        ])
        .env("CAPSEM_TEST_METRIC_ATTRIBUTION_SOCKET", &socket)
        .env("CAPSEM_TEST_METRIC_ATTRIBUTION_ID", id)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    insert_fake_instance(&state, id, child.id().unwrap());
    child.stdin.take().unwrap().write_all(b"1").await.unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .expect("metric-export child completes")
        .unwrap();
    assert!(status.success(), "metric-export child failed: {status}");

    let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(2), collected_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        headers.get(axum::http::header::CONTENT_TYPE).unwrap(),
        "application/x-protobuf"
    );
    for expected in [
        "service.name",
        "capsem-process",
        "session.id",
        id,
        capsem_telemetry::db::DB_WRITE_OPS_TOTAL,
    ] {
        assert!(protobuf_contains(&body, expected), "OTLP body lacks {expected}");
    }

    service.abort();
    collector.abort();
}

#[test]
fn metric_relay_wrong_process_client() {
    let Ok(socket) = std::env::var("CAPSEM_TEST_METRIC_RELAY_SOCKET") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let response = runtime
        .block_on(capsem_core::service_uds::post_bytes(
            std::path::Path::new(&socket),
            "/internal/vms/vm-a/metrics",
            axum::http::HeaderValue::from_static("application/x-protobuf"),
            axum::body::Bytes::from_static(b"wrong peer"),
        ))
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn metric_relay_uses_only_current_owner_and_configured_collector() {
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let settings_dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, corp_path) = install_empty_settings_env(&settings_dir);
    let collector_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let collector_base = format!("http://{}", collector_listener.local_addr().unwrap());
    let (collected_tx, mut collected_rx) = tokio::sync::mpsc::unbounded_channel();
    let collector = tokio::spawn(async move {
        axum::serve(
            collector_listener,
            axum::Router::new()
                .route("/v1/metrics", axum::routing::post(metric_collector))
                .with_state(collected_tx),
        )
        .await
        .unwrap();
    });
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some(collector_base);
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();

    let (state, directory) = make_test_state_with_tempdir();
    insert_fake_instance(&state, "vm-a", std::process::id());
    let peer = ServicePeer(Some(capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    }));
    let socket = directory.path().join("metric-relay.sock");
    let service_listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let service_state = Arc::clone(&state);
    let service = tokio::spawn(async move {
        axum::serve(
            service_listener,
            build_service_router(service_state).into_make_service_with_connect_info::<ServicePeer>(),
        )
        .await
        .unwrap();
    });

    let response = capsem_core::service_uds::post_bytes(
        &socket,
        "/internal/vms/vm-a/metrics",
        axum::http::HeaderValue::from_static("application/x-protobuf"),
        axum::body::Bytes::from_static(b"encoded metrics"),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.body(), b"collector response".as_slice());
    let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(2), collected_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        headers.get(axum::http::header::CONTENT_TYPE).unwrap(),
        "application/x-protobuf"
    );
    assert_eq!(body, b"encoded metrics".as_slice());

    use tower::ServiceExt as _;
    let mock_app = build_service_router(Arc::clone(&state)).layer(axum::extract::connect_info::MockConnectInfo(peer));
    let response = mock_app
        .clone()
        .oneshot(
            axum::http::Request::post("/internal/vms/vm-a/metrics")
                .header(axum::http::header::CONTENT_TYPE, "application/x-protobuf")
                .header(axum::http::header::AUTHORIZATION, "Bearer must-not-cross")
                .header("x-otlp-endpoint", "http://attacker.invalid")
                .body(axum::body::Body::from("fixed destination"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(2), collected_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!headers.contains_key(axum::http::header::AUTHORIZATION));
    assert!(!headers.contains_key("x-otlp-endpoint"));
    assert_eq!(body, b"fixed destination".as_slice());

    let response = mock_app
        .oneshot(
            axum::http::Request::post("/internal/vms/vm-a/metrics")
                .header(axum::http::header::CONTENT_TYPE, "application/x-protobuf")
                .body(axum::body::Body::from(vec![
                    0;
                    capsem_core::service_uds::MAX_BODY_BYTES + 1
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        collected_rx.try_recv().is_err(),
        "an oversized body reached the collector"
    );

    let status = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::telemetry_export::metric_relay_wrong_process_client",
            "--nocapture",
        ])
        .env("CAPSEM_TEST_METRIC_RELAY_SOCKET", &socket)
        .status()
        .await
        .unwrap();
    assert!(status.success(), "wrong-process probe failed: {status}");
    assert!(
        collected_rx.try_recv().is_err(),
        "a denied sibling reached the collector"
    );

    state.instances.lock().unwrap().get_mut("vm-a").unwrap().pid = 1;
    let response = capsem_core::service_uds::post_bytes(
        &socket,
        "/internal/vms/vm-a/metrics",
        axum::http::HeaderValue::from_static("application/x-protobuf"),
        axum::body::Bytes::from_static(b"stale generation"),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        collected_rx.try_recv().is_err(),
        "a replaced owner reached the collector"
    );
    state.instances.lock().unwrap().get_mut("vm-a").unwrap().pid = std::process::id();

    let attacker_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_target = format!("http://{}/stolen", attacker_listener.local_addr().unwrap());
    let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_base = format!("http://{}", redirect_listener.local_addr().unwrap());
    let redirect = tokio::spawn(async move {
        axum::serve(
            redirect_listener,
            axum::Router::new()
                .route(
                    "/v1/metrics",
                    axum::routing::post(|State(target): State<String>| async move {
                        (StatusCode::TEMPORARY_REDIRECT, [(axum::http::header::LOCATION, target)])
                    }),
                )
                .with_state(redirect_target),
        )
        .await
        .unwrap();
    });
    corp.corp_rule_files.open_telemetry = Some(redirect_base);
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let response = capsem_core::service_uds::post_bytes(
        &socket,
        "/internal/vms/vm-a/metrics",
        axum::http::HeaderValue::from_static("application/x-protobuf"),
        axum::body::Bytes::from_static(b"do not redirect"),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), attacker_listener.accept())
            .await
            .is_err(),
        "the collector redirect opened an ungranted destination"
    );

    capsem_core::net::policy_config::write_settings_file(
        &corp_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();
    let response = capsem_core::service_uds::post_bytes(
        &socket,
        "/internal/vms/vm-a/metrics",
        axum::http::HeaderValue::from_static("application/x-protobuf"),
        axum::body::Bytes::from_static(b"disabled"),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        collected_rx.try_recv().is_err(),
        "disabled export reached the old collector"
    );

    service.abort();
    collector.abort();
    redirect.abort();
}

#[tokio::test]
async fn metric_relay_drops_the_upstream_connection_when_generation_is_revoked() {
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};

    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let settings_dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, corp_path) = install_empty_settings_env(&settings_dir);
    let collector_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let collector_base = format!("http://{}", collector_listener.local_addr().unwrap());
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some(collector_base);
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let collector = tokio::spawn(async move {
        let (stream, _) = collector_listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut content_length = 0;
        loop {
            let mut line = Vec::new();
            reader.read_until(b'\n', &mut line).await.unwrap();
            if line == b"\r\n" {
                break;
            }
            let text = String::from_utf8_lossy(&line).to_ascii_lowercase();
            if let Some(value) = text.strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).await.unwrap();
        accepted_tx.send(body).unwrap();
        let mut byte = [0];
        closed_tx.send(reader.read(&mut byte).await.unwrap() == 0).unwrap();
    });

    let (state, directory) = make_test_state_with_tempdir();
    insert_fake_instance(&state, "vm-a", std::process::id());
    let generation = state.instances.lock().unwrap()["vm-a"].generation;
    let socket = directory.path().join("metric-revocation.sock");
    let service_listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let service_state = Arc::clone(&state);
    let service = tokio::spawn(async move {
        axum::serve(
            service_listener,
            build_service_router(service_state).into_make_service_with_connect_info::<ServicePeer>(),
        )
        .await
        .unwrap();
    });
    let request_socket = socket.clone();
    let request = tokio::spawn(async move {
        capsem_core::service_uds::post_bytes(
            &request_socket,
            "/internal/vms/vm-a/metrics",
            axum::http::HeaderValue::from_static("application/x-protobuf"),
            axum::body::Bytes::from_static(b"cancel me"),
        )
        .await
        .unwrap()
    });
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), accepted_rx)
            .await
            .unwrap()
            .unwrap(),
        b"cancel me"
    );
    assert!(state.evict_instance("vm-a", generation).is_some());
    let response = tokio::time::timeout(std::time::Duration::from_secs(1), request)
        .await
        .expect("revocation cancels the in-flight collector request")
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), closed_rx)
            .await
            .unwrap()
            .unwrap(),
        "canceling the upstream future closes its collector connection"
    );
    service.abort();
    collector.abort();
}
