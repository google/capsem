//! Metric export for one VM process.
//!
//! The process exports its own measurements -- ledger writer, MITM, DNS,
//! security engine, virtio-blk -- tagged with the session it serves. Broker
//! mode carries encoded OTLP bytes over the service socket without learning
//! a collector address or credential. The direct endpoint mode remains only
//! while the service launch grant moves to the broker in the next commit.

use capsem_telemetry::export::{Destination, Exporter, KeyValue};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::{info, warn};

/// OTLP transport that can reach only this process's fixed service route.
struct ServiceMetricClient {
    service_socket: PathBuf,
    path: String,
    runtime: Mutex<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for ServiceMetricClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceMetricClient")
            .field("service_socket", &self.service_socket)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ServiceMetricClient {
    fn new(service_socket: PathBuf, vm_id: &str) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("build metric broker runtime: {error}"))?;
        Ok(Self {
            service_socket,
            path: format!("/internal/vms/{vm_id}/metrics"),
            runtime: Mutex::new(runtime),
        })
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for ServiceMetricClient {
    async fn send_bytes(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        let response = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .block_on(capsem_core::service_uds::post_bytes(
                &self.service_socket,
                &self.path,
                http::HeaderValue::from_static("application/x-protobuf"),
                request.into_body(),
            ));
        response.map_err(|error| Box::new(std::io::Error::other(format!("metric broker request: {error:#}"))) as _)
    }
}

/// Install export when the service granted broker or transitional direct
/// transport. Failure is logged and leaves export off; metrics never prevent
/// a VM from booting.
pub(crate) fn install(
    vm_id: &str,
    service_socket: Option<&Path>,
    broker: bool,
    endpoint: Option<&str>,
) -> Option<Exporter> {
    let attributes = vec![KeyValue::new("session.id", vm_id.to_string())];
    let (installation, transport) = if broker {
        let socket = match service_socket {
            Some(socket) => socket,
            None => {
                warn!("metric broker granted without a service socket");
                return None;
            }
        };
        let client = match ServiceMetricClient::new(socket.to_path_buf(), vm_id) {
            Ok(client) => client,
            Err(error) => {
                warn!(%error, "metric export not installed");
                return None;
            }
        };
        (
            capsem_telemetry::export::install_with_http_client(client, "capsem-process", attributes),
            "service-broker",
        )
    } else {
        let destination = Destination::corp(endpoint)?;
        (
            capsem_telemetry::export::install(&destination, "capsem-process", attributes),
            "direct-transition",
        )
    };
    match installation {
        Ok(exporter) => {
            info!(transport, "metric export installed");
            Some(exporter)
        }
        Err(error) => {
            warn!(%error, transport, "metric export not installed");
            None
        }
    }
}

#[cfg(test)]
mod tests;
