//! Metric export for one VM process.
//!
//! The process exports its own measurements -- ledger writer, MITM, DNS,
//! security engine, virtio-blk -- tagged with the session it serves, to the
//! corp config's `open_telemetry` endpoint and nowhere else. It never reads
//! `OTEL_EXPORTER_OTLP_*`: that environment can carry collector credentials,
//! and the service's spawn allowlist keeps it out of this guest-facing
//! process. Authenticated collectors are out of scope here.
//!
//! When #206 confines this process, the endpoint becomes an upstream granted
//! at launch -- the one telemetry destination it may dial -- rather than
//! something it resolves for itself.

use capsem_telemetry::export::{Destination, Exporter, KeyValue};
use tracing::{info, warn};

/// Install export when the corp config names an endpoint. Failure is logged
/// and leaves export off; metrics are never a reason a VM does not boot.
pub(crate) fn install(vm_id: &str) -> Option<Exporter> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let destination = Destination::corp(corp.corp_rule_files.open_telemetry.as_deref())?;
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
