//! No caller-selected identity: the inherited broker binds the descriptor.
use super::*;

impl UpstreamGrantClient {
    pub async fn open_approval(&self) -> Result<UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::OpenApproval { reply })
            .await
            .map_err(|_| anyhow!("upstream grant channel is closed"))?;
        result
            .await
            .map_err(|_| anyhow!("upstream grant channel stopped"))?
            .map_err(anyhow::Error::msg)
    }
}

pub(super) async fn open(
    sender: &WireSender,
    receiver: &WireReceiver,
    request_id: u64,
) -> Result<Result<UnixStream, String>, String> {
    send_request(sender, &UpstreamGrantRequest::OpenApproval { request_id }).await?;
    let (response, mut descriptors) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::ApprovalGranted { request_id: actual } if actual == request_id => {
            let descriptor = descriptors.pop().ok_or("approval grant omitted its descriptor")?;
            fd::validate_connected_stream(descriptor.as_fd())
                .map_err(|error| format!("approval capability: {error}"))?;
            Ok(Ok(UnixStream::from(descriptor)))
        }
        UpstreamGrantResponse::Denied {
            request_id: actual,
            reason,
        } if actual == request_id => Ok(Err(format!("approval capability denied: {reason:?}"))),
        _ => Err("unexpected approval capability response identity".into()),
    }
}
