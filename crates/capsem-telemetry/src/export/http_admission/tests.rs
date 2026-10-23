use super::*;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::{any_value::Value, AnyValue, InstrumentationScope, KeyValue},
    metrics::v1::{metric::Data, number_data_point, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum},
    resource::v1::Resource,
};
use prost::Message;

fn attribute(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        key_strindex: 0,
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.into())),
        }),
    }
}

fn fixture() -> ExportMetricsServiceRequest {
    let spec = crate::http::SPECS
        .iter()
        .find(|spec| spec.name == crate::http::REQUESTS_TOTAL)
        .unwrap();
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
                    unit: super::super::ucum(spec.unit.unwrap()).into(),
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
}

#[test]
fn accepts_the_bounded_normalized_gateway_schema() {
    validate(&fixture().encode_to_vec()).unwrap();
}

#[test]
fn rejects_foreign_resource_and_arbitrary_metadata() {
    for (key, value) in [("service.name", "capsem-service"), ("secret-canary", "private-value")] {
        let mut request = fixture();
        request.resource_metrics[0]
            .resource
            .as_mut()
            .unwrap()
            .attributes
            .push(attribute(key, value));
        assert!(validate(&request.encode_to_vec()).is_err());
    }
    let mut request = fixture();
    request.resource_metrics[0].scope_metrics[0]
        .scope
        .as_mut()
        .unwrap()
        .version = "secret-version-canary".into();
    assert!(validate(&request.encode_to_vec()).is_err());
}

#[test]
fn rejects_unregistered_paths_and_custom_methods_even_when_shape_is_valid() {
    for (key, value) in [
        ("route", "/vms/private-id-canary/info"),
        ("method", "SECRET-METHOD"),
        ("component", "service"),
        ("token", "private-token-canary"),
    ] {
        let mut request = fixture();
        let Some(Data::Sum(sum)) = &mut request.resource_metrics[0].scope_metrics[0].metrics[0].data else {
            panic!()
        };
        if let Some(old) = sum.data_points[0].attributes.iter_mut().find(|old| old.key == key) {
            *old = attribute(key, value);
        } else {
            sum.data_points[0].attributes.push(attribute(key, value));
        }
        assert!(validate(&request.encode_to_vec()).is_err());
    }
}

#[test]
fn rejects_unknown_wire_fields_duplicate_singulars_and_excessive_points_before_owned_decode() {
    let mut unknown = fixture().encode_to_vec();
    // Unknown length-delimited root field must not become an opaque exfiltration channel.
    unknown.extend_from_slice(&[0x12, 0x06, b's', b'e', b'c', b'r', b'e', b't']);
    assert!(validate(&unknown).is_err());
    let mut request = fixture();
    let Some(Data::Sum(sum)) = &mut request.resource_metrics[0].scope_metrics[0].metrics[0].data else {
        panic!()
    };
    sum.data_points = vec![sum.data_points[0].clone(); MAX_POINTS + 1];
    assert!(validate(&request.encode_to_vec()).is_err());
    assert!(validate(&vec![0; MAX_BODY_BYTES + 1]).is_err());
}

fn metric(request: &mut ExportMetricsServiceRequest) -> &mut Metric {
    &mut request.resource_metrics[0].scope_metrics[0].metrics[0]
}

fn sum(request: &mut ExportMetricsServiceRequest) -> &mut Sum {
    let Some(Data::Sum(sum)) = &mut metric(request).data else {
        panic!()
    };
    sum
}

fn set_label(request: &mut ExportMetricsServiceRequest, key: &str, value: &str) {
    let attributes = &mut sum(request).data_points[0].attributes;
    if let Some(old) = attributes.iter_mut().find(|old| old.key == key) {
        *old = attribute(key, value);
    } else {
        attributes.push(attribute(key, value));
    }
}

fn rename(request: &mut ExportMetricsServiceRequest, name: &'static str) {
    let spec = crate::http::SPECS.iter().find(|spec| spec.name == name).unwrap();
    let metric = metric(request);
    metric.name = name.into();
    metric.description = spec.description.into();
    metric.unit = super::super::ucum(spec.unit.unwrap()).into();
}

#[test]
fn instrument_metadata_and_value_kinds_cannot_carry_arbitrary_payloads() {
    let cases: &[fn(&mut ExportMetricsServiceRequest)] = &[
        |r| metric(r).name = "secret-instrument".into(),
        |r| metric(r).unit = "secret-unit".into(),
        |r| metric(r).description = "secret-description".into(),
        |r| metric(r).metadata.push(attribute("secret", "value")),
        |r| sum(r).is_monotonic = false,
        |r| sum(r).aggregation_temporality = 0,
        |r| sum(r).data_points[0].value = Some(number_data_point::Value::AsDouble(1.0)),
        |r| sum(r).data_points[0].value = Some(number_data_point::Value::AsInt(-1)),
        |r| sum(r).data_points[0].flags = 1,
        |r| sum(r).data_points[0].exemplars.push(Default::default()),
        |r| sum(r).data_points[0].attributes[0].key_strindex = 1,
        |r| {
            sum(r).data_points[0].attributes[0].value.as_mut().unwrap().value =
                Some(Value::BytesValue(b"secret".to_vec()))
        },
        |r| {
            r.resource_metrics[0]
                .resource
                .as_mut()
                .unwrap()
                .entity_refs
                .push(Default::default())
        },
        |r| {
            r.resource_metrics[0].scope_metrics[0]
                .scope
                .as_mut()
                .unwrap()
                .attributes
                .push(attribute("secret", "value"))
        },
        |r| r.resource_metrics[0].schema_url = "https://secret.invalid".into(),
        |r| r.resource_metrics[0].scope_metrics[0].schema_url = "https://secret.invalid".into(),
    ];
    for change in cases {
        let mut request = fixture();
        change(&mut request);
        assert_eq!(validate(&request.encode_to_vec()), Err(AdmissionError));
    }
    let mut request = fixture();
    sum(&mut request).data_points[0]
        .attributes
        .push(attribute("method", "GET"));
    assert!(validate(&request.encode_to_vec()).is_err());
    let mut request = fixture();
    let duplicate = metric(&mut request).clone();
    request.resource_metrics[0].scope_metrics[0].metrics.push(duplicate);
    assert!(validate(&request.encode_to_vec()).is_err());
}

#[test]
fn status_and_completion_labels_are_finite_and_consistent() {
    for (status, completion, valid) in [
        ("200", "eof", true),
        ("599", "body_error", true),
        ("404", "dropped", true),
        ("101", "upgraded", true),
        ("none", "cancelled", true),
        ("none", "eof", false),
        ("200", "cancelled", false),
        ("200", "upgraded", false),
        ("101", "eof", false),
        ("099", "eof", false),
        ("600", "eof", false),
        ("+200", "eof", false),
        ("secret", "eof", false),
        ("200", "private-canary", false),
    ] {
        let mut request = fixture();
        rename(&mut request, crate::http::RESPONSES_TOTAL);
        set_label(&mut request, "status", status);
        set_label(&mut request, "completion", completion);
        assert_eq!(
            validate(&request.encode_to_vec()).is_ok(),
            valid,
            "{status}/{completion}"
        );
    }
    for method in [
        "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT", "OTHER",
    ] {
        let mut request = fixture();
        set_label(&mut request, "method", method);
        set_label(&mut request, "route", "unmatched");
        validate(&request.encode_to_vec()).unwrap();
    }
}

fn histogram_fixture() -> ExportMetricsServiceRequest {
    use opentelemetry_proto::tonic::metrics::v1::{Histogram, HistogramDataPoint};
    let mut request = fixture();
    rename(&mut request, crate::http::RESPONSE_READY_MS);
    set_label(&mut request, "status", "200");
    let attributes = sum(&mut request).data_points[0].attributes.clone();
    metric(&mut request).data = Some(Data::Histogram(Histogram {
        aggregation_temporality: 2,
        data_points: vec![HistogramDataPoint {
            attributes,
            count: 1,
            sum: Some(1.5),
            min: Some(1.5),
            max: Some(1.5),
            explicit_bounds: vec![1.0, 2.0],
            bucket_counts: vec![0, 1, 0],
            ..Default::default()
        }],
    }));
    request
}

#[test]
fn histogram_values_buckets_flags_and_completion_are_bounded() {
    use opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint;
    validate(&histogram_fixture().encode_to_vec()).unwrap();
    let cases: &[fn(&mut HistogramDataPoint)] = &[
        |p| p.explicit_bounds = vec![0.0; 65],
        |p| p.explicit_bounds = vec![2.0, 1.0],
        |p| p.explicit_bounds = vec![1.0, f64::NAN],
        |p| p.explicit_bounds[0] = -1.0,
        |p| p.bucket_counts.pop().map(drop).unwrap(),
        |p| p.bucket_counts = vec![u64::MAX, 1, 1],
        |p| p.count = 2,
        |p| p.sum = Some(f64::INFINITY),
        |p| p.min = Some(-1.0),
        |p| p.max = Some(1.0),
        |p| p.flags = 1,
        |p| p.exemplars.push(Default::default()),
    ];
    for change in cases {
        let mut request = histogram_fixture();
        let Some(Data::Histogram(histogram)) = &mut metric(&mut request).data else {
            panic!()
        };
        change(&mut histogram.data_points[0]);
        assert!(validate(&request.encode_to_vec()).is_err());
    }
    let mut request = histogram_fixture();
    rename(&mut request, crate::http::BODY_LIFETIME_MS);
    let Some(Data::Histogram(histogram)) = &mut metric(&mut request).data else {
        panic!()
    };
    histogram.data_points[0].attributes.push(attribute("completion", "eof"));
    validate(&request.encode_to_vec()).unwrap();
    let Some(Data::Histogram(histogram)) = &mut metric(&mut request).data else {
        panic!()
    };
    histogram.data_points[0].attributes.last_mut().unwrap().value = attribute("completion", "cancelled").value;
    assert!(validate(&request.encode_to_vec()).is_err());
}

fn wire_message(tag: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    prost::encoding::encode_key(tag, prost::encoding::WireType::LengthDelimited, &mut bytes);
    prost::encoding::encode_varint(payload.len() as u64, &mut bytes);
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn borrowed_preflight_rejects_hidden_fields_duplicates_and_all_truncations() {
    // An earlier secret value must not disappear when prost overwrites a singular field.
    let mut duplicate_name = wire_message(1, b"secret-hidden-name");
    duplicate_name.extend(wire_message(1, crate::http::REQUESTS_TOTAL.as_bytes()));
    let scope_metrics = wire_message(2, &duplicate_name);
    let resource_metrics = wire_message(2, &scope_metrics);
    let encoded = wire_message(1, &resource_metrics);
    assert!(wire::check(&encoded).is_err());
    // Reject repeated zero-length points before constructing any owned points.
    let point_bytes = [0x0a, 0x00].repeat(MAX_POINTS + 1);
    let sum = wire_message(7, &point_bytes);
    let encoded = wire_message(1, &wire_message(2, &wire_message(2, &sum)));
    assert!(wire::check(&encoded).is_err());
    let encoded = fixture().encode_to_vec();
    for end in 0..encoded.len() {
        assert!(validate(&encoded[..end]).is_err(), "truncated at {end}");
    }
    for malformed in [&[0xff; 11][..], &[0x0a, 0xff, 0xff, 0xff, 0xff, 0x7f], &[0x0b, 0x0c]] {
        assert!(validate(malformed).is_err());
    }
    assert_eq!(AdmissionError.to_string(), "gateway HTTP metric payload rejected");
}
