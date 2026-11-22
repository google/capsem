//! VM-owner RPC adapter; the service supervises native MCP processes.
use anyhow::Result;
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_foundation::unix::fd;
use capsem_proto::mcp_aggregator::{write_frame, AggregatorClient};
use std::os::fd::AsFd;

pub(crate) async fn connect_mcp_aggregator(
    servers: &[capsem_proto::mcp_contracts::McpServerDef],
    upstream: &UpstreamGrantClient,
    sandbox: Option<std::os::unix::net::UnixStream>,
) -> Result<AggregatorClient> {
    let stream = match sandbox {
        Some(sandbox) => upstream.open_mcp_aggregator_with_sandbox(sandbox.into()).await?,
        None => upstream.open_mcp_aggregator().await?,
    };
    fd::set_nonblocking(stream.as_fd(), true)?;
    let stream = tokio::net::UnixStream::from_std(stream)?;
    let (reader, mut writer) = stream.into_split();
    write_frame(&mut writer, &servers.to_vec()).await?;
    let (client, requests) = AggregatorClient::channel(capsem_proto::MCP_WORKER_QUEUE_CAPACITY);
    capsem_core::mcp::aggregator_driver::spawn(requests, writer, reader);
    Ok(client)
}
