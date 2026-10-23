use super::*;
use crate::tests::{install_empty_settings_env, EnvVarGuard, SETTINGS_ENV_LOCK};
use axum::http::StatusCode;
use capsem_foundation::ipc_channel::{channel_from_std_with_limit, Receiver, Sender};
use capsem_proto::gateway_grant::metrics::{
    GatewayMetricMessage, GatewayMetricOutcome, GatewayMetricRequest, MAX_GATEWAY_METRIC_FRAME_BYTES,
};
use prost::Message;
use tokio::time::{timeout, Duration};

type Client = (Sender<GatewayMetricRequest>, Receiver<GatewayMetricMessage>);

fn valid_body() -> Vec<u8> {
    use opentelemetry_proto::tonic::{
        collector::metrics::v1::ExportMetricsServiceRequest,
        common::v1::{any_value::Value, AnyValue, InstrumentationScope, KeyValue},
        metrics::v1::{metric::Data, number_data_point, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum},
        resource::v1::Resource,
    };
    let attribute = |key: &str, value: &str| KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.into())),
        }),
        ..Default::default()
    };
    let spec = &capsem_telemetry::http::SPECS[0];
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![attribute("service.name", "capsem-gateway")],
                ..Default::default()
            }),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(InstrumentationScope {
                    name: "capsem".into(),
                    ..Default::default()
                }),
                metrics: vec![Metric {
                    name: spec.name.into(),
                    description: spec.description.into(),
                    unit: "{count}".into(),
                    data: Some(Data::Sum(Sum {
                        aggregation_temporality: 2,
                        is_monotonic: true,
                        data_points: vec![NumberDataPoint {
                            attributes: vec![
                                attribute("component", "gateway"),
                                attribute("method", "GET"),
                                attribute("route", "/vms/list"),
                            ],
                            value: Some(number_data_point::Value::AsInt(1)),
                            ..Default::default()
                        }],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

async fn connect(state: &Arc<ServiceState>, authority: crate::instance::WorkerGrant) -> (Client, [u8; 16]) {
    let prepared = prepare(state, authority).await.unwrap();
    let socket = prepared.descriptor;
    prepared.activate.send(()).unwrap();
    let client = channel_from_std_with_limit(socket, MAX_GATEWAY_METRIC_FRAME_BYTES).unwrap();
    let GatewayMetricMessage::Hello { generation } =
        timeout(Duration::from_secs(2), client.1.recv()).await.unwrap().unwrap()
    else {
        panic!()
    };
    assert_ne!(generation, [0; 16]);
    (client, generation)
}

#[tokio::test]
async fn rejects_bad_payload_then_relays_only_admitted_bytes_and_revokes_with_owner() {
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_settings, _, corp_path) = install_empty_settings_env(&directory);
    let _base = EnvVarGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let _signal = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "");
    let _headers = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "");
    let _protocol = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "http/protobuf");
    let _compression = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_COMPRESSION", "");
    let state = crate::tests::make_test_state();
    let authority = crate::instance::WorkerAuthority::default();
    assert!(matches!(
        prepare(&state, authority.grant()).await,
        Err(GatewayGrantDenial::Unavailable)
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some(format!("http://{}", listener.local_addr().unwrap()));
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let (tx, mut received) = tokio::sync::mpsc::channel::<axum::body::Bytes>(1);
    let collector = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/v1/metrics",
                axum::routing::post(move |body: axum::body::Bytes| async move {
                    tx.send(body).await.unwrap();
                    (StatusCode::OK, "never-forward-collector-response-canary")
                }),
            ),
        )
        .await
        .unwrap()
    });
    let ((requests, responses), generation) = connect(&state, authority.grant()).await;
    requests
        .send(GatewayMetricRequest {
            generation,
            request_id: 1,
            body: b"private-payload-canary".to_vec(),
        })
        .await
        .unwrap();
    assert_eq!(
        responses.recv().await.unwrap(),
        GatewayMetricMessage::Receipt {
            request_id: 1,
            outcome: GatewayMetricOutcome::Rejected
        }
    );
    assert!(received.try_recv().is_err());
    let body = valid_body();
    requests
        .send(GatewayMetricRequest {
            generation,
            request_id: 2,
            body: body.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(2), responses.recv())
            .await
            .unwrap()
            .unwrap(),
        GatewayMetricMessage::Receipt {
            request_id: 2,
            outcome: GatewayMetricOutcome::Accepted
        }
    );
    assert_eq!(received.recv().await.unwrap().as_ref(), body.as_slice());
    // Disabling export after a grant is respected on the next batch.
    corp.corp_rule_files.open_telemetry = None;
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    requests
        .send(GatewayMetricRequest {
            generation,
            request_id: 3,
            body,
        })
        .await
        .unwrap();
    assert_eq!(
        responses.recv().await.unwrap(),
        GatewayMetricMessage::Receipt {
            request_id: 3,
            outcome: GatewayMetricOutcome::Unavailable
        }
    );
    assert!(received.try_recv().is_err());
    authority.revoke();
    assert!(timeout(Duration::from_secs(2), responses.recv())
        .await
        .unwrap()
        .is_err());
    collector.abort();
    let _ = collector.await;
}

#[tokio::test]
async fn stale_foreign_and_replayed_identities_close_without_exporting() {
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_settings, _, corp_path) = install_empty_settings_env(&directory);
    let _base = EnvVarGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let _signal = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "");
    let _headers = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "");
    let _protocol = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "http/protobuf");
    let _compression = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_COMPRESSION", "");
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some("http://127.0.0.1:1".into());
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let state = crate::tests::make_test_state();
    for mode in 0..4 {
        let authority = crate::instance::WorkerAuthority::default();
        let ((requests, responses), generation) = connect(&state, authority.grant()).await;
        if mode == 3 {
            requests
                .send(GatewayMetricRequest {
                    generation,
                    request_id: 1,
                    body: b"invalid-otlp".to_vec(),
                })
                .await
                .unwrap();
            assert_eq!(
                responses.recv().await.unwrap(),
                GatewayMetricMessage::Receipt {
                    request_id: 1,
                    outcome: GatewayMetricOutcome::Rejected
                }
            );
        }
        requests
            .send(GatewayMetricRequest {
                generation: if mode == 0 { [0xff; 16] } else { generation },
                request_id: if mode == 1 {
                    0
                } else if mode == 2 {
                    2
                } else {
                    1
                },
                body: valid_body(),
            })
            .await
            .unwrap();
        assert!(timeout(Duration::from_secs(2), responses.recv())
            .await
            .unwrap()
            .is_err());
    }
}

#[tokio::test]
async fn malformed_and_oversized_frames_close_with_static_error_before_export() {
    use tokio::io::AsyncWriteExt;
    for oversized in [false, true] {
        let state = crate::tests::make_test_state();
        let authority = crate::instance::WorkerAuthority::default();
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let serving = tokio::spawn(serve(state, server, authority.grant(), [7; 16]));
        client.set_nonblocking(true).unwrap();
        let mut client = tokio::net::UnixStream::from_std(client).unwrap();
        if oversized {
            client.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        } else {
            let request = GatewayMetricRequest {
                generation: [7; 16],
                request_id: 1,
                body: valid_body(),
            };
            let mut body = rmp_serde::to_vec_named(&request).unwrap();
            assert_eq!(body[0], 0x83);
            body[0] = 0x84;
            body.extend_from_slice(b"\xb9private-field-name-canary\xa1x");
            client.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
            client.write_all(&body).await.unwrap();
        }
        let error = timeout(Duration::from_secs(2), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.to_string(), "gateway metric capability rejected");
    }
}

#[tokio::test]
async fn owner_revocation_interrupts_an_inflight_collector_wait() {
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_settings, _, corp_path) = install_empty_settings_env(&directory);
    let _base = EnvVarGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let _signal = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "");
    let _headers = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some(format!("http://{}", listener.local_addr().unwrap()));
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let (seen, mut received) = tokio::sync::mpsc::channel(1);
    let collector = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/v1/metrics",
                axum::routing::post(move |_: axum::body::Bytes| async move {
                    seen.send(()).await.unwrap();
                    std::future::pending::<StatusCode>().await
                }),
            ),
        )
        .await
        .unwrap()
    });
    let state = crate::tests::make_test_state();
    let authority = crate::instance::WorkerAuthority::default();
    let ((requests, responses), generation) = connect(&state, authority.grant()).await;
    requests
        .send(GatewayMetricRequest {
            generation,
            request_id: 1,
            body: valid_body(),
        })
        .await
        .unwrap();
    timeout(Duration::from_secs(2), received.recv()).await.unwrap().unwrap();
    authority.revoke();
    assert!(timeout(Duration::from_secs(2), responses.recv())
        .await
        .unwrap()
        .is_err());
    collector.abort();
    let _ = collector.await;
}

#[tokio::test]
async fn gateway_child_environment_excludes_ambient_credentials_and_retains_diagnostics() {
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let _provider = EnvVarGuard::set("CAPSEM_TEST_PROVIDER_CREDENTIAL", "fake-provider-canary");
    let _header = EnvVarGuard::set("OTEL_EXPORTER_OTLP_HEADERS", "fake-collector-canary");
    let _unknown = EnvVarGuard::set("OTEL_FUTURE_CONFIG", "fake-future-canary");
    let _log = EnvVarGuard::set("RUST_LOG", "capsem_gateway=debug");
    let _backtrace = EnvVarGuard::set("RUST_BACKTRACE", "1");
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    strip_collector_environment(&mut command);
    let expected_profile = std::env::var_os("LLVM_PROFILE_FILE");
    assert_eq!(
        command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new("LLVM_PROFILE_FILE"))
            .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string)),
        expected_profile
    );
    let output = command
        .args([
            "--exact",
            "service_runtime::telemetry_export::gateway::tests::gateway_environment_child_probe",
            "--nocapture",
        ])
        .env("CAPSEM_GATEWAY_ENV_PROBE", "1")
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn gateway_environment_child_probe() {
    if std::env::var_os("CAPSEM_GATEWAY_ENV_PROBE").is_none() {
        return;
    }
    // Inspect only these synthetic canaries, never real provider credentials.
    for key in [
        "CAPSEM_TEST_PROVIDER_CREDENTIAL",
        "OTEL_EXPORTER_OTLP_HEADERS",
        "OTEL_FUTURE_CONFIG",
    ] {
        assert!(std::env::var_os(key).is_none(), "ambient canary inherited");
    }
    assert_eq!(std::env::var("RUST_LOG").unwrap(), "capsem_gateway=debug");
    assert_eq!(std::env::var("RUST_BACKTRACE").unwrap(), "1");
}

#[tokio::test]
async fn actual_service_router_counts_owner_denials_and_bounds_unknown_operation_cardinality() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use tower::ServiceExt;
    let state = crate::tests::make_test_state();
    crate::tests::insert_fake_instance_with_session_dir(
        &state,
        "owner",
        std::process::id(),
        state.run_dir.join("sessions/owner"),
    );
    let peer = crate::service_peer::ServicePeer(Some(capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    }));
    let app = crate::router_runtime::build_service_router(state);
    let recorder = DebuggingRecorder::new();
    let snapshot = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(async {
            for number in 0..100 {
                let request = axum::http::Request::builder()
                    .method(format!("CANARY{number}").as_str())
                    .uri(format!("/private-path-{number}?token=secret-canary"))
                    .body(axum::body::Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::NOT_FOUND);
                axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
            }
            let request = axum::http::Request::get("/version?token=private-query-canary")
                .extension(axum::extract::ConnectInfo(peer))
                .body(axum::body::Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        })
    });
    let recorded = snapshot.snapshot().into_vec();
    assert_eq!(
        recorded.len(),
        8,
        "Two normalized operations, four metric instruments each"
    );
    for (key, _, _, _) in &recorded {
        let labels: std::collections::HashMap<_, _> =
            key.key().labels().map(|label| (label.key(), label.value())).collect();
        assert_eq!(labels["component"], "service");
        assert!(!format!("{labels:?}").to_lowercase().contains("canary"));
        assert!(["unmatched", "/version"].contains(&labels["route"]));
    }
    assert!(recorded.iter().any(
        |(key, _, _, value)| key.key().name() == capsem_telemetry::http::REQUESTS_TOTAL
            && matches!(value, DebugValue::Counter(100))
    ));
    assert!(recorded.iter().any(|(key, _, _, _)| key
        .key()
        .labels()
        .any(|label| label.key() == "status" && label.value() == "403")));
}

#[tokio::test]
async fn public_http_requests_cannot_acquire_a_metric_capability() {
    use tower::ServiceExt;
    let app = crate::router_runtime::build_service_router(crate::tests::make_test_state());
    for path in ["/metrics", "/internal/gateway/metrics"] {
        let request = axum::http::Request::post(path)
            .header("content-type", "application/x-protobuf")
            .body(axum::body::Body::from("private-body-canary"))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
