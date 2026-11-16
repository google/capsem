//! Coordinator-minted connections for the confined HTTP gateway.

use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::auth::{
    decode_gateway_auth_request, encode_gateway_auth_response, is_gateway_auth_frame, GatewayAuthDenial,
    GatewayAuthRequest, GatewayAuthResponse,
};
use capsem_proto::gateway_grant::{
    decode_gateway_grant_request, encode_gateway_grant_response, GatewayGrantDenial, GatewayGrantKind,
    GatewayGrantRequest, GatewayGrantResponse, GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS,
};
use std::os::fd::AsRawFd;

enum ControlRequest {
    Grant(GatewayGrantRequest),
    Auth(GatewayAuthRequest),
}
impl ControlRequest {
    fn request_id(&self) -> u64 {
        match self {
            Self::Grant(request) => request.request_id(),
            Self::Auth(request) => request.request_id(),
        }
    }
}

type WireSender = DescriptorSender<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;
const ADOPTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) async fn serve(socket: std::os::unix::net::UnixStream, state: Arc<ServiceState>) -> std::io::Result<()> {
    let requests = WireReceiver::new(socket.try_clone()?)?;
    let responses = WireSender::new(socket)?;
    let authority = crate::instance::WorkerAuthority::default();
    let service_router = crate::router_runtime::build_service_router(Arc::clone(&state));
    let mut operator_epoch = state.caller_authority.begin();
    let mut metrics_issued = false;
    let mut expected_id = 1_u64;
    loop {
        let frame = requests.recv().await?;
        if !frame.fds.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "gateway grant request carried descriptors",
            ));
        }
        let request = if is_gateway_auth_frame(&frame.bytes) {
            ControlRequest::Auth(decode_gateway_auth_request(&frame.bytes).map_err(std::io::Error::other)?)
        } else {
            ControlRequest::Grant(decode_gateway_grant_request(&frame.bytes).map_err(std::io::Error::other)?)
        };
        let request_id = request.request_id();
        if request_id != expected_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "gateway grant request identity mismatch",
            ));
        }
        expected_id = expected_id
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("gateway grant request identity exhausted"))?;
        let request = match request {
            ControlRequest::Auth(request) => {
                let response = authenticate(&mut operator_epoch, request);
                let response = encode_gateway_auth_response(response).map_err(std::io::Error::other)?;
                responses.send(&response, &[]).await?;
                continue;
            }
            ControlRequest::Grant(request) => request,
        };
        let mut activate = None;
        let opened = match request {
            GatewayGrantRequest::OpenMetrics { .. } if metrics_issued => Err(GatewayGrantDenial::Revoked),
            GatewayGrantRequest::OpenMetrics { .. } => {
                let opened =
                    crate::service_runtime::telemetry_export::gateway::prepare(&state, authority.grant()).await;
                metrics_issued = opened.is_ok();
                opened.map(|prepared| {
                    activate = Some(prepared.activate);
                    (GatewayGrantKind::Metrics, prepared.descriptor)
                })
            }
            request => open(&state, &service_router, authority.grant(), request).await,
        };
        match opened {
            Ok((kind, descriptor)) => {
                send_grant(&responses, &requests, request_id, kind, descriptor).await?;
                if let Some(activate) = activate {
                    let _ = activate.send(());
                }
            }
            Err(reason) => {
                let response = encode_gateway_grant_response(GatewayGrantResponse::Denied { request_id, reason });
                responses.send(&response, &[]).await?;
            }
        }
    }
}

/// Bounded in-memory authentication; no filesystem registration or I/O.
fn authenticate(
    operator_epoch: &mut crate::caller_auth::OperatorEpoch,
    request: GatewayAuthRequest,
) -> GatewayAuthResponse {
    let request_id = request.request_id();
    match request {
        GatewayAuthRequest::RegisterOperator { token, .. } => match operator_epoch.register(token) {
            Ok(()) => GatewayAuthResponse::RegisteredOperator { request_id },
            Err(reason) => GatewayAuthResponse::Denied { request_id, reason },
        },
        // No delegation is issued before its durable authority is
        // installed. Unknown access identities always fail closed.
        GatewayAuthRequest::AuthenticateAgent { .. } => GatewayAuthResponse::Denied {
            request_id,
            reason: GatewayAuthDenial::Forbidden,
        },
    }
}

async fn send_grant(
    responses: &WireSender,
    requests: &WireReceiver,
    request_id: u64,
    kind: GatewayGrantKind,
    descriptor: std::os::unix::net::UnixStream,
) -> std::io::Result<()> {
    tokio::time::timeout(ADOPTION_TIMEOUT, async {
        let response = encode_gateway_grant_response(GatewayGrantResponse::Granted { request_id, kind });
        responses.send(&response, &[descriptor.as_raw_fd()]).await?;
        let receipt = requests.recv().await?;
        if !receipt.fds.is_empty()
            || decode_gateway_grant_request(&receipt.bytes).map_err(std::io::Error::other)?
                != (GatewayGrantRequest::Adopted { request_id })
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "gateway adoption receipt identity or direction mismatch",
            ));
        }
        Ok(())
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "gateway adoption receipt timed out"))??;
    // A send only queues SCM_RIGHTS; Darwin needs an owner through adoption.
    drop(descriptor);
    Ok(())
}

async fn open(
    state: &Arc<ServiceState>,
    service_router: &axum::Router,
    control: crate::instance::WorkerGrant,
    request: GatewayGrantRequest,
) -> Result<(GatewayGrantKind, std::os::unix::net::UnixStream), GatewayGrantDenial> {
    match request {
        GatewayGrantRequest::Adopted { .. } | GatewayGrantRequest::OpenMetrics { .. } => {
            Err(GatewayGrantDenial::InvalidRequest)
        }
        GatewayGrantRequest::OpenService { .. } => crate::gateway_channel::prepare(service_router.clone(), control)
            .map(|socket| (GatewayGrantKind::Service, socket))
            .map_err(|_| GatewayGrantDenial::Unavailable),
        GatewayGrantRequest::OpenOwnerHandoff { vm_id, .. } => {
            let handoff = crate::owner_handoff::OwnerHandoff::acquire(state, &vm_id)
                .await
                .map_err(|error| match error.status {
                    StatusCode::NOT_FOUND => GatewayGrantDenial::NotFound,
                    StatusCode::CONFLICT => GatewayGrantDenial::Revoked,
                    _ => GatewayGrantDenial::Unavailable,
                })?;
            handoff
                .connect(state)
                .await
                .map(|socket| (GatewayGrantKind::OwnerHandoff, socket))
                .map_err(|_| GatewayGrantDenial::Revoked)
        }
    }
}

#[cfg(test)]
mod tests;
