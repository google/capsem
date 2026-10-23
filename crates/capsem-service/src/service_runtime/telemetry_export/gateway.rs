//! One bounded metric capability per authenticated gateway control generation.
//!
//! The granting control channel owns revocation. A fresh epoch and ordered
//! requests prevent a retained/stale FD from submitting into its successor.
//! No response body/config/URL/error string is ever returned to the gateway.
use std::sync::Arc;

use capsem_foundation::ipc_channel::channel_from_std_with_limit;
use capsem_proto::gateway_grant::{
    metrics::{GatewayMetricMessage, GatewayMetricOutcome, GatewayMetricRequest, MAX_GATEWAY_METRIC_FRAME_BYTES},
    GatewayGrantDenial,
};
use capsem_telemetry::export::relay::{AdmittedBody, Collector};

use crate::{instance::WorkerGrant, ServiceState};

#[cfg(test)]
mod tests;

fn current_collector() -> Result<Option<Collector>, capsem_telemetry::export::relay::CollectorError> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    Collector::resolve(corp.corp_rule_files.open_telemetry.as_deref(), |name| {
        std::env::var(name).ok()
    })
}

pub(crate) struct PreparedGrant {
    pub(crate) descriptor: std::os::unix::net::UnixStream,
    pub(crate) activate: tokio::sync::oneshot::Sender<()>,
}

pub(crate) async fn prepare(
    state: &Arc<ServiceState>,
    authority: WorkerGrant,
) -> Result<PreparedGrant, GatewayGrantDenial> {
    let enabled = state
        .off_worker(|_| current_collector().map(|collector| collector.is_some()))
        .await
        .map_err(|_| GatewayGrantDenial::Unavailable)?
        .map_err(|_| GatewayGrantDenial::Unavailable)?;
    if authority.is_revoked() {
        return Err(GatewayGrantDenial::Revoked);
    }
    if !enabled {
        return Err(GatewayGrantDenial::Unavailable);
    }
    let (service, gateway) =
        capsem_foundation::unix::fd::connected_unix_pair().map_err(|_| GatewayGrantDenial::Unavailable)?;
    let generation = *uuid::Uuid::new_v4().as_bytes();
    let state = Arc::clone(state);
    let (activate, activated) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::select! {
            biased;
            () = authority.revoked() => return,
            activated = activated => if activated.is_err() { return },
        }
        if serve(state, service, authority, generation).await.is_err() {
            // Decoder errors can contain attacker-chosen field names. Never
            // log the underlying MessagePack/OTLP/HTTP error or body here.
            tracing::debug!("gateway metric capability closed after invalid transport or identity");
        }
    });
    Ok(PreparedGrant {
        descriptor: gateway,
        activate,
    })
}

enum Prepared {
    Rejected,
    Unavailable,
    Send(Box<Collector>, AdmittedBody),
}

async fn serve(
    state: Arc<ServiceState>,
    socket: std::os::unix::net::UnixStream,
    authority: WorkerGrant,
    generation: [u8; 16],
) -> std::io::Result<()> {
    let (responses, requests) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
        socket,
        MAX_GATEWAY_METRIC_FRAME_BYTES,
    )?;
    tokio::select! {
        biased;
        () = authority.revoked() => return Ok(()),
        sent = responses.send(GatewayMetricMessage::Hello { generation }) => sent?,
    }
    let mut expected = 1_u64;
    loop {
        let request = tokio::select! {
            biased;
            () = authority.revoked() => return Ok(()),
            request = requests.recv() => request.map_err(|_| invalid())?,
        };
        if !request.valid_for(generation, expected) {
            return Err(invalid());
        }
        expected = expected.checked_add(1).ok_or_else(invalid)?;
        // This bounded parse/config work never stalls a polled HTTP route.
        let prepared = tokio::select! {
            biased;
            () = authority.revoked() => return Ok(()),
            prepared = state.off_worker(move |_| {
                let Ok(body) = AdmittedBody::admit(request.body.into()) else { return Prepared::Rejected };
                match current_collector() {
                    Ok(Some(collector)) => Prepared::Send(Box::new(collector), body),
                    _ => Prepared::Unavailable,
                }
            }) => prepared.map_err(|_| invalid())?,
        };
        let outcome = match prepared {
            Prepared::Rejected => GatewayMetricOutcome::Rejected,
            Prepared::Unavailable => GatewayMetricOutcome::Unavailable,
            Prepared::Send(collector, body) => {
                if authority.is_revoked() {
                    return Ok(());
                }
                let delivered = tokio::select! {
                    biased;
                    () = authority.revoked() => return Ok(()),
                    delivered = collector.send(body) => delivered,
                };
                if delivered.is_ok() {
                    GatewayMetricOutcome::Accepted
                } else {
                    GatewayMetricOutcome::Unavailable
                }
            }
        };
        tokio::select! {
            biased;
            () = authority.revoked() => return Ok(()),
            sent = responses.send(GatewayMetricMessage::Receipt { request_id: expected - 1, outcome }) => sent?,
        }
    }
}

fn invalid() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "gateway metric capability rejected")
}

/// Descriptor/argument authority is explicit. Inherit only diagnostics and
/// coverage output, never ambient provider/collector credentials or settings.
pub(crate) fn strip_collector_environment(command: &mut tokio::process::Command) {
    command.env_clear();
    for key in ["RUST_LOG", "RUST_BACKTRACE", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
}
