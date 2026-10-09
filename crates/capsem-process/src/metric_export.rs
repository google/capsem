//! Metric export for one VM process.
//!
//! The process exports its own measurements -- ledger writer, MITM, DNS,
//! security engine, virtio-blk -- tagged with the session it serves. Broker
//! mode carries encoded OTLP bytes over the service socket without learning
//! a collector address or credential. Without a broker grant, export is off.

use capsem_telemetry::export::{Exporter, KeyValue};
use std::sync::Mutex;
use tracing::{info, warn};

/// OTLP transport that can reach only this process's fixed service route.
struct ServiceMetricClient {
    service_socket: Mutex<Option<std::os::unix::net::UnixStream>>,
    service: Mutex<Option<capsem_core::service_uds::Client>>,
    path: String,
    runtime: Mutex<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for ServiceMetricClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceMetricClient")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ServiceMetricClient {
    fn new(service_socket: std::os::unix::net::UnixStream, vm_id: &str) -> Result<Self, String> {
        service_socket
            .set_nonblocking(true)
            .map_err(|error| format!("prepare metric broker socket: {error}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("build metric broker runtime: {error}"))?;
        Ok(Self {
            service_socket: Mutex::new(Some(service_socket)),
            service: Mutex::new(None),
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
        let runtime = self.runtime.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut service = self.service.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if service.is_none() {
            let socket = self
                .service_socket
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .ok_or_else(|| std::io::Error::other("metric broker socket unavailable"))?;
            let client = runtime
                .block_on(async move {
                    let stream = tokio::net::UnixStream::from_std(socket)?;
                    capsem_core::service_uds::Client::from_stream(stream).await
                })
                .map_err(|error| std::io::Error::other(format!("prepare metric broker HTTP channel: {error:#}")))?;
            *service = Some(client);
        }
        let response = runtime.block_on(service.as_ref().unwrap().post_bytes(
            &self.path,
            http::HeaderValue::from_static("application/x-protobuf"),
            request.into_body(),
        ));
        drop(service);
        drop(runtime);
        response.map_err(|error| Box::new(std::io::Error::other(format!("metric broker request: {error:#}"))) as _)
    }
}

/// Install export when the service granted its broker. Failure is logged and
/// leaves export off; metrics never prevent a VM from booting.
pub(crate) fn install(
    vm_id: &str,
    service_socket: Option<std::os::unix::net::UnixStream>,
    broker: bool,
) -> Option<Exporter> {
    if !broker {
        return None;
    }
    let attributes = vec![KeyValue::new("session.id", vm_id.to_string())];
    let socket = match service_socket {
        Some(socket) => socket,
        None => {
            warn!("metric broker granted without a service socket");
            return None;
        }
    };
    let client = match ServiceMetricClient::new(socket, vm_id) {
        Ok(client) => client,
        Err(error) => {
            warn!(%error, "metric export not installed");
            return None;
        }
    };
    let installation = capsem_telemetry::export::install_with_http_client(client, "capsem-process", attributes);
    match installation {
        Ok(exporter) => {
            info!(transport = "service-broker", "metric export installed");
            Some(exporter)
        }
        Err(error) => {
            warn!(%error, transport = "service-broker", "metric export not installed");
            None
        }
    }
}

#[cfg(test)]
mod tests;
