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

/// One HTTP request, as a collector on loopback received it.
struct Received {
    request_line: String,
    body: Vec<u8>,
}

/// Accept one request on loopback, answer 200, and hand it back.
fn one_request_collector() -> (String, std::sync::mpsc::Receiver<Received>) {
    use std::io::{BufRead, BufReader, Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut length = 0;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header.trim().is_empty() {
                break;
            }
            if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut stream = stream;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        tx.send(Received { request_line, body }).unwrap();
    });
    (base, rx)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle.as_bytes())
}

/// The whole export path: the real provider and HTTP client deliver an OTLP
/// request to the corp endpoint's `/v1/metrics`, carrying the reporting
/// service and what was recorded. Only the pure helpers were tested before,
/// so nothing proved a metric ever left the process.
#[test]
fn a_corp_destination_receives_recorded_metrics_over_otlp_http() {
    let (base, received) = one_request_collector();
    let provider = provider(
        &Destination::Corp(base),
        "capsem-export-test",
        vec![KeyValue::new("capsem.session", "sess-1")],
    )
    .unwrap();
    let exporter = Exporter { provider };
    let recorder = OtelRecorder::new(exporter.meter());
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(crate::db::DB_WRITE_OPS_TOTAL, "insert_type" => "net_event").increment(3);
    });
    exporter.flush().unwrap();

    let request = received.recv_timeout(EXPORT_TIMEOUT).unwrap();
    assert!(
        request.request_line.starts_with("POST /v1/metrics "),
        "{}",
        request.request_line
    );
    for expected in ["capsem-export-test", "sess-1", crate::db::DB_WRITE_OPS_TOTAL] {
        assert!(contains(&request.body, expected), "the OTLP body lacks {expected}");
    }
}

#[test]
fn install_errors_say_what_failed() {
    assert_eq!(
        InstallError::Build("no TLS".into()).to_string(),
        "build the OTLP metric exporter: no TLS"
    );
    assert_eq!(
        InstallError::RecorderAlreadySet.to_string(),
        "a metrics recorder is already installed in this process"
    );
}

/// `install` routes the process's metrics facade to export, once: a second
/// exporter in the same process would split its metrics between two
/// recorders, so it is refused rather than silently ignored.
#[test]
fn install_takes_the_facade_once_and_refuses_a_second_exporter() {
    let (base, _received) = one_request_collector();
    let first = install(&Destination::Corp(base.clone()), "capsem-install-test", vec![]);
    assert!(first.is_ok(), "{:?}", first.err());
    let second = install(&Destination::Corp(base), "capsem-install-test", vec![]);
    assert!(
        matches!(second, Err(InstallError::RecorderAlreadySet)),
        "{:?}",
        second.err()
    );
    // Keep the provider alive past both calls, then let Drop shut it down.
    drop(first);
}

/// The unit a collector sees is the UCUM code, not the facade's name for it.
#[test]
fn every_facade_unit_maps_to_its_exact_ucum_code() {
    let expected = [
        (Unit::Count, "{count}"),
        (Unit::Percent, "%"),
        (Unit::Seconds, "s"),
        (Unit::Milliseconds, "ms"),
        (Unit::Microseconds, "us"),
        (Unit::Nanoseconds, "ns"),
        (Unit::Bytes, "By"),
        (Unit::Kibibytes, "KiBy"),
        (Unit::Mebibytes, "MiBy"),
        (Unit::Gibibytes, "GiBy"),
        (Unit::Tebibytes, "TiBy"),
        (Unit::BitsPerSecond, "bit/s"),
        (Unit::KilobitsPerSecond, "kbit/s"),
        (Unit::MegabitsPerSecond, "Mbit/s"),
        (Unit::GigabitsPerSecond, "Gbit/s"),
        (Unit::TerabitsPerSecond, "Tbit/s"),
        (Unit::CountPerSecond, "{count}/s"),
    ];
    for (unit, code) in expected {
        assert_eq!(ucum(unit), code, "{unit:?}");
    }
}

/// A name the catalog does not know still exports, undescribed; a catalog
/// histogram carries its unit; a gauge moves up as well as down.
#[test]
fn uncataloged_names_units_and_gauge_increments_reach_the_exporter() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let recorder = OtelRecorder::new(provider.meter("capsem"));
    let histogram = crate::all()
        .find(|spec| spec.kind == crate::MetricKind::Histogram && spec.unit.is_some())
        .expect("the catalog has a histogram with a unit");
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!("capsem.test.uncataloged_total").increment(1);
        metrics::gauge!(crate::db::DB_MEMORY_UNFLUSHED_OPS).increment(4.0);
        metrics::histogram!(histogram.name).record(2.0);
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
            .unwrap_or_else(|| panic!("{name} was not exported"))
    };
    assert_eq!(find("capsem.test.uncataloged_total").description(), "");
    assert_eq!(find(histogram.name).unit(), ucum(histogram.unit.unwrap()));
    let AggregatedMetrics::F64(MetricData::Gauge(gauge)) = find(crate::db::DB_MEMORY_UNFLUSHED_OPS).data() else {
        panic!("the unflushed-ops gauge exports as an f64 gauge");
    };
    assert_eq!(gauge.data_points().next().unwrap().value(), 4.0);
}
