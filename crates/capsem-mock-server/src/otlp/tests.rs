use super::*;
use http_body_util::Full;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue},
    metrics::v1::{metric, number_data_point, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum},
    resource::v1::Resource,
};
use prost::Message;

fn attribute(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.into())),
        }),
        ..Default::default()
    }
}
fn fixture() -> Bytes {
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
                    data: Some(metric::Data::Sum(Sum {
                        aggregation_temporality: 2,
                        is_monotonic: true,
                        data_points: vec![NumberDataPoint {
                            attributes: vec![
                                attribute("component", "gateway"),
                                attribute("method", "GET"),
                                attribute("route", "/vms/{id}/info"),
                            ],
                            value: Some(number_data_point::Value::AsInt(7)),
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
    .into()
}
fn request(body: Bytes) -> Request<Full<Bytes>> {
    Request::post("/otlp/v1/metrics")
        .header("authorization", AUTHORIZATION)
        .header(CONTENT_TYPE, "application/x-protobuf")
        .body(Full::new(body))
        .unwrap()
}

#[tokio::test]
async fn exact_generated_protobuf_yields_only_bounded_normalized_summary() {
    let collector = Collector::default();
    let bytes = fixture();
    let response = handle(request(bytes.clone()), Some(&collector)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let summary = collector.summary.lock().unwrap().clone();
    assert_eq!(summary.batches, 1);
    assert_eq!(summary.wire_bytes, bytes.len() as u64);
    assert_eq!(summary.points.len(), 1);
    assert_eq!(summary.points[0].value, 7);
    assert_eq!(summary.points[0].labels["route"], "/vms/{id}/info");
}

#[tokio::test]
async fn disabled_auth_media_and_oversized_or_invalid_payloads_cannot_pollute_summary() {
    let collector = Collector::default();
    assert_eq!(handle(request(fixture()), None).await.status(), StatusCode::NOT_FOUND);
    let mut bad = request(fixture());
    bad.headers_mut().remove("authorization");
    assert_eq!(handle(bad, Some(&collector)).await.status(), StatusCode::UNAUTHORIZED);
    let mut bad = request(fixture());
    bad.headers_mut()
        .insert(CONTENT_TYPE, "application/json".parse().unwrap());
    assert_eq!(
        handle(bad, Some(&collector)).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let mut bad = request(fixture());
    bad.headers_mut().insert("content-encoding", "gzip".parse().unwrap());
    assert_eq!(
        handle(bad, Some(&collector)).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        handle(request(Bytes::from(vec![0; MAX_BODY_BYTES + 1])), Some(&collector))
            .await
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        handle(request(Bytes::from_static(b"raw-secret-canary")), Some(&collector))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let summary = collector.summary.lock().unwrap().clone();
    assert_eq!(summary.batches, 0);
    assert!(summary.points.is_empty());
    assert!(!serde_json::to_string(&summary).unwrap().contains("canary"));
}

#[tokio::test]
async fn actual_http_collector_has_no_generic_payload_or_authorization_log() {
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("requests.jsonl");
    let file = std::fs::File::create(&log).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = crate::State {
        request_log: Some(Arc::new(Mutex::new(file))),
        dns_answers: crate::DnsAnswers::default(),
        otlp: Some(Arc::new(Collector::default())),
    };
    let server = tokio::spawn(crate::serve_http(listener, state, false));
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/otlp/v1/metrics"))
        .header("authorization", AUTHORIZATION)
        .header(CONTENT_TYPE, "application/x-protobuf")
        .body(fixture())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let summary: serde_json::Value = client
        .get(format!("http://{addr}/otlp/summary"))
        .header("authorization", AUTHORIZATION)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["batches"], 1);
    assert_eq!(summary["authorization_verified"], true);
    assert!(!summary.to_string().contains(AUTHORIZATION));
    assert_eq!(std::fs::read(&log).unwrap(), b"");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn private_route_values_are_rejected_and_latest_summary_does_not_accumulate() {
    let collector = Collector::default();
    let mut decoded = ExportMetricsServiceRequest::decode(fixture()).unwrap();
    let metric::Data::Sum(sum) = decoded.resource_metrics[0].scope_metrics[0].metrics[0]
        .data
        .as_mut()
        .unwrap()
    else {
        panic!()
    };
    sum.data_points[0].attributes[2] = attribute("route", "/vms/private-route-canary/info");
    assert_eq!(
        handle(request(decoded.encode_to_vec().into()), Some(&collector))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    for _ in 0..100 {
        assert_eq!(
            handle(request(fixture()), Some(&collector)).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    let summary = collector.summary.lock().unwrap().clone();
    assert_eq!(summary.batches, 100);
    assert_eq!(summary.rejected, 1);
    assert_eq!(summary.points.len(), 1);
    assert!(!serde_json::to_string(&summary).unwrap().contains("canary"));
}
