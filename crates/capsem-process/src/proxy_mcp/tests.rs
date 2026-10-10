use std::collections::BTreeSet;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use capsem_core::net::mitm_proxy::ScopedMcpTools;
use capsem_proto::mcp_aggregator::{
    read_frame, write_frame, AggregatorClient, AggregatorMethod, AggregatorRequest, AggregatorResponse,
    AggregatorResult,
};
use capsem_proto::mcp_contracts::McpToolDef;
use capsem_proto::proxy_mcp::ProxyMcpHello;

struct Scoped;

impl ScopedMcpTools for Scoped {
    fn definitions(&self) -> Vec<McpToolDef> {
        vec![McpToolDef {
            namespaced_name: "capsem__fixture".into(),
            original_name: "fixture".into(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
            server_name: "capsem".into(),
            annotations: None,
            timeout_secs: None,
        }]
    }

    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        arguments: serde_json::Value,
    ) -> futures::future::BoxFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async move { Ok(serde_json::json!({"arguments": arguments})) })
    }
}

#[tokio::test]
async fn bridge_bootstraps_limits_and_dispatches_scoped_tools_without_aggregator_handoff() {
    let (aggregator, mut aggregator_rx) = AggregatorClient::channel(4);
    let aggregator_task = tokio::spawn(async move {
        let (request, reply) = aggregator_rx.recv().await.unwrap();
        assert!(matches!(request.method, AggregatorMethod::ListTools));
        reply
            .send(AggregatorResponse {
                id: request.id,
                body: AggregatorResult::Tools { tools: Vec::new() },
            })
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(50), aggregator_rx.recv()).await,
            Err(_) | Ok(None)
        ));
    });
    let (proxy, owner) = UnixStream::pair().unwrap();
    let server = tokio::spawn(super::serve(
        owner,
        aggregator,
        Arc::new(Scoped),
        BTreeSet::from(["local".into()]),
        7,
        capsem_core::net::mitm_proxy::McpTimeouts {
            default_timeout: std::time::Duration::from_secs(2),
            tool_call_default: std::time::Duration::from_secs(3),
            tool_call_ceiling: std::time::Duration::from_secs(4),
        },
    ));
    capsem_foundation::unix::fd::set_nonblocking(proxy.as_fd(), true).unwrap();
    let mut read = tokio::net::UnixStream::from_std(proxy.try_clone().unwrap()).unwrap();
    let mut write = tokio::net::UnixStream::from_std(proxy).unwrap();
    let hello = read_frame::<_, ProxyMcpHello>(&mut read).await.unwrap().unwrap();
    assert_eq!(hello.inflight_cap, 7);
    assert_eq!(hello.builtin_servers, ["local"]);

    write_frame(
        &mut write,
        &AggregatorRequest {
            id: 1,
            method: AggregatorMethod::ListTools,
        },
    )
    .await
    .unwrap();
    let listed = read_frame::<_, AggregatorResponse>(&mut read).await.unwrap().unwrap();
    assert!(matches!(listed.body, AggregatorResult::Tools { tools } if tools.len() == 1));
    write_frame(
        &mut write,
        &AggregatorRequest {
            id: 2,
            method: AggregatorMethod::CallTool {
                name: "capsem__fixture".into(),
                arguments: serde_json::json!({"value": 7}),
                timeout_ms: Some(3_000),
            },
        },
    )
    .await
    .unwrap();
    let called = read_frame::<_, AggregatorResponse>(&mut read).await.unwrap().unwrap();
    assert!(matches!(called.body, AggregatorResult::CallResult { .. }));
    drop(read);
    drop(write);
    server.await.unwrap().unwrap();
    aggregator_task.await.unwrap();
}
