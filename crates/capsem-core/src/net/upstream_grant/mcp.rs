//! Coordinator-owned native MCP capabilities; never executable/argv grants.
use super::*;

impl UpstreamGrantClient {
    pub async fn open_mcp_aggregator(&self) -> Result<UnixStream> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::OpenMcpAggregator { reply })
            .await
            .map_err(|_| anyhow!("upstream grant channel is closed"))?;
        result
            .await
            .map_err(|_| anyhow!("upstream grant channel stopped"))?
            .map_err(anyhow::Error::msg)
    }

    pub async fn attach_mcp_http(&self, role: McpHttpRole, descriptor: OwnedFd) -> Result<()> {
        fd::validate_connected_stream(descriptor.as_fd())?;
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::AttachMcpHttp {
                role,
                descriptor,
                reply,
            })
            .await
            .map_err(|_| anyhow!("upstream grant channel is closed"))?;
        result
            .await
            .map_err(|_| anyhow!("upstream grant channel stopped"))?
            .map_err(anyhow::Error::msg)
    }
}

pub(super) async fn open_aggregator(
    sender: &WireSender,
    receiver: &WireReceiver,
    request_id: u64,
) -> Result<Result<UnixStream, String>, String> {
    let bytes = encode_upstream_grant_request(&UpstreamGrantRequest::OpenMcpAggregator { request_id })
        .map_err(|error| format!("encode MCP aggregator grant: {error:#}"))?;
    tokio::time::timeout(WIRE_TIMEOUT, sender.send(&bytes, &[]))
        .await
        .map_err(|_| "send MCP aggregator grant timed out".to_string())?
        .map_err(|error| format!("send MCP aggregator grant: {error}"))?;
    let (response, mut descriptors) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::McpAggregatorGranted {
            request_id: response_id,
        } if response_id == request_id => {
            let descriptor = descriptors.pop().ok_or("MCP aggregator grant omitted its descriptor")?;
            fd::validate_connected_stream(descriptor.as_fd()).map_err(|error| error.to_string())?;
            Ok(Ok(UnixStream::from(descriptor)))
        }
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("MCP aggregator grant denied: {reason:?}"))),
        response => Err(format!(
            "unexpected MCP aggregator response for request {request_id}: {response:?}"
        )),
    }
}

pub(super) async fn attach_http(
    sender: &WireSender,
    receiver: &WireReceiver,
    request_id: u64,
    role: McpHttpRole,
    descriptor: OwnedFd,
) -> Result<Result<(), String>, String> {
    let bytes = encode_upstream_grant_request(&UpstreamGrantRequest::AttachMcpHttp { request_id, role })
        .map_err(|error| format!("encode MCP HTTP handoff: {error:#}"))?;
    tokio::time::timeout(WIRE_TIMEOUT, sender.send(&bytes, &[descriptor.as_raw_fd()]))
        .await
        .map_err(|_| "send MCP HTTP handoff timed out".to_string())?
        .map_err(|error| format!("send MCP HTTP handoff: {error}"))?;
    drop(descriptor);
    let (response, _) = receive_response(receiver).await?;
    match response {
        UpstreamGrantResponse::McpHttpAdopted {
            request_id: response_id,
        } if response_id == request_id => Ok(Ok(())),
        UpstreamGrantResponse::Denied {
            request_id: response_id,
            reason,
        } if response_id == request_id => Ok(Err(format!("MCP HTTP handoff denied: {reason:?}"))),
        response => Err(format!(
            "unexpected MCP HTTP response for request {request_id}: {response:?}"
        )),
    }
}
