//! Bounded original MCP calls owned by their IPC connection.
use super::{McpRuntime, ProcessToService, ServiceToProcess};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub(super) async fn spawn(
    mcp_capacity: &Arc<Semaphore>,
    mcp_calls: &mut JoinSet<()>,
    mcp_runtime: &Arc<McpRuntime>,
    ipc_tx_out: &mpsc::Sender<ProcessToService>,
    cancellation: &CancellationToken,
    request: ServiceToProcess,
) {
    let ServiceToProcess::McpCallTool {
        id,
        namespaced_name,
        arguments_json,
    } = request
    else {
        unreachable!("MCP call handler received another request")
    };
    let permit = match (arguments_json.len() <= capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST)
        .then(|| Arc::clone(mcp_capacity).try_acquire_owned().ok())
        .flatten()
    {
        Some(permit) => permit,
        None => {
            capsem_core::try_send!(
                "ipc_mcp_capacity",
                ipc_tx_out
                    .send(ProcessToService::McpCallToolResult {
                        id,
                        result_json: None,
                        event_id: None,
                        error: Some("MCP request capacity exceeded".into()),
                    })
                    .await
            );
            return;
        }
    };
    let mcp = Arc::clone(mcp_runtime);
    let ipc_tx_out = ipc_tx_out.clone();
    let cancellation = cancellation.clone();
    mcp_calls.spawn(async move {
        let _permit = permit;
        // The MCP boundary remains JSON even though internal IPC
        // is MessagePack, so its dynamic result shape stays owned
        // by the MCP contract rather than the transport.
        let arguments: serde_json::Value = serde_json::from_str(&arguments_json).unwrap_or(serde_json::Value::Null);
        let request = capsem_proto::mcp_contracts::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(id)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({"name": namespaced_name, "arguments": arguments})),
            meta: None,
        };
        let response = capsem_core::net::mitm_proxy::dispatch_cancellable_mcp_request(
            Arc::clone(&mcp.endpoint),
            Arc::clone(&mcp.db),
            request,
            "capsem-service".to_string(),
            capsem_core::net::mitm_proxy::McpTransport::Direct,
            cancellation,
        )
        .await;
        mcp.db.flush().await;
        let result_json = response
            .as_ref()
            .and_then(|result| serde_json::to_string(&result.response).ok());
        let event_id = response.as_ref().and_then(|result| result.event_id.clone());
        let error = response
            .as_ref()
            .and_then(|result| result.response.error.as_ref())
            .map(|error| error.message.clone())
            .or_else(|| {
                response
                    .is_none()
                    .then(|| "MCP request produced no response".to_string())
            });
        capsem_core::try_send!(
            "ipc_mcp_call_tool",
            ipc_tx_out
                .send(ProcessToService::McpCallToolResult {
                    id,
                    result_json,
                    event_id,
                    error
                })
                .await
        );
    });
}
