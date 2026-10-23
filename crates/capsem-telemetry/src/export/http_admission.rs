//! Admission for the confined gateway's normalized HTTP metric export.
//!
//! This is not a general OTLP relay. Reject unknown/duplicate wire fields before
//! generated decoding; accept only the finite resource, instrument and label
//! schema owned by our HTTP adapter. Errors contain no payload-derived text.

use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::{any_value::Value, KeyValue},
    metrics::v1::{metric::Data, number_data_point, HistogramDataPoint, Metric},
};
use prost::Message;

use crate::http::{operations, BODY_LIFETIME_MS, REQUESTS_TOTAL, RESPONSES_TOTAL, RESPONSE_READY_MS, SPECS};

#[cfg(test)]
mod tests;
mod wire;

pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_POINTS: usize = 4096;

/// Deliberately does not reveal rejected attribute values, paths or secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionError;

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("gateway HTTP metric payload rejected")
    }
}
impl std::error::Error for AdmissionError {}

type Result<T> = std::result::Result<T, AdmissionError>;

/// Validates borrowed bytes without collecting or rewriting the original body.
/// Owned protobuf allocation occurs only after the finite wire preflight.
pub fn validate(body: &[u8]) -> Result<()> {
    ensure(!body.is_empty() && body.len() <= MAX_BODY_BYTES)?;
    wire::check(body)?;
    let request = ExportMetricsServiceRequest::decode(body).map_err(|_| AdmissionError)?;
    let [resource_metrics] = request.resource_metrics.as_slice() else {
        return Err(AdmissionError);
    };
    ensure(resource_metrics.schema_url.is_empty())?;
    let resource = resource_metrics.resource.as_ref().ok_or(AdmissionError)?;
    ensure(resource.dropped_attributes_count == 0)?;
    let attributes = attributes(&resource.attributes)?;
    ensure(attributes.as_slice() == [("service.name", "capsem-gateway")])?;
    let [scope_metrics] = resource_metrics.scope_metrics.as_slice() else {
        return Err(AdmissionError);
    };
    ensure(scope_metrics.schema_url.is_empty())?;
    let scope = scope_metrics.scope.as_ref().ok_or(AdmissionError)?;
    ensure(
        scope.name == "capsem"
            && scope.version.is_empty()
            && scope.attributes.is_empty()
            && scope.dropped_attributes_count == 0,
    )?;
    ensure(!scope_metrics.metrics.is_empty())?;
    let mut names = Vec::with_capacity(4);
    for metric in &scope_metrics.metrics {
        ensure(!names.contains(&metric.name.as_str()))?;
        names.push(metric.name.as_str());
        validate_metric(metric)?;
    }
    Ok(())
}

fn validate_metric(metric: &Metric) -> Result<()> {
    let spec = SPECS
        .iter()
        .find(|spec| spec.name == metric.name)
        .ok_or(AdmissionError)?;
    ensure(metric.description == spec.description && metric.unit == super::ucum(spec.unit.ok_or(AdmissionError)?))?;
    match metric.data.as_ref().ok_or(AdmissionError)? {
        Data::Sum(sum) => {
            ensure(
                matches!(metric.name.as_str(), REQUESTS_TOTAL | RESPONSES_TOTAL)
                    && sum.is_monotonic
                    && matches!(sum.aggregation_temporality, 1 | 2)
                    && !sum.data_points.is_empty(),
            )?;
            for point in &sum.data_points {
                ensure(point.flags == 0 && point.exemplars.is_empty())?;
                ensure(matches!(point.value, Some(number_data_point::Value::AsInt(value)) if value >= 0))?;
                labels(&metric.name, &point.attributes)?;
            }
        }
        Data::Histogram(histogram) => {
            ensure(
                matches!(metric.name.as_str(), RESPONSE_READY_MS | BODY_LIFETIME_MS)
                    && matches!(histogram.aggregation_temporality, 1 | 2)
                    && !histogram.data_points.is_empty(),
            )?;
            for point in &histogram.data_points {
                histogram_point(point)?;
                labels(&metric.name, &point.attributes)?;
            }
        }
        _ => return Err(AdmissionError),
    }
    Ok(())
}

fn histogram_point(point: &HistogramDataPoint) -> Result<()> {
    ensure(point.flags == 0 && point.exemplars.is_empty())?;
    ensure(point.explicit_bounds.len() <= 64 && point.bucket_counts.len() == point.explicit_bounds.len() + 1)?;
    ensure(
        point.explicit_bounds.iter().all(|n| n.is_finite() && *n >= 0.0)
            && point.explicit_bounds.windows(2).all(|pair| pair[0] < pair[1]),
    )?;
    ensure(
        point
            .bucket_counts
            .iter()
            .try_fold(0_u64, |sum, value| sum.checked_add(*value))
            == Some(point.count),
    )?;
    ensure(
        [point.sum, point.min, point.max]
            .iter()
            .flatten()
            .all(|n| n.is_finite() && *n >= 0.0),
    )?;
    ensure(point.min.zip(point.max).is_none_or(|(min, max)| min <= max))
}

fn attributes(values: &[KeyValue]) -> Result<Vec<(&str, &str)>> {
    let mut result = Vec::with_capacity(values.len());
    for attribute in values {
        ensure(attribute.key_strindex == 0 && !result.iter().any(|(key, _)| *key == attribute.key))?;
        let Some(Value::StringValue(value)) = attribute.value.as_ref().and_then(|v| v.value.as_ref()) else {
            return Err(AdmissionError);
        };
        result.push((attribute.key.as_str(), value.as_str()));
    }
    Ok(result)
}

fn labels(name: &str, values: &[KeyValue]) -> Result<()> {
    let attrs = attributes(values)?;
    let get = |key| attrs.iter().find_map(|(k, value)| (*k == key).then_some(*value));
    ensure(get("component") == Some("gateway"))?;
    ensure(matches!(
        get("method"),
        Some("GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "CONNECT" | "TRACE" | "OTHER")
    ))?;
    let route = get("route").ok_or(AdmissionError)?;
    ensure(route == "unmatched" || operations::ROUTES.binary_search(&route).is_ok())?;
    if name == REQUESTS_TOTAL {
        return ensure(attrs.len() == 3);
    }
    let status = get("status").ok_or(AdmissionError)?;
    let is_status = status.len() == 3
        && status.as_bytes()[0].is_ascii_digit()
        && status
            .parse::<u16>()
            .is_ok_and(|n| (100..=599).contains(&n) && n.to_string() == status);
    if name == RESPONSE_READY_MS {
        return ensure(attrs.len() == 4 && is_status);
    }
    let completion = get("completion").ok_or(AdmissionError)?;
    ensure(attrs.len() == 5)?;
    match completion {
        "cancelled" => ensure(name == RESPONSES_TOTAL && status == "none"),
        "upgraded" => ensure(name == RESPONSES_TOTAL && status == "101"),
        "eof" | "dropped" | "body_error" => ensure(is_status && status != "101"),
        _ => Err(AdmissionError),
    }
}

fn ensure(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(AdmissionError)
    }
}
