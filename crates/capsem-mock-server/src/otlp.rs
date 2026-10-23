//! Explicitly enabled bounded native-test collector. Never retains raw exports.
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use capsem_telemetry::export::http_admission::{validate, MAX_BODY_BYTES};
use http_body_util::{BodyExt, Limited};
use hyper::header::CONTENT_TYPE;
use hyper::{Method, Request, StatusCode};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::{any_value, KeyValue},
    metrics::v1::{metric, number_data_point},
};
use prost::Message;
use serde::Serialize;

pub(crate) const ENDPOINTS: &[&str] = &["/otlp/v1/metrics", "/otlp/summary"];
// Public synthetic fixture credential; never use a real collector secret.
pub(crate) const AUTHORIZATION: &str = "Bearer capsem-test-collector";

#[derive(Default)]
pub(crate) struct Collector {
    summary: Mutex<Summary>,
}
#[derive(Clone, Default, Serialize)]
struct Summary {
    batches: u64,
    rejected: u64,
    wire_bytes: u64,
    authorization_verified: bool,
    points: Vec<Point>,
}
#[derive(Clone, Serialize)]
struct Point {
    name: String,
    labels: BTreeMap<String, String>,
    /// Counter value or histogram sample count, never a duration estimate.
    value: u64,
}

pub(crate) fn is_path(path: &str) -> bool {
    ENDPOINTS.contains(&path)
}

pub(crate) async fn handle<B>(req: Request<B>, collector: Option<&Collector>) -> hyper::Response<crate::RespBody>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let Some(collector) = collector else {
        return empty(StatusCode::NOT_FOUND);
    };
    if req.uri().query().is_some() {
        return empty(StatusCode::BAD_REQUEST);
    }
    if req
        .headers()
        .get("authorization")
        .is_none_or(|value| value.as_bytes() != AUTHORIZATION.as_bytes())
    {
        return empty(StatusCode::UNAUTHORIZED);
    }
    if req.method() == Method::GET && req.uri().path() == "/otlp/summary" {
        let snapshot = collector.summary.lock().unwrap().clone();
        return crate::json_response(serde_json::to_value(snapshot).unwrap());
    }
    if req.method() != Method::POST || req.uri().path() != "/otlp/v1/metrics" {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    if req
        .headers()
        .get(CONTENT_TYPE)
        .is_none_or(|value| value.as_bytes() != b"application/x-protobuf")
        || req.headers().contains_key("content-encoding")
    {
        return empty(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let body = match tokio::time::timeout(
        Duration::from_secs(10),
        Limited::new(req.into_body(), MAX_BODY_BYTES).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => return empty(StatusCode::PAYLOAD_TOO_LARGE),
        Err(_) => return empty(StatusCode::REQUEST_TIMEOUT),
    };
    // Shared production admission is the authority. A service's independent
    // non-gateway metrics are intentionally rejected rather than projected as
    // gateway evidence. This collector is only a functional test fixture.
    if validate(&body).is_err() {
        let mut summary = collector.summary.lock().unwrap();
        summary.rejected = summary.rejected.saturating_add(1);
        drop(summary);
        return empty(StatusCode::BAD_REQUEST);
    }
    // Decode with the same generated Rust OTLP owner to expose only admitted
    // facts. The fixture never exports raw protobuf, headers, paths or errors.
    let decoded = ExportMetricsServiceRequest::decode(body.clone()).expect("admitted protobuf");
    let mut points = Vec::new();
    for resource in decoded.resource_metrics {
        for scope in resource.scope_metrics {
            for metric in scope.metrics {
                match metric.data.expect("admitted metric") {
                    metric::Data::Sum(sum) => {
                        for point in sum.data_points {
                            let Some(number_data_point::Value::AsInt(value)) = point.value else {
                                unreachable!()
                            };
                            points.push(Point {
                                name: metric.name.clone(),
                                labels: labels(point.attributes),
                                value: value as u64,
                            });
                        }
                    }
                    metric::Data::Histogram(histogram) => {
                        for point in histogram.data_points {
                            points.push(Point {
                                name: metric.name.clone(),
                                labels: labels(point.attributes),
                                value: point.count,
                            });
                        }
                    }
                    _ => unreachable!("only the admitted HTTP schema"),
                }
            }
        }
    }
    let mut summary = collector.summary.lock().unwrap();
    summary.batches = summary.batches.saturating_add(1);
    summary.wire_bytes = summary.wire_bytes.saturating_add(body.len() as u64);
    summary.authorization_verified = true;
    summary.points = points; // Latest bounded batch, never an accumulating cache.
    drop(summary);
    empty(StatusCode::NO_CONTENT)
}
fn labels(attributes: Vec<KeyValue>) -> BTreeMap<String, String> {
    attributes
        .into_iter()
        .map(|attribute| {
            let Some(any_value::Value::StringValue(value)) = attribute.value.and_then(|value| value.value) else {
                unreachable!()
            };
            (attribute.key, value)
        })
        .collect()
}
fn empty(status: StatusCode) -> hyper::Response<crate::RespBody> {
    crate::response(status, Bytes::new(), "text/plain")
}

#[cfg(test)]
mod tests;
