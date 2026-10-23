//! Bounded metric-only transport over an adopted service capability.
//! Destination, headers and collector responses never cross this descriptor.
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use capsem_foundation::ipc_channel::{channel_from_std_with_limit, Receiver, Sender};
use capsem_proto::gateway_grant::metrics::{
    GatewayMetricMessage, GatewayMetricOutcome, GatewayMetricRequest, MAX_GATEWAY_METRIC_BODY_BYTES,
    MAX_GATEWAY_METRIC_FRAME_BYTES,
};
use tokio::sync::Mutex;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);

type ExportTask = tokio::task::JoinHandle<Option<capsem_telemetry::export::Exporter>>;

pub(crate) fn start(grants: crate::service_grant::GatewayGrantClient) -> ExportTask {
    tokio::spawn(async move {
        let socket = grants.open_metrics().await.ok()?;
        let client = MetricClient::connect(socket).await.ok()?;
        tokio::task::spawn_blocking(move || capsem_telemetry::export::install_gateway_http_client(client).ok())
            .await
            .ok()
            .flatten()
    })
}

pub(crate) async fn stop(task: ExportTask) {
    if !task.is_finished() {
        task.abort();
    }
    if let Ok(Some(exporter)) = task.await {
        // SDK shutdown synchronously waits for its export thread. Keep this
        // reactor alive so that thread's granted FD exchange can complete.
        let _ = tokio::task::spawn_blocking(move || drop(exporter)).await;
    }
}

struct Seat {
    outgoing: Sender<GatewayMetricRequest>,
    incoming: Receiver<GatewayMetricMessage>,
    generation: [u8; 16],
    next: u64,
}

#[derive(Clone)]
struct MetricClient {
    seat: Arc<Mutex<Option<Seat>>>,
    runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for MetricClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GrantedGatewayMetricClient")
    }
}

impl MetricClient {
    async fn connect(socket: std::os::unix::net::UnixStream) -> std::io::Result<Self> {
        let (outgoing, incoming) = channel_from_std_with_limit(socket, MAX_GATEWAY_METRIC_FRAME_BYTES)?;
        let hello = tokio::time::timeout(HELLO_TIMEOUT, incoming.recv())
            .await
            .map_err(|_| invalid())?
            .map_err(|_| invalid())?;
        let GatewayMetricMessage::Hello { generation } = hello else {
            return Err(invalid());
        };
        if generation == [0; 16] {
            return Err(invalid());
        }
        Ok(Self {
            seat: Arc::new(Mutex::new(Some(Seat {
                outgoing,
                incoming,
                generation,
                next: 1,
            }))),
            runtime: tokio::runtime::Handle::current(),
        })
    }
}

fn invalid() -> std::io::Error {
    std::io::Error::other("gateway metric capability unavailable")
}

// The SDK owns this Bytes buffer. Vec::from moves its unique allocation into
// the typed envelope; unlike to_vec it does not unconditionally copy it.
fn take_body(body: Bytes) -> std::io::Result<Vec<u8>> {
    if body.is_empty() || body.len() > MAX_GATEWAY_METRIC_BODY_BYTES {
        return Err(invalid());
    }
    Ok(body.into())
}

struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for MetricClient {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, opentelemetry_http::HttpError> {
        // Ignore the SDK's synthetic URI and all headers. Only its encoded
        // metric body can enter the service's strict metric admission owner.
        let body = take_body(request.into_body())?;
        let shared = Arc::clone(&self.seat);
        let task = self.runtime.spawn(async move {
            let mut guard = shared.lock().await;
            let mut seat = guard.take().ok_or_else(invalid)?;
            let exchange = async {
                seat.outgoing
                    .send(GatewayMetricRequest {
                        generation: seat.generation,
                        request_id: seat.next,
                        body,
                    })
                    .await
                    .map_err(|_| invalid())?;
                match seat.incoming.recv().await.map_err(|_| invalid())? {
                    GatewayMetricMessage::Receipt { request_id, outcome } if request_id == seat.next => Ok(outcome),
                    _ => Err(invalid()),
                }
            };
            let outcome = tokio::time::timeout(EXCHANGE_TIMEOUT, exchange)
                .await
                .map_err(|_| invalid())??;
            seat.next = seat.next.checked_add(1).ok_or_else(invalid)?;
            *guard = Some(seat);
            drop(guard);
            Ok::<_, std::io::Error>(outcome)
        });
        let _cancel = AbortOnDrop(task.abort_handle());
        let outcome = task.await.map_err(|_| invalid())??;
        match outcome {
            GatewayMetricOutcome::Accepted => Ok(http::Response::builder()
                .status(http::StatusCode::NO_CONTENT)
                .body(Bytes::new())?),
            GatewayMetricOutcome::Rejected | GatewayMetricOutcome::Unavailable => Err(invalid().into()),
        }
    }
}

#[cfg(test)]
mod tests;
