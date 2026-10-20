use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};
use capsem_proto::mcp_aggregator::AggregatorClient;
use capsem_proto::proxy_mcp::ProxyMcpHello;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn start(
    socket: UnixStream,
) -> Result<(AggregatorClient, ProxyMcpHello, tokio::sync::watch::Receiver<bool>)> {
    capsem_foundation::unix::fd::set_nonblocking(socket.as_fd(), true)?;
    let reader = tokio::net::UnixStream::from_std(socket.try_clone()?)?;
    let writer = tokio::net::UnixStream::from_std(socket)?;
    let mut reader = reader;
    let hello = tokio::time::timeout(
        HELLO_TIMEOUT,
        capsem_proto::mcp_aggregator::read_frame::<_, ProxyMcpHello>(&mut reader),
    )
    .await
    .context("proxy MCP hello timed out")??
    .context("proxy MCP capability closed before hello")?;
    hello.validate()?;

    let (client, requests) = AggregatorClient::channel(64);
    let driver = capsem_core::mcp::aggregator_driver::spawn(requests, writer, reader);
    let stopped = driver.stop_receiver();
    Ok((client, hello, stopped))
}

#[cfg(test)]
mod tests;
