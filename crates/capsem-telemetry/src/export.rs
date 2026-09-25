//! OTLP/HTTP export of the metrics this crate names.
//!
//! Off unless asked for. Export is enabled by the corp config's
//! `open_telemetry` (an OTLP/HTTP base endpoint) or by the standard
//! `OTEL_EXPORTER_OTLP_*` environment, which wins when both are set. With
//! neither, nothing is installed: every `counter!` stays a no-op and no
//! connection is ever opened.
//!
//! When enabled, a `metrics` recorder forwards each counter, gauge and
//! histogram to an OpenTelemetry instrument, and a periodic reader on its own
//! thread exports them as protobuf over HTTP. The transport is our own client
//! on the workspace reqwest (rustls with ring): no gRPC stack and no second
//! TLS provider come with it.
//!
//! Every process that records metrics installs this itself -- the service
//! and each `capsem-process` -- tagged with its own resource attributes, so
//! nothing has to relay one process's measurements through another.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder, SharedString, Unit,
};
use opentelemetry::metrics::MeterProvider as _;
/// The OpenTelemetry types a caller registering its own instruments needs.
pub use opentelemetry::{metrics::Meter, KeyValue};
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::Resource;

/// How long one export request may take before it is abandoned.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// The environment variables that turn export on without a corp config.
const OTEL_ENDPOINT_ENV: [&str; 2] = ["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"];

/// Every `OTEL_EXPORTER_OTLP_*` variable a child process needs to export the
/// way its parent does. The exporter reads them itself.
pub const OTEL_EXPORT_ENV: [&str; 8] = [
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    "OTEL_EXPORTER_OTLP_TIMEOUT",
    "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT",
    "OTEL_EXPORTER_OTLP_COMPRESSION",
    "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE",
];

/// Where metrics go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The `OTEL_EXPORTER_OTLP_*` environment decides, read by the exporter.
    Environment,
    /// The corp config's OTLP/HTTP base endpoint; metrics go to `/v1/metrics`.
    Corp(String),
}

impl Destination {
    /// The destination this process should export to, or `None` to export
    /// nothing. `env` is the process environment, passed in to be testable.
    pub fn resolve(corp_endpoint: Option<&str>, env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if OTEL_ENDPOINT_ENV
            .iter()
            .any(|name| env(name).is_some_and(|value| !value.trim().is_empty()))
        {
            return Some(Self::Environment);
        }
        Self::corp(corp_endpoint)
    }

    /// The corp endpoint alone, ignoring the environment.
    ///
    /// For a guest-facing process: the `OTEL_EXPORTER_OTLP_*` environment can
    /// carry collector credentials, which the service never forwards into
    /// one, so the corp config -- a file it already trusts -- is the only
    /// place its destination may come from.
    pub fn corp(endpoint: Option<&str>) -> Option<Self> {
        endpoint
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| Self::Corp(endpoint.trim_end_matches('/').to_string()))
    }
}

/// An installed exporter. Dropping it flushes what was recorded and stops
/// the export thread; keep it for the life of the process.
pub struct Exporter {
    provider: SdkMeterProvider,
}

impl Exporter {
    /// The meter for instruments a caller registers itself, such as the
    /// service's observable session gauges.
    pub fn meter(&self) -> Meter {
        self.provider.meter("capsem")
    }

    /// Export everything recorded so far, now.
    pub fn flush(&self) -> Result<(), String> {
        self.provider.force_flush().map_err(|error| error.to_string())
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

/// Why export could not be installed.
#[derive(Debug)]
pub enum InstallError {
    Build(String),
    RecorderAlreadySet,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => write!(f, "build the OTLP metric exporter: {error}"),
            Self::RecorderAlreadySet => write!(f, "a metrics recorder is already installed in this process"),
        }
    }
}

impl std::error::Error for InstallError {}

/// Install OTLP export for this process and route the `metrics` facade to it.
///
/// `service_name` and `attributes` become the exported resource: who is
/// reporting (`capsem-service`, `capsem-process`) and, for a VM process, the
/// session it serves. They are the only attributes every series carries.
pub fn install(
    destination: &Destination,
    service_name: &'static str,
    attributes: Vec<KeyValue>,
) -> Result<Exporter, InstallError> {
    let provider = provider(destination, service_name, attributes)?;
    let recorder = OtelRecorder::new(provider.meter("capsem"));
    metrics::set_global_recorder(recorder).map_err(|_| InstallError::RecorderAlreadySet)?;
    crate::describe_all();
    Ok(Exporter { provider })
}

fn provider(
    destination: &Destination,
    service_name: &'static str,
    attributes: Vec<KeyValue>,
) -> Result<SdkMeterProvider, InstallError> {
    let client = BlockingHttpClient::new().map_err(InstallError::Build)?;
    let mut builder = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_http_client(client)
        .with_timeout(EXPORT_TIMEOUT);
    if let Destination::Corp(base) = destination {
        builder = builder.with_endpoint(format!("{base}/v1/metrics"));
    }
    let exporter = builder
        .build()
        .map_err(|error| InstallError::Build(error.to_string()))?;
    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attributes(attributes)
        .build();
    Ok(SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource)
        .build())
}

/// The OTLP HTTP transport: the workspace reqwest, blocking, on its own
/// runtime. The periodic reader calls it from its export thread, never from
/// a caller's async runtime.
#[derive(Debug)]
struct BlockingHttpClient(reqwest::blocking::Client);

impl BlockingHttpClient {
    fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            .timeout(EXPORT_TIMEOUT)
            .build()
            .map(Self)
            .map_err(|error| format!("build the OTLP HTTP client: {error}"))
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for BlockingHttpClient {
    async fn send_bytes(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        let (parts, body) = request.into_parts();
        let mut outgoing = self.0.request(parts.method, parts.uri.to_string()).body(body.to_vec());
        for (name, value) in &parts.headers {
            outgoing = outgoing.header(name, value);
        }
        let response = outgoing.send()?;
        let mut incoming = http::Response::builder().status(response.status());
        for (name, value) in response.headers() {
            incoming = incoming.header(name, value);
        }
        Ok(incoming.body(response.bytes()?)?)
    }
}

/// The `metrics` facade, forwarded to OpenTelemetry instruments.
///
/// One instrument per metric name, built on first use with the unit and
/// description this crate's catalog gives it, and one handle per facade key
/// (name and labels). The facade may register the same key again on every
/// macro call, so a handle's state -- a gauge's running value, a counter's
/// last absolute -- has to live with the key, not with one registration.
pub(crate) struct OtelRecorder {
    meter: Meter,
    counters: Mutex<HashMap<Key, Arc<OtelCounter>>>,
    gauges: Mutex<HashMap<Key, Arc<OtelGauge>>>,
    histograms: Mutex<HashMap<Key, Arc<OtelHistogram>>>,
}

impl OtelRecorder {
    pub(crate) fn new(meter: Meter) -> Self {
        Self {
            meter,
            counters: Mutex::default(),
            gauges: Mutex::default(),
            histograms: Mutex::default(),
        }
    }
}

/// The catalog's unit and description for `name`, as OpenTelemetry spells
/// them. A name the catalog does not know still exports, undescribed.
fn described<'a, T>(
    builder: opentelemetry::metrics::InstrumentBuilder<'a, T>,
    name: &str,
) -> opentelemetry::metrics::InstrumentBuilder<'a, T> {
    match crate::all().find(|spec| spec.name == name) {
        Some(spec) => {
            let builder = builder.with_description(spec.description);
            match spec.unit {
                Some(unit) => builder.with_unit(ucum(unit)),
                None => builder,
            }
        }
        None => builder,
    }
}

/// The UCUM code OpenTelemetry expects for a facade unit.
fn ucum(unit: Unit) -> &'static str {
    match unit {
        Unit::Count => "{count}",
        Unit::Percent => "%",
        Unit::Seconds => "s",
        Unit::Milliseconds => "ms",
        Unit::Microseconds => "us",
        Unit::Nanoseconds => "ns",
        Unit::Bytes => "By",
        Unit::Kibibytes => "KiBy",
        Unit::Mebibytes => "MiBy",
        Unit::Gibibytes => "GiBy",
        Unit::Tebibytes => "TiBy",
        Unit::BitsPerSecond => "bit/s",
        Unit::KilobitsPerSecond => "kbit/s",
        Unit::MegabitsPerSecond => "Mbit/s",
        Unit::GigabitsPerSecond => "Gbit/s",
        Unit::TerabitsPerSecond => "Tbit/s",
        Unit::CountPerSecond => "{count}/s",
    }
}

fn attributes(key: &Key) -> Vec<KeyValue> {
    key.labels()
        .map(|label| KeyValue::new(label.key().to_string(), label.value().to_string()))
        .collect()
}

/// The handle for `key`, registering it on first use.
fn handle<T>(handles: &Mutex<HashMap<Key, Arc<T>>>, key: &Key, build: impl FnOnce() -> T) -> Arc<T> {
    let mut handles = handles.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(handles.entry(key.clone()).or_insert_with(|| Arc::new(build())))
}

impl Recorder for OtelRecorder {
    // Metadata comes from the catalog, which the recorder reads directly.
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        Counter::from_arc(handle(&self.counters, key, || OtelCounter {
            counter: described(self.meter.u64_counter(key.name().to_string()), key.name()).build(),
            attributes: attributes(key),
            last_absolute: AtomicU64::new(0),
        }))
    }

    fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::from_arc(handle(&self.gauges, key, || OtelGauge {
            gauge: described(self.meter.f64_gauge(key.name().to_string()), key.name()).build(),
            attributes: attributes(key),
            current: AtomicU64::new(0f64.to_bits()),
        }))
    }

    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(handle(&self.histograms, key, || {
            // Histograms have a builder of their own, with the same two knobs.
            let mut builder = self.meter.f64_histogram(key.name().to_string());
            if let Some(spec) = crate::all().find(|spec| spec.name == key.name()) {
                builder = builder.with_description(spec.description);
                if let Some(unit) = spec.unit {
                    builder = builder.with_unit(ucum(unit));
                }
            }
            OtelHistogram {
                histogram: builder.build(),
                attributes: attributes(key),
            }
        }))
    }
}

struct OtelCounter {
    counter: opentelemetry::metrics::Counter<u64>,
    attributes: Vec<KeyValue>,
    /// The last absolute value set, so an absolute set exports as the delta
    /// OpenTelemetry's monotonic counter adds.
    last_absolute: AtomicU64,
}

impl CounterFn for OtelCounter {
    fn increment(&self, value: u64) {
        self.counter.add(value, &self.attributes);
    }

    fn absolute(&self, value: u64) {
        let previous = self.last_absolute.fetch_max(value, Ordering::AcqRel);
        if value > previous {
            self.counter.add(value - previous, &self.attributes);
        }
    }
}

struct OtelGauge {
    gauge: opentelemetry::metrics::Gauge<f64>,
    attributes: Vec<KeyValue>,
    /// The facade can move a gauge by a delta; OpenTelemetry records values.
    current: AtomicU64,
}

impl OtelGauge {
    fn update(&self, next: impl Fn(f64) -> f64) {
        let mut current = self.current.load(Ordering::Acquire);
        loop {
            let value = next(f64::from_bits(current));
            match self
                .current
                .compare_exchange_weak(current, value.to_bits(), Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    self.gauge.record(value, &self.attributes);
                    return;
                }
                Err(actual) => current = actual,
            }
        }
    }
}

impl GaugeFn for OtelGauge {
    fn increment(&self, value: f64) {
        self.update(|current| current + value);
    }

    fn decrement(&self, value: f64) {
        self.update(|current| current - value);
    }

    fn set(&self, value: f64) {
        self.update(|_| value);
    }
}

struct OtelHistogram {
    histogram: opentelemetry::metrics::Histogram<f64>,
    attributes: Vec<KeyValue>,
}

impl HistogramFn for OtelHistogram {
    fn record(&self, value: f64) {
        self.histogram.record(value, &self.attributes);
    }
}

#[cfg(test)]
mod tests;
