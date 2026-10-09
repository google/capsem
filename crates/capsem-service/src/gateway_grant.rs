//! Coordinator-minted connections for the confined HTTP gateway.

use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::{
    decode_gateway_grant_request, encode_gateway_grant_response, GatewayGrantDenial, GatewayGrantKind,
    GatewayGrantRequest, GatewayGrantResponse, GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS,
};
use std::os::fd::AsRawFd;

type WireSender = DescriptorSender<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;
type WireReceiver = DescriptorReceiver<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>;

pub(super) async fn serve(socket: std::os::unix::net::UnixStream, state: Arc<ServiceState>) -> std::io::Result<()> {
    let requests = WireReceiver::new(socket.try_clone()?)?;
    let responses = WireSender::new(socket)?;
    loop {
        let frame = requests.recv().await?;
        if !frame.fds.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "gateway grant request carried descriptors",
            ));
        }
        let request = decode_gateway_grant_request(&frame.bytes).map_err(std::io::Error::other)?;
        let request_id = request.request_id();
        match open(&state, request).await {
            Ok((kind, descriptor)) => {
                let response = encode_gateway_grant_response(GatewayGrantResponse::Granted { request_id, kind });
                responses.send(&response, &[descriptor.as_raw_fd()]).await?;
            }
            Err(reason) => {
                let response = encode_gateway_grant_response(GatewayGrantResponse::Denied { request_id, reason });
                responses.send(&response, &[]).await?;
            }
        }
    }
}

async fn open(
    state: &Arc<ServiceState>,
    request: GatewayGrantRequest,
) -> Result<(GatewayGrantKind, std::os::unix::net::UnixStream), GatewayGrantDenial> {
    match request {
        GatewayGrantRequest::OpenService { .. } => tokio::net::UnixStream::connect(&state.service_socket)
            .await
            .and_then(|socket| socket.into_std())
            .map(|socket| (GatewayGrantKind::Service, socket))
            .map_err(|_| GatewayGrantDenial::Unavailable),
        GatewayGrantRequest::OpenOwnerHandoff { vm_id, .. } => {
            let handoff = crate::owner_handoff::OwnerHandoff::acquire(state, &vm_id)
                .await
                .map_err(|error| match error.0 {
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
