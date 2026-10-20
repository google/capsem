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
const ADOPTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
                send_grant(&responses, &requests, request_id, kind, descriptor).await?;
            }
            Err(reason) => {
                let response = encode_gateway_grant_response(GatewayGrantResponse::Denied { request_id, reason });
                responses.send(&response, &[]).await?;
            }
        }
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
    request: GatewayGrantRequest,
) -> Result<(GatewayGrantKind, std::os::unix::net::UnixStream), GatewayGrantDenial> {
    match request {
        GatewayGrantRequest::Adopted { .. } => Err(GatewayGrantDenial::InvalidRequest),
        GatewayGrantRequest::OpenService { .. } => tokio::net::UnixStream::connect(&state.service_socket)
            .await
            .and_then(|socket| socket.into_std())
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
