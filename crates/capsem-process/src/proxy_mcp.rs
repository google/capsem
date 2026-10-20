use std::collections::{BTreeSet, HashSet};
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use anyhow::{Context, Result};
use capsem_core::net::mitm_proxy::{McpTimeouts, ScopedMcpTools};
use capsem_proto::mcp_aggregator::{
    AggregatorClient, AggregatorMethod, AggregatorRequest, AggregatorResponse, AggregatorResult,
};
use capsem_proto::proxy_mcp::ProxyMcpHello;

const RESPONSE_CAPACITY: usize = 64;

pub(crate) async fn serve(
    socket: UnixStream,
    aggregator: AggregatorClient,
    scoped_tools: Arc<dyn ScopedMcpTools>,
    builtin_servers: BTreeSet<String>,
    inflight_cap: usize,
    timeouts: McpTimeouts,
) -> Result<()> {
    capsem_foundation::unix::fd::set_nonblocking(socket.as_fd(), true)?;
    let mut reader = tokio::net::UnixStream::from_std(socket.try_clone()?)?;
    let mut writer = tokio::net::UnixStream::from_std(socket)?;
    let hello = ProxyMcpHello::new(
        builtin_servers.into_iter().collect(),
        u16::try_from(inflight_cap).context("proxy MCP in-flight cap exceeds its wire field")?,
        timeout_ms(timeouts.default_timeout)?,
        timeout_ms(timeouts.tool_call_default)?,
        timeout_ms(timeouts.tool_call_ceiling)?,
    )?;
    capsem_proto::mcp_aggregator::write_frame(&mut writer, &hello).await?;

    let scoped_names: HashSet<String> = scoped_tools
        .definitions()
        .into_iter()
        .map(|definition| definition.namespaced_name)
        .collect();
    let (responses, mut response_rx) = tokio::sync::mpsc::channel::<AggregatorResponse>(RESPONSE_CAPACITY);
    let writer_task = tokio::spawn(async move {
        while let Some(response) = response_rx.recv().await {
            if capsem_proto::mcp_aggregator::write_frame(&mut writer, &response)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(inflight_cap));
    let mut requests = tokio::task::JoinSet::new();

    let result = loop {
        let request = match capsem_proto::mcp_aggregator::read_frame::<_, AggregatorRequest>(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error.context("read proxy MCP request")),
        };
        let permit = Arc::clone(&permits).acquire_owned().await?;
        let aggregator = aggregator.clone();
        let scoped_tools = Arc::clone(&scoped_tools);
        let scoped_names = scoped_names.clone();
        let responses = responses.clone();
        requests.spawn(async move {
            let _permit = permit;
            let body = dispatch(request.method, aggregator, scoped_tools, &scoped_names).await;
            let _ = responses.send(AggregatorResponse { id: request.id, body }).await;
        });
    };

    requests.abort_all();
    while requests.join_next().await.is_some() {}
    drop(responses);
    let _ = writer_task.await;
    result
}

async fn dispatch(
    method: AggregatorMethod,
    aggregator: AggregatorClient,
    scoped_tools: Arc<dyn ScopedMcpTools>,
    scoped_names: &HashSet<String>,
) -> AggregatorResult {
    match method {
        AggregatorMethod::ListTools => match aggregator.list_tools().await {
            Ok(mut tools) => {
                tools.extend(scoped_tools.definitions());
                AggregatorResult::Tools { tools }
            }
            Err(error) => AggregatorResult::Error {
                error: format!("{error:#}"),
            },
        },
        AggregatorMethod::CallTool {
            name,
            arguments,
            timeout_ms: _,
        } if scoped_names.contains(&name) => match scoped_tools.call_tool(&name, arguments).await {
            Ok(result) => AggregatorResult::CallResult { result },
            Err(error) => AggregatorResult::Error { error },
        },
        method => aggregator
            .request(method)
            .await
            .unwrap_or_else(|error| AggregatorResult::Error {
                error: format!("{error:#}"),
            }),
    }
}

fn timeout_ms(timeout: std::time::Duration) -> Result<u64> {
    u64::try_from(timeout.as_millis()).context("proxy MCP timeout exceeds its wire field")
}

#[cfg(test)]
mod tests;
