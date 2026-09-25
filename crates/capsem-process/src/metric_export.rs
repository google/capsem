//! Metric export for one VM process.
//!
//! The process exports its own measurements -- ledger writer, MITM, DNS,
//! security engine, virtio-blk -- tagged with the session it serves, to the
//! corp config's `open_telemetry` endpoint and nowhere else. The service
//! resolves that endpoint and grants it at launch (`--metric-endpoint`): a
//! process takes its runtime config from what the service hands it, never
//! from settings or corp files of its own reading. It never reads
//! `OTEL_EXPORTER_OTLP_*` either: that environment can carry collector
//! credentials, and the service's spawn allowlist keeps it out of this
//! guest-facing process. Authenticated collectors are out of scope here.

use capsem_telemetry::export::{Destination, Exporter, KeyValue};
use tracing::{info, warn};

/// Install export when the service granted an endpoint. Failure is logged
/// and leaves export off; metrics are never a reason a VM does not boot.
pub(crate) fn install(vm_id: &str, endpoint: Option<&str>) -> Option<Exporter> {
    let destination = Destination::corp(endpoint)?;
    match capsem_telemetry::export::install(
        &destination,
        "capsem-process",
        vec![KeyValue::new("session.id", vm_id.to_string())],
    ) {
        Ok(exporter) => {
            info!(?destination, "metric export installed");
            Some(exporter)
        }
        Err(error) => {
            warn!(%error, ?destination, "metric export not installed");
            None
        }
    }
}
