use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

use super::*;

fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn export_is_off_without_a_corp_endpoint_or_otel_environment() {
    assert_eq!(Destination::resolve(None, env_of(&[])), None);
    assert_eq!(
        Destination::resolve(Some("  "), env_of(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "")])),
        None
    );
    // Headers alone name no destination.
    assert_eq!(
        Destination::resolve(None, env_of(&[("OTEL_EXPORTER_OTLP_HEADERS", "a=b")])),
        None
    );
}

#[test]
fn the_corp_endpoint_is_a_base_and_the_environment_wins() {
    assert_eq!(
        Destination::resolve(Some("https://otel.example/ "), env_of(&[])),
        Some(Destination::Corp("https://otel.example".into()))
    );
    for var in ["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"] {
        assert_eq!(
            Destination::resolve(Some("https://corp.example"), env_of(&[(var, "http://collector:4318")])),
            Some(Destination::Environment),
            "{var}"
        );
    }
}

#[test]
fn a_corp_only_destination_ignores_the_environment() {
    assert_eq!(Destination::corp(None), None);
    assert_eq!(Destination::corp(Some(" ")), None);
    assert_eq!(
        Destination::corp(Some("https://otel.example/")),
        Some(Destination::Corp("https://otel.example".into()))
    );
}

/// Every facade shape reaches OpenTelemetry with its value, its labels as
/// attributes, and the catalog's unit and description.
#[test]
fn the_recorder_forwards_counters_gauges_and_histograms() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let recorder = OtelRecorder::new(provider.meter("capsem"));
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(crate::db::DB_WRITE_OPS_TOTAL, "insert_type" => "net_event").increment(3);
        metrics::counter!(crate::db::DB_WRITE_OPS_TOTAL, "insert_type" => "net_event").increment(2);
        metrics::counter!(crate::db::DB_WRITE_OPS_TOTAL, "insert_type" => "file_event").absolute(4);
        metrics::gauge!(crate::db::DB_MEMORY_UNFLUSHED_OPS).set(10.0);
        metrics::gauge!(crate::db::DB_MEMORY_UNFLUSHED_OPS).decrement(3.0);
        metrics::histogram!(crate::db::DB_QUERY_DURATION_MS, "phase" => "read").record(1.5);
        metrics::histogram!(crate::db::DB_QUERY_DURATION_MS, "phase" => "read").record(2.5);
    });
    provider.force_flush().unwrap();

    let exported = exporter.get_finished_metrics().unwrap();
    let metrics: Vec<_> = exported
        .iter()
        .flat_map(|resource| resource.scope_metrics())
        .flat_map(|scope| scope.metrics())
        .collect();
    let find = |name: &str| {
        *metrics
            .iter()
            .find(|metric| metric.name() == name)
            .unwrap_or_else(|| panic!("{name} not exported"))
    };

    let writes = find(crate::db::DB_WRITE_OPS_TOTAL);
    assert!(!writes.description().is_empty());
    let AggregatedMetrics::U64(MetricData::Sum(sum)) = writes.data() else {
        panic!("a counter exports as a u64 sum: {:?}", writes.data());
    };
    let by_type: HashMap<String, u64> = sum
        .data_points()
        .map(|point| (point.attributes().next().unwrap().value.to_string(), point.value()))
        .collect();
    assert_eq!(
        by_type,
        HashMap::from([("net_event".into(), 5), ("file_event".into(), 4)])
    );

    let unflushed = find(crate::db::DB_MEMORY_UNFLUSHED_OPS);
    let AggregatedMetrics::F64(MetricData::Gauge(gauge)) = unflushed.data() else {
        panic!("a gauge exports as an f64 gauge");
    };
    assert_eq!(gauge.data_points().next().unwrap().value(), 7.0);

    let durations = find(crate::db::DB_QUERY_DURATION_MS);
    assert_eq!(durations.unit(), "ms");
    let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = durations.data() else {
        panic!("a histogram exports as an f64 histogram");
    };
    let point = histogram.data_points().next().unwrap();
    assert_eq!((point.count(), point.sum()), (2, 4.0));
}

#[test]
fn every_catalog_unit_has_a_ucum_spelling() {
    for spec in crate::all() {
        if let Some(unit) = spec.unit {
            assert!(!ucum(unit).is_empty(), "{}", spec.name);
        }
    }
}
