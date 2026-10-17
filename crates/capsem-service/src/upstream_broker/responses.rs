//! Bounded wire responses; the caller retains all descriptor owners.
use super::*;

pub(super) async fn send_denied(
    sender: &WireSender,
    request_id: u64,
    reason: UpstreamGrantDenial,
) -> Result<(), String> {
    send_response(sender, &UpstreamGrantResponse::Denied { request_id, reason }, None).await
}

pub(super) async fn send_response(
    sender: &WireSender,
    response: &UpstreamGrantResponse,
    descriptor: Option<&OwnedFd>,
) -> Result<(), String> {
    let descriptors = descriptor.into_iter().collect::<Vec<_>>();
    send_response_descriptors(sender, response, &descriptors).await
}

pub(super) async fn send_response_descriptors(
    sender: &WireSender,
    response: &UpstreamGrantResponse,
    descriptors: &[&OwnedFd],
) -> Result<(), String> {
    let bytes =
        encode_upstream_grant_response(response).map_err(|error| format!("encode grant response: {error:#}"))?;
    let descriptors = descriptors.iter().map(|fd| fd.as_raw_fd()).collect::<Vec<_>>();
    tokio::time::timeout(RESPONSE_TIMEOUT, sender.send(&bytes, &descriptors))
        .await
        .map_err(|_| "send upstream grant response timed out".to_string())?
        .map_err(|error| format!("send upstream grant response: {error}"))?;
    Ok(())
}
